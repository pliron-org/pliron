use alloc::vec::Vec;
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
    /// Lattice bottom
    Reachable,
    /// Block is unreachable.
    /// Lattice top.
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
            (BlockState::Reachable, BlockState::Unreachable)
                | (BlockState::Unreachable, BlockState::Unreachable)
                | (BlockState::Reachable, BlockState::Reachable)
        )
    }

    fn meet(a: &BlockState, b: &BlockState) -> BlockState {
        match (a, b) {
            (BlockState::Reachable, _) | (_, BlockState::Reachable) => BlockState::Reachable,
            (BlockState::Unreachable, BlockState::Unreachable) => BlockState::Unreachable,
        }
    }
}

/// Represents compile-time knowledge about the "constantness" of a
/// of a variable (SSA value)  during execution of the algorithm.
///
/// Partial ordering relation:
/// ```text
///                         Undetermined  (⊤)
///                      /       |       \
///                     /        |        \
///         ... Constant{c₀} Constant{c₁} Constant{c₂} ...
///                     \        |        /
///                      \       |       /
///                         NotAConstant  (⊥)
/// ```
#[derive(Clone, Debug)]
pub(super) enum Constness {
    /// The analysis has not determined whether this is a constant or not.
    /// The lattice ⊤.
    Undetermined,
    /// The analysis has (as yet) seen only one definition of the variable (SSA Value),
    /// which assigns `val` to it.
    Constant { val: AttrObj },
    /// The analysis cannot prove that this variable (SSA value) is a constant.
    /// The lattice ⊥.
    NotAConstant,
}

impl Printable for Constness {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            Constness::Undetermined => write!(f, "Undetermined"),
            Constness::NotAConstant => write!(f, "NotAConstant"),
            Constness::Constant { val } => {
                write!(f, "Constant(")?;
                Printable::fmt(val, ctx, state, f)?;
                write!(f, ")")
            }
        }
    }
}

impl Constness {
    fn leq(a: &Constness, b: &Constness) -> bool {
        match (a, b) {
            (Constness::NotAConstant, _) | (_, Constness::Undetermined) => true,
            (Constness::Constant { val: va }, Constness::Constant { val: vb }) => va == vb,
            _ => false,
        }
    }

    fn meet(a: &Constness, b: &Constness) -> Constness {
        match (a, b) {
            (Constness::NotAConstant, _) | (_, Constness::NotAConstant) => Constness::NotAConstant,
            (Constness::Undetermined, x) | (x, Constness::Undetermined) => x.clone(),
            (Constness::Constant { val: va }, Constness::Constant { val: vb }) => {
                if va == vb {
                    Constness::Constant { val: va.clone() }
                } else {
                    Constness::NotAConstant
                }
            }
        }
    }
}

pub(super) struct SccpState {
    /// Maps each block to information about whether the block is reachable
    /// Blocks not present as keys are assumed to be unreachable.
    block_states: FxHashMap<Ptr<BasicBlock>, BlockState>,
    /// Maps each [Value] to information about whether the [Value] is known to be const
    /// [Value]s not present are assumed to be undetermined.
    val_states: FxHashMap<Value, Constness>,
    /// After a [BasicBlock] has been marked as reachable, we must traverse
    /// each of its operations and process them to draw inferences.
    /// This set contains all blocks marked as reachable but not yet traversed.
    block_worklist: FxHashSet<Ptr<BasicBlock>>,
    /// When we infer information about the constness of a [Value], we must
    /// process all its uses to try to infer more information about the constness
    /// of its uses' results. This set contains such [Value]s that we have not yet
    /// processed.
    val_worklist: FxHashSet<Value>,
}

impl SccpState {
    /// Creates an initial state for analyzing `root_op`. Marks all of `root_op`'s
    /// regions' entry blocks as Reachable and their entry-block arguments as NotAConstant.
    pub(super) fn new(root_op: Ptr<Operation>, ctx: &Context) -> SccpState {
        let mut state = SccpState {
            block_states: FxHashMap::default(),
            val_states: FxHashMap::default(),
            block_worklist: FxHashSet::default(),
            val_worklist: FxHashSet::default(),
        };
        state.seed_nested_regions(root_op, ctx);
        state
    }

    /// For ops that do not implement a `RegionBranchOpInterface` analog,
    /// propagate information from the op to its nested regions conservatively
    /// by marking each of their entry blocks as Reachable and each of their
    /// entry blocks' arguments as NotAConstant.
    pub(super) fn seed_nested_regions(&mut self, op: Ptr<Operation>, ctx: &Context) {
        let regions: Vec<_> = op.deref(ctx).regions().collect();
        for region in regions {
            let Some(entry) = region.deref(ctx).get_head() else {
                continue;
            };
            self.merge_block_state(ctx, entry, BlockState::Reachable);
            let entry_args: Vec<Value> = entry.deref(ctx).arguments().collect();
            for arg in entry_args {
                self.merge_val_state(ctx, arg, Constness::NotAConstant);
            }
        }
    }

    /// Meet `incoming` with whatever [Constness] is currently stored at `val`,
    /// storing the result. If the meet is strictly less than the previous stored value,
    /// insert `val` into the value worklist so its users get re-processed.
    pub(super) fn merge_val_state(&mut self, ctx: &Context, val: Value, incoming: Constness) {
        let old = self.get_val_state(val);
        let new = Constness::meet(&old, &incoming);
        log::trace!(
            "Merging val state {} into value {}",
            incoming.disp(ctx),
            val.disp(ctx)
        );
        if !Constness::leq(&old, &new) {
            log::trace!("Deflated state of {} to {}", val.disp(ctx), new.disp(ctx));
            self.val_states.insert(val, new);
            self.val_worklist.insert(val);
        }
    }

    /// Meet `incoming` into whatever [BlockState] is currently stored at `block`,
    /// storing the result. If the meet is strictly less than the previous stored value,
    /// insert `block` into the block worklist.
    pub(super) fn merge_block_state(
        &mut self,
        ctx: &Context,
        block: Ptr<BasicBlock>,
        incoming: BlockState,
    ) {
        let old = self.get_block_state(block);
        let new = BlockState::meet(&old, &incoming);
        log::trace!(
            "Merging block state {} into block {}",
            incoming.disp(ctx),
            block.unique_name(ctx)
        );
        if !BlockState::leq(&old, &new) {
            log::trace!(
                "Deflated state of {} to {}",
                block.unique_name(ctx),
                new.disp(ctx)
            );
            self.block_states.insert(block, new);
            self.block_worklist.insert(block);
        }
    }

    /// Get the [Constness] of `val`.
    pub(super) fn get_val_state(&self, val: Value) -> Constness {
        self.val_states
            .get(&val)
            .cloned()
            .unwrap_or(Constness::Undetermined)
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
