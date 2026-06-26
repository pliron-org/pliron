//! Utilites for matching operations and rewriting them.
//! Similar in spirit to MLIR's pattern matching rewrites.

use alloc::collections::VecDeque;

use rustc_hash::FxHashSet;

use crate::{
    context::{Context, Ptr},
    graph::walkers::{IRNode, WalkConfig, uninterruptible::immutable::walk_op},
    irbuild::{
        IRStatus,
        inserter::{Inserter, OpInsertionPoint},
        listener::{Recorder, RecorderEvent},
        rewriter::{IRRewriter, Rewriter},
    },
    operation::Operation,
    pass_manager::{AnalysisManager, Pass, PassResult},
    result::Result,
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
/// and then applies a rewrite to them.
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

    let mut erased = FxHashSet::<Ptr<Operation>>::default();
    let mut rewriter = MatchRewriter::default();
    rewriter.set_listener(Recorder::default());

    // Rewrite collected and newly added operations that match.
    while !to_rewrite.is_empty() {
        let op = to_rewrite.pop_front().unwrap();
        if erased.contains(&op) {
            continue;
        }
        rewriter.set_insertion_point(OpInsertionPoint::BeforeOperation(op));
        match_rewrite.rewrite(ctx, &mut rewriter, op)?;
        let listener = rewriter.get_listener_mut();
        // First process all erased operations to avoid dereferencing them.
        for event in &listener.events {
            if let RecorderEvent::ErasedOperation(erased_op) = event {
                erased.insert(*erased_op);
            }
        }
        // Then process all other events.
        for event in &listener.events {
            match event {
                RecorderEvent::ErasedOperation(_) => {
                    // Already processed above.
                }
                RecorderEvent::InsertedOperation(new_op) => {
                    // Check if the newly inserted operation also matches.
                    if !erased.contains(new_op) && match_rewrite.r#match(ctx, *new_op) {
                        match order.enque {
                            EnqueueOrder::EnqueFront => to_rewrite.push_front(*new_op),
                            EnqueueOrder::EnqueBack => to_rewrite.push_back(*new_op),
                        }
                    }
                }
                RecorderEvent::ReplacedValueUses { .. } => {
                    // No action needed for value use replacements.
                }
                RecorderEvent::InsertedBlock(_) => {
                    // No action needed for block insertions.
                }
                RecorderEvent::ErasedBlock(_) => {
                    // No action needed for block erasures.
                    // Operations inside the block will have triggered operation erasure events.
                    // and we only care about operations here.
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
