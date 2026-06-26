//! Utilities for dialect conversion style rewrites.
//! Similar in spirit to MLIR dialect conversion, but intentionally simpler:
//! - no unrealized conversion casts,
//! - definitions are always converted before their uses.

use core::cell::Ref;

use alloc::{collections::vec_deque::VecDeque, vec, vec::Vec};
use rustc_hash::FxHashMap;

use crate::{
    builtin::op_interfaces::IsTerminatorInterface,
    context::{Context, Ptr},
    graph::walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    irbuild::{
        IRStatus,
        inserter::{Inserter, OpInsertionPoint},
        listener::{Recorder, RecorderEvent},
        rewriter::{IRRewriter, Rewriter},
    },
    irfmt::printers::list_with_sep,
    op::op_impls,
    operation::{OpDbg, Operation},
    pass::{AnalysisManager, Pass, PassResult},
    printable::{ListSeparator, Printable},
    result::Result,
    r#type::{Type, TypeHandle, Typed},
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
    /// All operands are already converted before this callback is invoked.
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
/// operand definitions are rewritten before rewriting the current operation.
///
/// Block argument types are updated only in two cases:
/// 1. The block argument is an operand use of an operation being processed.
/// 2. An operation being processed has successor blocks, in which case all
///    arguments of those successor blocks are considered for type conversion.
//
// ## Algorithm
//
// 1. Collect all initially convertible operations (and terminators) into a
//    worklist.
// 2. Repeatedly pop from the front; only entries still marked `Queued` are
//    processed.
// 3. For each op, convert relevant block-argument types, then check operand
//    defining ops. If defs are still pending, re-enqueue this op and those defs
//    to the front so defs are handled first.
// 4. Actually call the conversion pattern's `rewrite` callback.
// 5. Post rewrite, process recorder events:
//    - mark erased ops,
//    - update value type-history,
//    - enqueue newly inserted convertible ops.
// 6. Mark rewritten/non-convertible ops as `Processed`.
pub fn apply_dialect_conversion<C: DialectConversion>(
    ctx: &mut Context,
    conversion: &mut C,
    op: Ptr<Operation>,
) -> Result<IRStatus> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum OpState {
        Queued,
        Processed,
        Erased,
    }

    struct Driver<'a, C: DialectConversion> {
        conversion: &'a mut C,
        rewriter: DialectConversionRewriter,
        worklist: VecDeque<Ptr<Operation>>,
        op_states: FxHashMap<Ptr<Operation>, OpState>,
        previous_types: FxHashMap<Value, Vec<TypeHandle>>,
    }

    impl<'a, C: DialectConversion> Driver<'a, C> {
        fn new(conversion: &'a mut C) -> Self {
            let mut rewriter = DialectConversionRewriter::default();
            rewriter.set_listener(Recorder::default());
            Self {
                conversion,
                rewriter,
                worklist: VecDeque::new(),
                op_states: FxHashMap::default(),
                previous_types: FxHashMap::default(),
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

        fn mark_erased(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Erased);
        }

        fn mark_processed(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Processed);
        }

        fn mark_enqueued(&mut self, op: Ptr<Operation>) {
            self.op_states.insert(op, OpState::Queued);
        }

        fn enqueue_front(&mut self, op: Ptr<Operation>) {
            assert!(
                !self.is_processed(op) && !self.is_erased(op),
                "Attempted to enqueue an operation that is already terminal-state (processed/erased)"
            );
            self.mark_enqueued(op);
            self.worklist.push_front(op);
        }

        fn enqueue_back(&mut self, op: Ptr<Operation>) {
            assert!(
                !self.is_processed(op) && !self.is_erased(op),
                "Attempted to enqueue an operation that is already terminal-state (processed/erased)"
            );
            self.mark_enqueued(op);
            self.worklist.push_back(op);
        }

        fn op_eligible_for_processing(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if self.is_erased(op) || self.is_processed(op) {
                return false;
            }
            self.conversion.can_convert_op(ctx, op)
                || op_impls::<dyn IsTerminatorInterface>(&*Operation::get_op_dyn(op, ctx))
        }

        fn collect_operations(&mut self, ctx: &mut Context, root: Ptr<Operation>) {
            self.worklist.clear();
            self.op_states.clear();
            fn walker_callback<C: DialectConversion>(
                ctx: &Context,
                driver: &mut Driver<C>,
                node: IRNode,
            ) {
                if let IRNode::Operation(op) = node
                    && driver.op_eligible_for_processing(ctx, op)
                {
                    driver.enqueue_back(op);
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

        fn convert_successor_block_argument_types(
            &mut self,
            ctx: &mut Context,
            op: Ptr<Operation>,
        ) -> Result<()> {
            let successors: Vec<_> = op.deref(ctx).successors().collect();
            for succ in successors {
                let args: Vec<_> = succ.deref(ctx).arguments().collect();
                for arg in args {
                    self.convert_block_argument_type(ctx, arg)?;
                }
            }
            Ok(())
        }

        fn process_recorder_events(&mut self, ctx: &mut Context) -> Result<()> {
            let events = {
                let listener = self.rewriter.get_listener_mut();
                core::mem::take(&mut listener.events)
            };

            for event in &events {
                if let RecorderEvent::ErasedOperation(op) = event {
                    self.mark_erased(*op);
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
                if let RecorderEvent::InsertedOperation(new_op) = event
                    && self.op_eligible_for_processing(ctx, new_op)
                    && !self.is_queued(new_op)
                {
                    log::trace!(
                        "Inserted operation added to worklist: {}",
                        OpDbg { op: new_op, ctx }
                    );
                    self.enqueue_back(new_op);
                }
            }

            Ok(())
        }

        fn process_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>) -> Result<()> {
            log::trace!("Beginning to process operation: {}", OpDbg { op, ctx });

            self.convert_successor_block_argument_types(ctx, op)?;

            if !self.conversion.can_convert_op(ctx, op) {
                log::trace!(
                    "Skipping operation as it is not convertible: {}",
                    OpDbg { op, ctx }
                );
                self.mark_processed(op);
                return Ok(());
            }

            let operands: Vec<_> = op.deref(ctx).operands().collect();
            let mut pending_defs = Vec::new();
            for operand in &operands {
                match operand.defining_entity() {
                    DefiningEntity::Op(def_op) => {
                        assert_ne!(def_op, op, "Operation cannot depend on its own result");
                        if self.op_eligible_for_processing(ctx, def_op) {
                            pending_defs.push(def_op);
                        }
                    }
                    DefiningEntity::Block(_) => self.convert_block_argument_type(ctx, *operand)?,
                }
            }

            if !pending_defs.is_empty() {
                // We aren't going to process it now, so add it back to the queue
                // to be processed after its operands' defs are processed.
                self.enqueue_front(op);
                for def_op in pending_defs.into_iter().rev() {
                    self.enqueue_front(def_op);
                }
                log::trace!(
                    "Operation re-enqueued, to be processed after its operands' defs: {}",
                    OpDbg { op, ctx }
                );
                return Ok(());
            }

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

            self.mark_processed(op);
            Ok(())
        }

        fn run(&mut self, ctx: &mut Context, root: Ptr<Operation>) -> Result<()> {
            self.collect_operations(ctx, root);
            while let Some(op) = self.worklist.pop_front() {
                // Skip stale duplicate entries and ops that became terminal-state
                // while queued. For the queued case, remove the queue-state first so
                // this op can be re-enqueued if it must be deferred.
                match self.op_states.get(&op).copied() {
                    Some(OpState::Queued) => {
                        self.op_states.remove(&op);
                        self.process_operation(ctx, op)?;
                    }
                    Some(OpState::Processed | OpState::Erased) | None => continue,
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
