//! IR and control-flow-graph utilities

pub mod dominance;
pub mod traversals;
pub mod visualize;
pub mod walkers;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::hash::Hash;

use crate::{
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    linked_list::{ContainsLinkedList, LinkedList},
    operation::Operation,
    region::Region,
};

/// A trait for graph nodes that can be labeled for visualization purposes.
pub trait HasLabel<GraphContext> {
    fn label(&self, ctx: &GraphContext) -> String;
}

pub trait ControlFlowGraph<GraphContext> {
    type Node: Eq + Hash + Clone + HasLabel<GraphContext>;

    /// Number of successors of `node`.
    fn num_successors(&self, ctx: &GraphContext, node: &Self::Node) -> usize;

    /// Get the `i`-th successor of `node`. Panics if `i` is out of bounds.
    fn get_successor(&self, ctx: &GraphContext, node: &Self::Node, i: usize) -> Self::Node;

    /// Get all successors of `node`.
    /// The default implementation provided may be overridden for better performance.
    fn successors(&self, ctx: &GraphContext, node: &Self::Node) -> Vec<Self::Node> {
        (0..self.num_successors(ctx, node))
            .map(|i| self.get_successor(ctx, node, i))
            .collect()
    }
    /// Number of predecessors of `node`.
    fn num_predecessors(&self, ctx: &GraphContext, node: &Self::Node) -> usize;

    /// Get the `i`-th predecessor of `node`. Panics if `i` is out of bounds.
    fn get_predecessor(&self, ctx: &GraphContext, node: &Self::Node, i: usize) -> Self::Node;

    /// Get all predecessors of `node`.
    /// The default implementation provided may be overridden for better performance.
    fn predecessors(&self, ctx: &GraphContext, node: &Self::Node) -> Vec<Self::Node> {
        (0..self.num_predecessors(ctx, node))
            .map(|i| self.get_predecessor(ctx, node, i))
            .collect()
    }

    /// Returns the entry node of the graph.
    fn entry_node(&self, ctx: &GraphContext) -> Option<Self::Node>;

    /// Get all nodes in the graph.
    fn nodes<'a>(&'a self, ctx: &'a GraphContext) -> Box<dyn Iterator<Item = Self::Node> + 'a>;
}

impl ControlFlowGraph<Context> for Ptr<Region> {
    type Node = Ptr<BasicBlock>;

    fn num_successors(&self, ctx: &Context, node: &Self::Node) -> usize {
        node.deref(ctx).num_succ(ctx)
    }

    fn get_successor(&self, ctx: &Context, node: &Self::Node, i: usize) -> Self::Node {
        node.deref(ctx).get_succ(ctx, i)
    }

    fn successors(&self, ctx: &Context, node: &Self::Node) -> Vec<Self::Node> {
        node.deref(ctx).succs(ctx)
    }

    fn num_predecessors(&self, ctx: &Context, node: &Self::Node) -> usize {
        node.num_preds(ctx)
    }

    fn get_predecessor(&self, ctx: &Context, node: &Self::Node, i: usize) -> Self::Node {
        node.get_pred(ctx, i)
    }

    fn predecessors(&self, ctx: &Context, node: &Self::Node) -> Vec<Self::Node> {
        node.preds(ctx)
    }

    fn entry_node(&self, ctx: &Context) -> Option<Self::Node> {
        self.deref(ctx).get_head()
    }

    fn nodes<'a>(&'a self, ctx: &'a Context) -> Box<dyn Iterator<Item = Self::Node> + 'a> {
        Box::new(self.deref(ctx).iter(ctx))
    }
}

impl HasLabel<Context> for Ptr<BasicBlock> {
    fn label(&self, ctx: &Context) -> String {
        self.deref(ctx).unique_name(ctx).to_string()
    }
}

/// Does operation `a` strictly precede operation `b` in `a`'s block?
pub fn strictly_precedes_in_block(ctx: &Context, a: Ptr<Operation>, b: Ptr<Operation>) -> bool {
    // Quick exit if `a` and `b` are not in the same block, or if they are the same operation.
    if a.deref(ctx).get_parent_block() != b.deref(ctx).get_parent_block() || a == b {
        return false;
    }

    let mut cursor = a.deref(ctx).get_next();
    while let Some(op) = cursor {
        if op == b {
            return true;
        }
        cursor = op.deref(ctx).get_next();
    }
    false
}

/// Returns the ancestor operation of `op` contained in `target_region`, or returns `None`
/// if no ancestor of `op` is in `target_region`.
pub fn find_ancestor_op_of_op_in_region(
    ctx: &Context,
    mut op: Ptr<Operation>,
    target_region: Ptr<Region>,
) -> Option<Ptr<Operation>> {
    while let Some(ancestor_region) = op.deref(ctx).get_parent_region(ctx) {
        if ancestor_region == target_region {
            return Some(op);
        }
        op = ancestor_region.deref(ctx).get_parent_op();
    }
    None
}

/// Returns the ancestor block of `block` contained in `target_region`, or returns `None`
/// if no ancestor of `block` is in `target_region`.
pub fn find_ancestor_block_of_block_in_region(
    ctx: &Context,
    mut block: Ptr<BasicBlock>,
    target_region: Ptr<Region>,
) -> Option<Ptr<BasicBlock>> {
    while let Some(ancestor_region) = block.deref(ctx).get_parent_region() {
        if ancestor_region == target_region {
            return Some(block);
        }
        block = ancestor_region.deref(ctx).get_parent_block(ctx)?;
    }
    None
}

/// Returns the ancestor block of `op` contained in `target_region`, or returns `None`
/// if no ancestor of `op` is in `target_region`.
pub fn find_ancestor_block_of_op_in_region(
    ctx: &Context,
    op: Ptr<Operation>,
    target_region: Ptr<Region>,
) -> Option<Ptr<BasicBlock>> {
    let parent_block = op.deref(ctx).get_parent_block()?;
    find_ancestor_block_of_block_in_region(ctx, parent_block, target_region)
}

/// Returns the ancestor operation of `op` contained in `target_block`, or returns `None`
/// if no ancestor of `op` is in `target_block`.
pub fn find_ancestor_op_of_op_in_block(
    ctx: &Context,
    mut op: Ptr<Operation>,
    target_block: Ptr<BasicBlock>,
) -> Option<Ptr<Operation>> {
    while let Some(parent_block) = op.deref(ctx).get_parent_block() {
        if parent_block == target_block {
            return Some(op);
        }
        op = parent_block.deref(ctx).get_parent_op(ctx)?;
    }
    None
}
