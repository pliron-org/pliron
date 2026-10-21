// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use alloc::vec::Vec;

use crate::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    context::{Context, Ptr},
    graph::walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    irfmt::printers::enclosed,
    linked_list::{ContainsLinkedList, LinkedList},
    operation::{OpDbg, Operation},
    printable::{self, Printable},
    utils::table::{HMap, HSet, ISet},
    value::Value,
};

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
            Constness::Constant { val } => enclosed("Constant(", ")", val).fmt(ctx, state, f),
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
    /// The outermost operation whose regions are analyzed.
    root_op: Ptr<Operation>,
    /// Maps each [Value] to its known [Constness].
    /// All local [Value]s are initialized to [Constness::Undetermined].
    val_states: HMap<Value, Constness>,
    /// Operations that the analysis has marked as reachable.
    reachable_ops: HSet<Ptr<Operation>>,
    /// Reachable operations that require further processing.
    op_worklist: ISet<Ptr<Operation>>,
}

impl SccpState {
    /// Creates an initial state for analyzing `root_op`. Marks the entry blocks of
    /// `root_op`'s regions as reachable, and their arguments as `NotAConstant`.
    pub(super) fn new(root_op: Ptr<Operation>, ctx: &Context) -> SccpState {
        let mut state = SccpState {
            root_op,
            val_states: HMap::default(),
            reachable_ops: HSet::default(),
            op_worklist: ISet::default(),
        };
        walk_op(
            ctx,
            &mut (&mut state.val_states, root_op),
            &WALKCONFIG_PREORDER_FORWARD,
            root_op,
            |ctx, (values, root_op), node| match node {
                // SCCP does not process the root (treating its results as `NotAConstant`).
                IRNode::Operation(op) if op == *root_op => {}
                IRNode::Operation(op) => values.extend(
                    op.deref(ctx)
                        .results()
                        .map(|value| (value, Constness::Undetermined)),
                ),
                IRNode::BasicBlock(block) => values.extend(
                    block
                        .deref(ctx)
                        .arguments()
                        .map(|value| (value, Constness::Undetermined)),
                ),
                IRNode::Region(_) => {}
            },
        );
        state.seed_nested_regions(root_op, ctx);
        state
    }

    /// Mark the entry blocks of `op`'s regions as reachable and
    /// each of their block arguments as [Constness::NotAConstant].
    pub(super) fn seed_nested_regions(&mut self, op: Ptr<Operation>, ctx: &Context) {
        let regions: Vec<_> = op.deref(ctx).regions().collect();
        for region in regions {
            let Some(entry) = region.deref(ctx).get_entry_block() else {
                continue;
            };
            let entry_args: Vec<Value> = entry.deref(ctx).arguments().collect();
            for arg in entry_args {
                self.merge_val_state(ctx, arg, Constness::NotAConstant);
            }
            self.mark_block_reachable(ctx, entry);
        }
    }

    /// Meet `incoming` with the currently known [Constness] of `val`:
    ///   - Update `val`'s [Constness] with the meet.
    ///   - If the meet is strictly below the previous [Constness], insert
    ///     the reachable users of `val` into the operation worklist.
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
            for user in val.uses(ctx).into_iter().map(|u| u.user_op()) {
                if self.is_reachable(user) {
                    self.op_worklist.insert(user);
                }
            }
        }
    }

    /// Mark `op` as reachable. If it was unreachable before,
    /// insert it into the operation worklist.
    fn mark_reachable(&mut self, ctx: &Context, op: Ptr<Operation>) {
        if self.reachable_ops.insert(op) {
            log::trace!("Marked {} as reachable", OpDbg { op, ctx });
            self.op_worklist.insert(op);
        }
    }

    /// Mark the first operation of `block` as reachable.
    pub(super) fn mark_block_reachable(&mut self, ctx: &Context, block: Ptr<BasicBlock>) {
        let first = block.deref(ctx).get_head();
        if let Some(first) = first {
            self.mark_reachable(ctx, first);
        }
    }

    /// Mark the operation after `op` (in its block) as reachable.
    pub(super) fn mark_next_reachable(&mut self, ctx: &Context, op: Ptr<Operation>) {
        if op == self.root_op {
            return;
        }
        let next = op.deref(ctx).get_next();
        if let Some(next) = next {
            self.mark_reachable(ctx, next);
        }
    }

    /// Get the [Constness] of `val`.
    pub(super) fn get_val_state(&self, val: Value) -> Constness {
        self.val_states
            .get(&val)
            .cloned()
            .unwrap_or(Constness::NotAConstant)
    }

    /// Has the analysis marked `op` as reachable?
    pub(super) fn is_reachable(&self, op: Ptr<Operation>) -> bool {
        self.reachable_ops.contains(&op)
    }

    /// Has the analysis marked the first operation of `block` as reachable?
    pub(super) fn is_block_reachable(&self, ctx: &Context, block: Ptr<BasicBlock>) -> bool {
        block
            .deref(ctx)
            .get_head()
            .is_some_and(|first| self.is_reachable(first))
    }

    /// Pop an arbitrary operation (if any) from the operation worklist.
    pub(super) fn pop_op(&mut self) -> Option<Ptr<Operation>> {
        self.op_worklist.pop()
    }
}
