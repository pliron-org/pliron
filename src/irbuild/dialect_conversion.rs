// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Utilities for dialect conversion style rewrites.
//! Similar in spirit to MLIR dialect conversion, but intentionally simpler:
//! - no unrealized conversion casts,
//! - definitions are converted before their uses, except for cycle backedges
//!   in graph regions.

use core::cell::Ref;

use alloc::{collections::vec_deque::VecDeque, vec, vec::Vec};

use crate::{
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    graph::walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    irbuild::{
        IRStatus,
        inserter::{Inserter, OpInsertionPoint},
        listener::{Recorder, RecorderEvent},
        rewriter::{IRRewriter, Rewriter},
    },
    irfmt::printers::list_with_sep,
    operation::{OpDbg, Operation},
    pass::{AnalysisManager, Pass, PassResult},
    printable::{ListSeparator, Printable},
    result::Result,
    r#type::{Type, TypeHandle, Typed},
    utils::table::{HMap, HSet},
    value::{DefiningEntity, Value},
};

/// A rewriter that uses the [Recorder] listener.
pub type DialectConversionRewriter = IRRewriter<Recorder>;

/// Additional type information for operation operands during conversion.
///
/// For each operand, we track a history of previously observed types during conversion.
/// This allows conversion patterns access to evolution of operand types,
/// rather than just the current type. The most recent type before conversion,
/// for each operand, is the last entry.
#[derive(Clone, Default)]
pub struct OperandsInfo(Vec<(Value, Vec<TypeHandle>)>);

impl Printable for OperandsInfo {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &crate::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "[")?;
        for (opd_idx, (opd, previous_types)) in self.0.iter().enumerate() {
            write!(
                f,
                "{{Operand: {}, current type: {}, previous types: [{}]}}",
                opd.disp(ctx),
                opd.get_type(ctx).disp(ctx),
                list_with_sep(previous_types, ListSeparator::CharSpace(',')).disp(ctx),
            )?;
            if opd_idx != self.0.len() - 1 {
                write!(f, ", ")?;
            }
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl OperandsInfo {
    pub fn new(operands: Vec<(Value, Vec<TypeHandle>)>) -> Self {
        Self(operands)
    }

    /// Lookup the most recent (excluding current) `T: Type` recorded for an operand, if any.
    pub fn lookup_most_recent_of_type<'a, T: Type>(
        &self,
        ctx: &'a Context,
        opd: Value,
    ) -> Option<Ref<'a, T>> {
        self.0
            .iter()
            .find(|(operand, _)| *operand == opd)
            .and_then(|(_, previous_types)| {
                previous_types.iter().rev().find_map(|ty| {
                    let ty_ref = ty.deref(ctx);
                    Ref::filter_map(ty_ref, |ty_ref| ty_ref.downcast_ref::<T>()).ok()
                })
            })
    }

    /// Lookup the most recent type (excluding current) recorded for an operand, if any.
    pub fn lookup_most_recent_type(&self, opd: Value) -> Option<TypeHandle> {
        self.0
            .iter()
            .find(|(operand, _)| *operand == opd)
            .and_then(|(_, previous_types)| previous_types.last().cloned())
    }

    /// Lookup the full history of types (excluding current) recorded for an operand,
    /// ordered from oldest to newest.
    pub fn lookup_operand_history(&self, opd: Value) -> Vec<TypeHandle> {
        self.0
            .iter()
            .find(|(operand, _)| *operand == opd)
            .map(|(_, previous_types)| previous_types.clone())
            .unwrap_or_default()
    }
}

/// Interface for dialect conversion matching and rewriting.
pub trait DialectConversion {
    /// Should this operation be converted?
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool;

    /// Should this type be converted?
    fn can_convert_type(&self, _ctx: &Context, _ty: TypeHandle) -> bool {
        false
    }

    /// Convert the type and return the converted type.
    fn convert_type(&mut self, _ctx: &mut Context, ty: TypeHandle) -> Result<TypeHandle> {
        Ok(ty)
    }

    /// Rewrite the operation.
    ///
    /// Insertion point is set to be before the operation being rewritten.
    /// Operand definitions are converted before this callback is invoked,
    /// except for definitions on cycle backedges in graph regions.
    /// Conversion order within such a cycle is unspecified.
    /// `operands_info` provides the current operand values along with their
    /// historical types observed during conversion. The last type in the history
    /// is the most recent type before conversion.
    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()>;
}

/// Applies dialect conversion rewrites rooted at `op`.
///
/// Conversion is trait-driven and ensures that any convertible
/// operand definitions are rewritten before rewriting the current operation,
/// except for cycle backedges in graph regions.
///
/// All block arguments reachable from `op` are converted up front.
/// Block arguments of blocks inserted during conversion are
/// converted as soon as they're observed via the listener.
//
// ## Algorithm
//
// 1. Collect:
//    - All initially convertible operations
//    - All basic blocks structurally nested under `op`
// 2. Convert the arguments of every collected block.
// 3. Repeatedly pop work items from the front. `Enter` items begin processing
//    an operation and `Resume` items continue it after its operand definitions.
// 4. Mark each entered op as `Processing`. If it has pending operand
//    definitions, schedule them before a `Resume` item for the op; otherwise,
//    rewrite the op immediately.
// 5. A definition already marked `Processing` is a cycle backedge. Do not wait
//    on it; all other definitions are handled first.
// 6. On `Resume`, re-read operands because earlier rewrites may have replaced
//    their definitions, then call the conversion pattern's `rewrite` callback.
// 7. Post rewrite, process recorder events:
//    - mark erased (during this batch) ops and blocks,
//    - update value type-history,
//    - enqueue newly inserted convertible ops and basic blocks.
// 8. Mark rewritten/non-convertible ops as `Processed`.
// 9. Before processing the next work item, convert arguments of newly inserted blocks.
pub fn apply_dialect_conversion<C: DialectConversion>(
    ctx: &mut Context,
    conversion: &mut C,
    op: Ptr<Operation>,
) -> Result<IRStatus> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum OpState {
        Queued,
        Processing,
        Processed,
        Erased,
    }

    #[derive(Clone, Copy)]
    enum WorkItem {
        Enter(Ptr<Operation>),
        Resume(Ptr<Operation>),
    }

    struct Driver<'a, C: DialectConversion> {
        conversion: &'a mut C,
        rewriter: DialectConversionRewriter,
        worklist: VecDeque<WorkItem>,
        pending_block_arg_conversions: Vec<Ptr<BasicBlock>>,
        op_states: HMap<Ptr<Operation>, OpState>,
        previous_types: HMap<Value, Vec<TypeHandle>>,
    }

    impl<'a, C: DialectConversion> Driver<'a, C> {
        fn new(conversion: &'a mut C) -> Self {
            let mut rewriter = DialectConversionRewriter::default();
            rewriter.set_listener(Recorder::default());
            Self {
                conversion,
                rewriter,
                worklist: VecDeque::new(),
                pending_block_arg_conversions: Vec::new(),
                op_states: HMap::default(),
                previous_types: HMap::default(),
            }
        }

        fn is_erased(&self, op: Ptr<Operation>) -> bool {
            matches!(self.op_states.get(&op), Some(OpState::Erased))
        }

        fn is_processed(&self, op: Ptr<Operation>) -> bool {
            matches!(self.op_states.get(&op), Some(OpState::Processed))
        }

        fn is_queued(&self, op: Ptr<Operation>) -> bool {
            matches!(self.op_states.get(&op), Some(OpState::Queued))
        }

        fn is_processing(&self, op: Ptr<Operation>) -> bool {
            matches!(self.op_states.get(&op), Some(OpState::Processing))
        }

        fn mark_erased(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Erased);
        }

        fn mark_processing(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Processing);
        }

        fn mark_processed(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Processed);
        }

        fn mark_enqueued(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Queued);
        }

        fn schedule_enter_front(&mut self, op: Ptr<Operation>) {
            assert!(
                self.is_queued(op),
                "Only queued operations can be scheduled for processing"
            );
            self.worklist.push_front(WorkItem::Enter(op));
        }

        fn enqueue_back(&mut self, op: Ptr<Operation>) {
            assert!(
                !self.is_processing(op) && !self.is_processed(op) && !self.is_erased(op),
                "Attempted to enqueue an operation that is already active or terminal-state"
            );
            self.mark_enqueued(op);
            self.worklist.push_back(WorkItem::Enter(op));
        }

        fn op_eligible_for_processing(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if self.is_erased(op) || self.is_processed(op) {
                return false;
            }
            self.conversion.can_convert_op(ctx, op)
        }

        /// Collects the initial worklist of operations (into `self.worklist`)
        /// and every block structurally nested under `root` (into
        /// `self.pending_block_arg_conversions`).
        fn collect_operations_blocks(&mut self, ctx: &mut Context, root: Ptr<Operation>) {
            self.worklist.clear();
            self.pending_block_arg_conversions.clear();
            self.op_states.clear();
            fn walker_callback<C: DialectConversion>(
                ctx: &Context,
                driver: &mut Driver<C>,
                node: IRNode,
            ) {
                match node {
                    IRNode::Operation(op) if driver.op_eligible_for_processing(ctx, op) => {
                        driver.enqueue_back(op);
                    }
                    IRNode::BasicBlock(block) => driver.pending_block_arg_conversions.push(block),
                    _ => {}
                }
            }
            walk_op(
                ctx,
                self,
                &WALKCONFIG_PREORDER_FORWARD,
                root,
                walker_callback::<C>,
            );
        }

        fn append_type_history(existing: &mut Vec<TypeHandle>, mut additional: Vec<TypeHandle>) {
            for ty in additional.drain(..) {
                if !existing.contains(&ty) {
                    existing.push(ty);
                }
            }
        }

        fn record_value_replacement(
            &mut self,
            old_value: Value,
            old_type: TypeHandle,
            new_value: Value,
        ) {
            let mut history = self.previous_types.remove(&old_value).unwrap_or_default();
            history.push(old_type);
            let existing = self.previous_types.entry(new_value).or_default();
            Self::append_type_history(existing, history);
        }

        fn record_type_change(&mut self, value: Value, old_type: TypeHandle) {
            let existing = self.previous_types.entry(value).or_default();
            Self::append_type_history(existing, vec![old_type]);
        }

        fn convert_block_argument_type(&mut self, ctx: &mut Context, value: Value) -> Result<()> {
            assert!(matches!(value.defining_entity(), DefiningEntity::Block(_)));

            loop {
                let current_type = value.get_type(ctx);
                if !self.conversion.can_convert_type(ctx, current_type) {
                    break;
                }

                let converted_type = self.conversion.convert_type(ctx, current_type)?;
                if converted_type == current_type {
                    break;
                }

                self.rewriter.set_value_type(ctx, value, converted_type);
                self.process_recorder_events(ctx)?;
            }

            Ok(())
        }

        fn convert_block_arguments(
            &mut self,
            ctx: &mut Context,
            block: Ptr<BasicBlock>,
        ) -> Result<()> {
            log::trace!(
                "Converting block arguments for block: {}",
                block.deref(ctx).unique_name(ctx).disp(ctx)
            );
            let args: Vec<_> = block.deref(ctx).arguments().collect();
            for arg in args {
                self.convert_block_argument_type(ctx, arg)?;
            }
            Ok(())
        }

        fn process_recorder_events(&mut self, ctx: &mut Context) -> Result<()> {
            let events = {
                let listener = self.rewriter.get_listener_mut();
                core::mem::take(&mut listener.events)
            };

            let mut erased_blocks = HSet::default();
            for event in &events {
                match event {
                    RecorderEvent::ErasedOperation(op) => self.mark_erased(*op),
                    RecorderEvent::ErasedBlock(block) => {
                        erased_blocks.insert(*block);
                    }
                    _ => {}
                }
            }

            for event in &events {
                match event {
                    RecorderEvent::ReplacedValueUses {
                        old_value,
                        old_type,
                        new_value,
                    } => {
                        self.record_value_replacement(*old_value, *old_type, *new_value);
                    }
                    RecorderEvent::ValueTypeChanged {
                        value,
                        old_type,
                        new_type: _,
                    } => {
                        self.record_type_change(*value, *old_type);
                    }
                    RecorderEvent::InsertedOperation(_) => {}
                    RecorderEvent::ErasedOperation(_)
                    | RecorderEvent::InsertedBlock(_)
                    | RecorderEvent::ErasedBlock(_)
                    | RecorderEvent::ErasedRegion(_)
                    | RecorderEvent::UnlinkedOperation(_, _)
                    | RecorderEvent::UnlinkedBlock(_, _) => {}
                }
            }

            for event in events {
                match event {
                    RecorderEvent::InsertedOperation(new_op)
                        if self.op_eligible_for_processing(ctx, new_op)
                            && !self.is_queued(new_op)
                            && !self.is_processing(new_op) =>
                    {
                        log::trace!(
                            "Inserted operation added to worklist: {}",
                            OpDbg { op: new_op, ctx }
                        );
                        self.enqueue_back(new_op);
                    }
                    RecorderEvent::InsertedBlock(new_block)
                        if !erased_blocks.contains(&new_block) =>
                    {
                        self.pending_block_arg_conversions.push(new_block);
                    }
                    _ => {}
                }
            }

            Ok(())
        }

        fn is_graph_region_backedge(
            ctx: &Context,
            op: Ptr<Operation>,
            def_op: Ptr<Operation>,
        ) -> bool {
            // A legal operand-dependency cycle cannot cross region boundaries:
            // values defined in a nested region cannot escape to an ancestor operation.
            let Some(region) = op.deref(ctx).get_parent_region(ctx) else {
                return false;
            };
            def_op.deref(ctx).get_parent_region(ctx) == Some(region)
                && !region.deref(ctx).has_ssa_dominance(ctx)
        }

        fn collect_pending_defs(
            &mut self,
            ctx: &Context,
            op: Ptr<Operation>,
        ) -> Vec<Ptr<Operation>> {
            let operands: Vec<_> = op.deref(ctx).operands().collect();
            let mut pending_defs = Vec::new();
            for operand in operands {
                match operand.defining_entity() {
                    DefiningEntity::Op(def_op) => match self.op_states.get(&def_op).copied() {
                        Some(OpState::Processing) => {
                            assert!(
                                Self::is_graph_region_backedge(ctx, op, def_op),
                                "Operation dependency cycles are only valid in graph regions"
                            );
                            log::trace!(
                                "Not waiting on graph-region cycle backedge: {} -> {}",
                                OpDbg { op, ctx },
                                OpDbg { op: def_op, ctx }
                            );
                        }
                        Some(OpState::Processed | OpState::Erased) => {}
                        Some(OpState::Queued) => pending_defs.push(def_op),
                        None if self.op_eligible_for_processing(ctx, def_op) => {
                            self.mark_enqueued(def_op);
                            pending_defs.push(def_op);
                        }
                        None => {}
                    },
                    DefiningEntity::Block(_) => {}
                }
            }
            pending_defs
        }

        fn schedule_pending_defs(&mut self, op: Ptr<Operation>, pending_defs: Vec<Ptr<Operation>>) {
            self.worklist.push_front(WorkItem::Resume(op));
            for def_op in pending_defs.into_iter().rev() {
                self.schedule_enter_front(def_op);
            }
        }

        fn rewrite_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>) -> Result<()> {
            let operands: Vec<_> = op.deref(ctx).operands().collect();
            let operands_info = OperandsInfo::new(
                operands
                    .into_iter()
                    .map(|operand| {
                        (
                            operand,
                            self.previous_types
                                .get(&operand)
                                .cloned()
                                .unwrap_or_default(),
                        )
                    })
                    .collect(),
            );

            log::trace!("Rewriting operation: {}", OpDbg { op, ctx });
            log::trace!(
                "with the following operands info: {}",
                operands_info.disp(ctx)
            );

            self.rewriter
                .set_insertion_point(OpInsertionPoint::BeforeOperation(op));
            self.conversion
                .rewrite(ctx, &mut self.rewriter, op, &operands_info)?;
            self.process_recorder_events(ctx)?;

            if !self.is_erased(op) {
                self.mark_processed(op);
            }
            Ok(())
        }

        fn enter_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>) -> Result<()> {
            log::trace!("Beginning to process operation: {}", OpDbg { op, ctx });
            self.mark_processing(op);

            if !self.conversion.can_convert_op(ctx, op) {
                log::trace!(
                    "Skipping operation as it is not convertible: {}",
                    OpDbg { op, ctx }
                );
                self.mark_processed(op);
                return Ok(());
            }

            let pending_defs = self.collect_pending_defs(ctx, op);
            if pending_defs.is_empty() {
                self.rewrite_operation(ctx, op)
            } else {
                self.schedule_pending_defs(op, pending_defs);
                Ok(())
            }
        }

        fn resume_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>) -> Result<()> {
            if !self.conversion.can_convert_op(ctx, op) {
                log::trace!(
                    "Skipping operation as it is no longer convertible: {}",
                    OpDbg { op, ctx }
                );
                self.mark_processed(op);
                return Ok(());
            }

            // Re-read operands after processing definitions. Rewrites may have
            // replaced an operand with a newly inserted, still-pending definition.
            let pending_defs = self.collect_pending_defs(ctx, op);
            if !pending_defs.is_empty() {
                self.schedule_pending_defs(op, pending_defs);
                log::trace!(
                    "Operation suspended again for newly pending operand definitions: {}",
                    OpDbg { op, ctx }
                );
                return Ok(());
            }

            self.rewrite_operation(ctx, op)
        }

        fn run(&mut self, ctx: &mut Context, root: Ptr<Operation>) -> Result<()> {
            self.collect_operations_blocks(ctx, root);

            // Convert block arguments first
            for block in core::mem::take(&mut self.pending_block_arg_conversions) {
                self.convert_block_arguments(ctx, block)?;
            }

            while let Some(item) = self.worklist.pop_front() {
                match item {
                    WorkItem::Enter(op) => {
                        if self.is_queued(op) {
                            self.enter_operation(ctx, op)?;
                        }
                    }
                    WorkItem::Resume(op) => {
                        if self.is_processing(op) {
                            self.resume_operation(ctx, op)?;
                        }
                    }
                }

                // Convert block arguments for any new blocks added, before processing the next op.
                for block in core::mem::take(&mut self.pending_block_arg_conversions) {
                    self.convert_block_arguments(ctx, block)?;
                }
            }
            Ok(())
        }
    }

    let mut driver = Driver::new(conversion);
    driver.run(ctx, op)?;
    Ok(driver.rewriter.is_modified().into())
}

/// Make [DialectConversion] into a [Pass]
pub struct PassWrapper<C: DialectConversion> {
    name: &'static str,
    conversion: C,
}

impl<C: DialectConversion> PassWrapper<C> {
    pub fn new(name: &'static str, conversion: C) -> Self {
        Self { name, conversion }
    }
}

impl<C: DialectConversion> Pass for PassWrapper<C> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_result = PassResult::default();
        pass_result.ir_changed |= apply_dialect_conversion(ctx, &mut self.conversion, op)?;
        Ok(pass_result)
    }
}
