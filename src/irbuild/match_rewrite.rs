// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Utilites for matching operations and rewriting them.
//! Similar in spirit to MLIR's pattern matching rewrites.

use alloc::{collections::VecDeque, vec::Vec};

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    graph::walkers::{IRNode, WalkConfig, uninterruptible::immutable::walk_op},
    irbuild::{
        IRStatus,
        inserter::{Inserter, OpInsertionPoint},
        listener::{Recorder, RecorderEvent},
        rewriter::{IRRewriter, Rewriter},
    },
    operation::Operation,
    pass::{AnalysisManager, Pass, PassResult},
    result::Result,
    utils::table::HSet,
    value::DefiningEntity,
};

/// A rewriter that uses the [Recorder] listener.
pub type MatchRewriter = IRRewriter<Recorder>;

/// Interface for matching and rewriting operations.
pub trait MatchRewrite {
    /// Should operation be rewritten?
    fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool;
    /// Rewrite the matched operation.
    /// Insertion point is set to be before the operation being rewritten.
    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut MatchRewriter,
        op: Ptr<Operation>,
    ) -> Result<()>;
}

/// Should new operations that match be enqueued to the front or back of the queue?
#[derive(Clone, Copy, Debug, Default)]
pub enum EnqueueOrder {
    EnqueFront,
    #[default]
    EnqueBack,
}

/// Configuration for the order of collecting and enqueuing operations.
#[derive(Clone, Debug, Default)]
pub struct RewriterOrder {
    /// Order of initial collection of operations.
    pub collect: WalkConfig,
    /// Order of enqueuing new operations that match.
    pub enque: EnqueueOrder,
}

/// Collects all operations (recursively) that match a given pattern
/// and then applies a rewrite to them. An individual rewrite can trigger
/// more matches.
///
/// Non-rewriter modifications of an operation must be
/// [reported](Rewriter::notify_operation_modified):
/// - for the operation to be matched again.
/// - for the modification to be included in the returned [IRStatus].
pub fn apply_match_rewrite<M: MatchRewrite>(
    ctx: &mut Context,
    match_rewrite: &mut M,
    order: RewriterOrder,
    op: Ptr<Operation>,
) -> Result<IRStatus> {
    let mut to_rewrite = VecDeque::new();

    // Collect all operations that match.
    struct WalkerState<'a, M> {
        match_rewrite: &'a mut M,
        to_rewrite: &'a mut VecDeque<Ptr<Operation>>,
    }
    let mut state = WalkerState {
        match_rewrite,
        to_rewrite: &mut to_rewrite,
    };
    // A callback for the walker.
    fn walker_callback<M: MatchRewrite>(ctx: &Context, state: &mut WalkerState<M>, node: IRNode) {
        if let IRNode::Operation(op) = node
            && state.match_rewrite.r#match(ctx, op)
        {
            state.to_rewrite.push_back(op);
        }
    }
    // Walk the operation tree.
    walk_op(ctx, &mut state, &order.collect, op, walker_callback);

    let mut erased = HSet::<Ptr<Operation>>::default();
    let mut erased_blocks = HSet::<Ptr<BasicBlock>>::default();
    let mut rewriter = MatchRewriter::default();
    rewriter.set_listener(Recorder::default());

    // Rewrite collected and newly added/modified operations that match.
    while !to_rewrite.is_empty() {
        let op = to_rewrite.pop_front().unwrap();
        if erased.contains(&op) {
            continue;
        }
        rewriter.set_insertion_point(OpInsertionPoint::BeforeOperation(op));
        match_rewrite.rewrite(ctx, &mut rewriter, op)?;
        let listener = rewriter.get_listener_mut();

        // 1. First process all erased operations and blocks to avoid dereferencing them.
        for event in &listener.events {
            match event {
                RecorderEvent::ErasedOperation(erased_op) => {
                    erased.insert(*erased_op);
                }
                RecorderEvent::ErasedBlock(erased_block) => {
                    erased_blocks.insert(*erased_block);
                }
                _ => {}
            }
        }
        // Operations to match and enqueue, in the order of the events.
        let mut candidates = Vec::new();
        let mut is_candidate = HSet::<Ptr<Operation>>::default();
        let mut add_candidate = |op: Ptr<Operation>| {
            if !erased.contains(&op) && is_candidate.insert(op) {
                candidates.push(op);
            }
        };

        // 2. Then process all other events.
        for event in &listener.events {
            match event {
                RecorderEvent::ErasedOperation(_) => {
                    // Already processed above.
                }
                RecorderEvent::InsertedOperation(new_op) => {
                    // Check if the newly inserted operation also matches.
                    add_candidate(*new_op);
                }
                RecorderEvent::ModifiedOperation(modified_op) => {
                    // Check if the modified operation matches now.
                    add_candidate(*modified_op);
                }
                RecorderEvent::ReplacedValueUses { new_value, .. } => {
                    // A later event may have erased `new_value`.
                    let is_defined = match new_value.defining_entity() {
                        DefiningEntity::Op(def_op) => !erased.contains(&def_op),
                        DefiningEntity::Block(def_block) => !erased_blocks.contains(&def_block),
                    };
                    if is_defined {
                        for r#use in new_value.uses(ctx) {
                            // The user may have a different operand now. Check if it matches now.
                            add_candidate(r#use.user_op());
                        }
                    }
                }
                RecorderEvent::InsertedBlock(_) => {
                    // No action needed for block insertions.
                }
                RecorderEvent::ErasedBlock(_) => {
                    // Already processed above.
                }
                RecorderEvent::ErasedRegion(_) => {
                    // No action needed for region erasures.
                    // Operations inside the region will have triggered operation erasure events.
                    // and we only care about operations here.
                }
                RecorderEvent::ValueTypeChanged { .. } => {
                    // No action needed for type changes.
                }
                RecorderEvent::UnlinkedOperation(_op, _prev_position) => {
                    // No action needed for operation unlinking.
                }
                RecorderEvent::UnlinkedBlock(_block, _prev_position) => {
                    // No action needed for block unlinking.
                }
            }
        }
        listener.clear();

        // 3. Enqueue matching candidates.
        for candidate in candidates {
            if match_rewrite.r#match(ctx, candidate) {
                match order.enque {
                    EnqueueOrder::EnqueFront => to_rewrite.push_front(candidate),
                    EnqueueOrder::EnqueBack => to_rewrite.push_back(candidate),
                }
            }
        }
    }
    Ok(rewriter.is_modified().into())
}

/// Make [MatchRewrite] into a [Pass]
pub struct PassWrapper<M: MatchRewrite> {
    name: &'static str,
    match_rewrite: M,
    rewrite_order: RewriterOrder,
}

impl<M: MatchRewrite> PassWrapper<M> {
    /// Create a new [PassWrapper] with the given name and [MatchRewrite].
    pub fn new(name: &'static str, match_rewrite: M) -> Self {
        Self {
            name,
            match_rewrite,
            rewrite_order: RewriterOrder::default(),
        }
    }

    /// Set the rewrite order for the pass.
    pub fn set_rewrite_order(mut self, order: RewriterOrder) -> Self {
        self.rewrite_order = order;
        self
    }
}

impl<M: MatchRewrite> Pass for PassWrapper<M> {
    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_result = PassResult::default();
        pass_result.ir_changed |=
            apply_match_rewrite(ctx, &mut self.match_rewrite, self.rewrite_order.clone(), op)?;
        Ok(pass_result)
    }

    fn name(&self) -> &str {
        self.name
    }
}
