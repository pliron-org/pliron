use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    linked_list::ContainsLinkedList,
    operation::Operation,
    printable::{self, Printable},
    value::Value,
};

/// Information about whether a block is reachable or not.
#[derive(Clone, Debug)]
pub(super) enum BlockState {
    /// Block is reachable.
    Reachable,
    /// Block is unreachable.
    Unreachable,
}

impl Printable for BlockState {
    fn fmt(
        &self,
        _ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            BlockState::Reachable => write!(f, "Reachable"),
            BlockState::Unreachable => write!(f, "Unreachable"),
        }
    }
}

impl BlockState {
    fn leq(a: &BlockState, b: &BlockState) -> bool {
        matches!(
            (a, b),
            (BlockState::Unreachable, BlockState::Reachable)
                | (BlockState::Unreachable, BlockState::Unreachable)
                | (BlockState::Reachable, BlockState::Reachable)
        )
    }

    fn join(a: &BlockState, b: &BlockState) -> BlockState {
        match (a, b) {
            (BlockState::Reachable, _) | (_, BlockState::Reachable) => BlockState::Reachable,
            (BlockState::Unreachable, BlockState::Unreachable) => BlockState::Unreachable,
        }
    }
}

/// Information about the possible runtime values a static value may be bound to.
#[derive(Clone, Debug)]
pub(super) enum ValState {
    /// No definition or use of this variable is reachable.
    Undefinable,
    /// Value can only be defined as constant `val`.
    Constant { val: AttrObj },
    /// Value definition is reachable, but nothing is known about the dynamic value.
    Unknown,
}

impl Printable for ValState {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            ValState::Undefinable => write!(f, "Unassignable"),
            ValState::Unknown => write!(f, "Unknown"),
            ValState::Constant { val } => {
                write!(f, "Constant(")?;
                Printable::fmt(val, ctx, state, f)?;
                write!(f, ")")
            }
        }
    }
}

impl ValState {
    fn leq(a: &ValState, b: &ValState) -> bool {
        match (a, b) {
            (ValState::Undefinable, _) | (_, ValState::Unknown) => true,
            (ValState::Constant { val: va }, ValState::Constant { val: vb }) => va == vb,
            _ => false,
        }
    }

    fn join(a: &ValState, b: &ValState) -> ValState {
        match (a, b) {
            (ValState::Undefinable, x) | (x, ValState::Undefinable) => x.clone(),
            (ValState::Unknown, _) | (_, ValState::Unknown) => ValState::Unknown,
            (ValState::Constant { val: va }, ValState::Constant { val: vb }) => {
                if va == vb {
                    ValState::Constant { val: va.clone() }
                } else {
                    ValState::Unknown
                }
            }
        }
    }
}

pub(super) struct SccpState {
    /// Maps each block to information about whether the block is reachable
    /// Blocks not present as a keys are assumed to be unreachable.
    block_states: FxHashMap<Ptr<BasicBlock>, BlockState>,
    /// Maps each value to information about whether the value is defined and
    /// what dynamic value it is defined as.
    val_states: FxHashMap<Value, ValState>,
    /// After a [BasicBlock] has been marked as reachable, we must traverse
    /// each of its operations and process them to draw inferences.
    /// This set contains all blocks marked as reachable but not yet traversed.
    block_worklist: FxHashSet<Ptr<BasicBlock>>,
    /// When we infer information about the dynamic value of a [Value], we must
    /// process all its uses to try to infer more information about its uses'
    /// results. This set contains such [Value]s that we have not yet processed.
    val_worklist: FxHashSet<Value>,
}

impl SccpState {
    /// Creates an initial state for analyzing `root_op`. Marks all of `root_op`'s
    /// regions' entry blocks as reachable and their entry-block arguments as Unknown.
    pub(super) fn new(root_op: Ptr<Operation>, ctx: &Context) -> SccpState {
        let mut state = SccpState {
            block_states: FxHashMap::default(),
            val_states: FxHashMap::default(),
            block_worklist: FxHashSet::default(),
            val_worklist: FxHashSet::default(),
        };
        for region in root_op.deref(ctx).regions() {
            let entry = region.deref(ctx).get_head().unwrap();
            state.merge_block_state(ctx, entry, BlockState::Reachable);
            for arg in entry.deref(ctx).arguments() {
                state.merge_val_state(ctx, arg, ValState::Unknown);
            }
        }
        state
    }

    /// Join `incoming` into whatever [ValState] is currently stored at `val`,
    /// storing the result. If the join is strictly greater than the previous stored value,
    /// insert `val` into the value worklist so its users get re-processed.
    pub(super) fn merge_val_state(&mut self, ctx: &Context, val: Value, incoming: ValState) {
        let old = self.get_val_state(val);
        let new = ValState::join(&old, &incoming);
        log::trace!(
            "Merging val state {} into value {}",
            incoming.disp(ctx),
            val.disp(ctx)
        );
        if !ValState::leq(&new, &old) {
            log::trace!("Inflated state of {} to {}", val.disp(ctx), new.disp(ctx));
            self.val_states.insert(val, new);
            self.val_worklist.insert(val);
        }
    }

    /// Join `incoming` into whatever [BlockState] is currently stored at `block`,
    /// storing the result. If the join is strictly greater than the previous stored value,
    /// insert `block` into the block worklist.
    pub(super) fn merge_block_state(
        &mut self,
        ctx: &Context,
        block: Ptr<BasicBlock>,
        incoming: BlockState,
    ) {
        let old = self.get_block_state(block);
        let new = BlockState::join(&old, &incoming);
        log::trace!(
            "Merging block state {} into block {}",
            incoming.disp(ctx),
            block.given_name(ctx).unwrap()
        );
        if !BlockState::leq(&new, &old) {
            log::trace!(
                "Inflated state of {} to {}",
                block.given_name(ctx).unwrap(),
                new.disp(ctx)
            );
            self.block_states.insert(block, new);
            self.block_worklist.insert(block);
        }
    }

    /// Get the [ValState] of `val`.
    pub(super) fn get_val_state(&self, val: Value) -> ValState {
        self.val_states
            .get(&val)
            .cloned()
            .unwrap_or(ValState::Undefinable)
    }

    /// Get the [BlockState] of `block`.
    pub(super) fn get_block_state(&self, block: Ptr<BasicBlock>) -> BlockState {
        self.block_states
            .get(&block)
            .cloned()
            .unwrap_or(BlockState::Unreachable)
    }

    /// Pop an arbitrary block from the block worklist, if any.
    pub(super) fn pop_block(&mut self) -> Option<Ptr<BasicBlock>> {
        let block = self.block_worklist.iter().next().copied()?;
        self.block_worklist.remove(&block);
        Some(block)
    }

    /// Pop an arbitrary value from the value worklist, if any.
    pub(super) fn pop_val(&mut self) -> Option<Value> {
        let val = self.val_worklist.iter().next().copied()?;
        self.val_worklist.remove(&val);
        Some(val)
    }

    /// Are both block and value worklists empty?
    pub(super) fn are_worklists_empty(&self) -> bool {
        self.block_worklist.is_empty() && self.val_worklist.is_empty()
    }
}
