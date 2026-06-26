use alloc::{vec, vec::Vec};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    graph::{
        ControlFlowGraph, find_ancestor_block_of_block_in_region, find_ancestor_op_of_op_in_region,
        strictly_precedes_in_block, traversals,
    },
    operation::Operation,
    pass::{Analysis, AnalysisManager},
    region::Region,
    result::Result,
    value::{DefiningEntity, Value},
};

/// A node in the dominator tree.
struct DomTreeNode<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// The immediate dominator of self.
    parent: Option<G::Node>,
    /// The nodes that self immediately dominates.
    children: Vec<G::Node>,
}

/// Represents dominator tree for a control-flow-graph
pub struct DomTree<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    // An empty tree has no root.
    root: Option<G::Node>,
    dominators_map: FxHashMap<G::Node, DomTreeNode<G, GraphContext>>,
}

/// Maps each node to its dominance frontier
pub struct DomFrontierMap<G, GraphContext>(FxHashMap<G::Node, FxHashSet<G::Node>>)
where
    G: ControlFlowGraph<GraphContext>;

/// Computes a dominator tree for `graph`.
/// Only considers nodes reachable from the entry node of the graph.
// An implementation of the algorithm from page 7 of
// "A Simple, Fast Dominance Algorithm" by Cooper et. al.
pub fn compute_dominator_tree<G, GraphContext>(
    ctx: &GraphContext,
    graph: &G,
) -> DomTree<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    let Some(entry_node) = graph.entry_node(ctx) else {
        return DomTree {
            root: None,
            dominators_map: FxHashMap::default(),
        };
    };

    // We consider only the first connected component for the dominator tree,
    // since the entry node is guaranteed to be in the first component,
    // and all nodes not reachable from the entry node are not dominated by the entry.
    // (i.e. they are roots of their own dominator trees - which we don't care about).
    let rpo = traversals::region::topological_order_by_component(ctx, graph)
        .into_iter()
        .next()
        .expect("Graph has no components, but has an entry node");

    let rpo_index: FxHashMap<G::Node, usize> = rpo
        .iter()
        .enumerate()
        .map(|(i, node)| (node.clone(), i))
        .collect();

    let mut dom: Vec<Option<usize>> = vec![None; rpo.len()];
    assert!(
        rpo_index[&entry_node] == 0,
        "Entry node is not the first block in our CFG"
    );
    dom[0] = Some(0);

    fn intersect(mut finger1: usize, mut finger2: usize, dom: &[Option<usize>]) -> usize {
        while finger1 != finger2 {
            while finger1 > finger2 {
                finger1 = dom[finger1].unwrap();
            }
            while finger2 > finger1 {
                finger2 = dom[finger2].unwrap();
            }
        }
        finger1
    }

    let mut changed = true;
    while changed {
        changed = false;

        for (i, node) in rpo.iter().enumerate().skip(1) {
            let preds = graph.predecessors(ctx, node);
            // only consider predecessors reachable from entry (exactly the predecessors in rpo_index)
            let reachable_preds = preds.iter().filter(|p| rpo_index.contains_key(p));

            // new_idom <- first (processed) predecessor of b (pick one)
            let picked_pred = reachable_preds
                .clone()
                .find(|p| dom[rpo_index[p]].is_some())
                .unwrap();
            let mut new_idom = rpo_index[picked_pred];

            // for all other (reachable) predecessors, p, of b:
            for pred in reachable_preds.filter(|p| *p != picked_pred) {
                let pred_idx = rpo_index[pred];
                match dom[pred_idx] {
                    None => {}
                    Some(_) => {
                        new_idom = intersect(pred_idx, new_idom, &dom);
                    }
                }
            }

            if dom[i] != Some(new_idom) {
                dom[i] = Some(new_idom);
                changed = true;
            }
        }
    }

    let mut dom_tree = DomTree {
        root: Some(entry_node),
        dominators_map: FxHashMap::default(),
    };
    let entry = DomTreeNode {
        parent: None,
        children: vec![],
    };
    dom_tree.dominators_map.insert(rpo[0].clone(), entry);

    let child_parent = dom
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, parent)| (rpo[i].clone(), rpo[parent.unwrap()].clone()));

    for (child_node, parent_node) in child_parent {
        let child_dom_node = DomTreeNode {
            parent: Some(parent_node.clone()),
            children: vec![],
        };
        dom_tree
            .dominators_map
            .insert(child_node.clone(), child_dom_node);
        let parent_dom_node = dom_tree.dominators_map.get_mut(&parent_node).unwrap();
        parent_dom_node.children.push(child_node.clone())
    }

    dom_tree
}

impl<G, GraphContext> DomTree<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// Does `dominator` dominate `dominatee`?
    pub fn dominates(&self, dominator: &G::Node, dominatee: &G::Node) -> bool {
        let mut node_opt = Some(dominatee.clone());
        while let Some(node) = node_opt {
            if node == *dominator {
                return true;
            }
            node_opt = self.dominators_map[&node].parent.clone();
        }
        false
    }

    /// Nearest common dominator of `node1` and `node2`.
    pub fn nearest_common_dominator(&self, node1: &G::Node, node2: &G::Node) -> G::Node {
        self.dominators(node1)
            .find(|node1_dom| self.dominates(node1_dom, node2))
            .expect("For nodes reachable from entry, a common dominator must exist")
    }

    /// Does the dominator tree contain `node`?
    /// That is, is `node` reachable from the entry node?
    pub fn contains(&self, node: &G::Node) -> bool {
        self.dominators_map.contains_key(node)
    }

    /// Return the immediate dominator of `node`
    pub fn idom(&self, node: &G::Node) -> Option<G::Node> {
        self.dominators_map[node].parent.clone()
    }

    /// Return an iterator over the dominators of `node`, starting with `node` itself,
    /// then its immediate dominator, and so on up to the root.
    pub fn dominators(&self, node: &G::Node) -> impl Iterator<Item = G::Node> + Clone + '_ {
        core::iter::successors(Some(node.clone()), |n| {
            self.dominators_map[n].parent.clone()
        })
    }

    /// Get an iterator over the children nodes
    pub fn children(&self, node: &G::Node) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.dominators_map[node].children.iter().cloned()
    }

    /// Get the root of the dominator tree (i.e. the entry node of the graph)
    /// Returns `None` if the graph has no entry node (empty dominator tree).
    pub fn root(&self) -> Option<G::Node> {
        self.root.clone()
    }

    /// Get the number of nodes in the dominator tree
    pub fn num_nodes(&self) -> usize {
        self.dominators_map.len()
    }

    /// Get an iterator over all nodes in the dominator tree
    pub fn nodes(&self) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.dominators_map.keys().cloned()
    }
}

impl<G, GraphContext> DomFrontierMap<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// Construct `graph`'s dominance frontier map, given that `dom_tree` is the dominance tree
    /// generated from `graph`
    // This method implements the algorithm from "A Simple, Fast Dominance Algorithm" by Cooper et. al.
    pub fn new(ctx: &GraphContext, graph: &G, dom_tree: &DomTree<G, GraphContext>) -> Self {
        let mut res: FxHashMap<G::Node, FxHashSet<G::Node>> = graph
            .nodes(ctx)
            .map(|n| (n, FxHashSet::default()))
            .collect();
        for b in graph.nodes(ctx) {
            if graph.num_predecessors(ctx, &b) < 2 {
                continue;
            }
            let preds = graph.predecessors(ctx, &b);
            let b_idom = dom_tree.idom(&b).unwrap();
            for p in preds.into_iter().filter(|n| dom_tree.contains(n)) {
                for runner in dom_tree.dominators(&p).take_while(|n| *n != b_idom) {
                    res.get_mut(&runner).unwrap().insert(b.clone());
                }
            }
        }
        DomFrontierMap(res)
    }

    /// Gets the set of all nodes in `n`'s dominance frontier
    pub fn frontier<'a>(&'a self, n: &G::Node) -> &'a FxHashSet<G::Node> {
        &self.0[n]
    }
}

/// Tries to update `a` and `b` to blocks in the same region.
///
/// This mirrors MLIR's `tryGetBlocksInSameRegion`: if the input blocks are in different
/// regions, each block is lifted through ancestor blocks until both blocks are in a common
/// region, or `None` is returned if there is no common region.
fn try_get_blocks_in_same_region(
    ctx: &Context,
    mut a: Ptr<BasicBlock>,
    mut b: Ptr<BasicBlock>,
) -> Option<(Ptr<BasicBlock>, Ptr<BasicBlock>)> {
    // Fast path: if both blocks already live in the same region, there is nothing to lift.
    let a_region = a.deref(ctx).get_parent_region()?;
    let b_region = b.deref(ctx).get_parent_region()?;
    if a_region == b_region {
        return Some((a, b));
    }

    // Walk `a` up its ancestor blocks looking for one that already lives in `b`'s region.
    let mut a_region_depth = 0usize;
    let mut a_cursor = Some(a);
    while let Some(block) = a_cursor {
        a_region_depth += 1;
        if block.deref(ctx).get_parent_region() == Some(b_region) {
            a = block;
            return Some((a, b));
        }
        a_cursor = block.deref(ctx).get_parent_block(ctx);
    }

    // Symmetrically, walk `b` up its ancestor blocks looking for one in `a`'s region.
    let mut b_region_depth = 0usize;
    let mut b_cursor = Some(b);
    while let Some(block) = b_cursor {
        b_region_depth += 1;
        if block.deref(ctx).get_parent_region() == Some(a_region) {
            b = block;
            return Some((a, b));
        }
        b_cursor = block.deref(ctx).get_parent_block(ctx);
    }

    let mut a_opt = Some(a);
    let mut b_opt = Some(b);

    // If neither side reaches the other's region directly, equalize their region depths first.
    while a_region_depth > b_region_depth {
        a_opt = a_opt.and_then(|block| block.deref(ctx).get_parent_block(ctx));
        a_region_depth -= 1;
    }
    while b_region_depth > a_region_depth {
        b_opt = b_opt.and_then(|block| block.deref(ctx).get_parent_block(ctx));
        b_region_depth -= 1;
    }

    // With both sides at the same depth, walk them upward in lockstep until a common region is found.
    while let (Some(a_block), Some(b_block)) = (a_opt, b_opt) {
        if a_block.deref(ctx).get_parent_region() == b_block.deref(ctx).get_parent_region() {
            return Some((a_block, b_block));
        }
        a_opt = a_block.deref(ctx).get_parent_block(ctx);
        b_opt = b_block.deref(ctx).get_parent_block(ctx);
    }

    // The blocks do not share any common ancestor region.
    None
}

/// Caches dominance trees for multiple regions in a program
#[derive(Default)]
pub struct DomInfo(FxHashMap<Ptr<Region>, DomTree<Ptr<Region>, Context>>);

impl DomInfo {
    /// If dominator tree for `region` is cached, return it.
    /// Otherwise, computes, caches, and returns the `region`'s dominator tree.
    pub fn get_dom_tree(
        &mut self,
        ctx: &Context,
        region: Ptr<Region>,
    ) -> &DomTree<Ptr<Region>, Context> {
        self.0
            .entry(region)
            .or_insert_with(|| compute_dominator_tree(ctx, &region))
    }

    /// Does block `a` strictly dominate block `b`?
    ///
    /// Caches region dominator trees as a side effect.
    ///
    /// Defining `BlockB_` as a block immediately in `BlockA`'s region, and either
    /// 1. is the same as `BlockB`, OR
    /// 2. contains `BlockB` in some nested region.
    ///
    /// This function returns true when
    /// 1. `BlockA` strictly dominates (in the traditional definition of dominance, for
    ///    single-region CFGs) `BlockB_`, OR
    /// 2. `BlockA` and `BlockB_` are the same block in a graph region.
    pub fn block_strictly_dominates_block(
        &mut self,
        ctx: &Context,
        a: Ptr<BasicBlock>,
        b: Ptr<BasicBlock>,
    ) -> bool {
        let region_a = a
            .deref(ctx)
            .get_parent_region()
            .expect("A block must be in a region");
        let region_a_ssa = region_a.deref(ctx).has_ssa_dominance(ctx);

        let Some(b) = find_ancestor_block_of_block_in_region(ctx, b, region_a) else {
            return false;
        };

        if a == b {
            return !region_a_ssa;
        }

        let dom_tree = self.get_dom_tree(ctx, region_a);
        dom_tree.dominates(&a, &b)
    }

    /// Does `a` strictly dominate `b`?
    ///
    /// Caches region dominator trees as a side effect.
    ///
    /// Defining `OpB_` as an operation immediately in `OpA`'s region, and either
    /// 1. contains `OpB` in its (possibly nested) regions, OR
    /// 2. is the same as `OpB`.
    ///
    /// This function returns true when
    /// 1. `OpA` and `OpB_` are in the same basic block of an SSA region and `OpA` strictly precedes `OpB_`, OR
    /// 2. `OpA` strictly dominates (in the traditional definition of dominance, for single-region CFGs) `OpB_`, OR
    /// 3. `OpA` and `OpB_` are in a graph region's sole basic block.
    pub fn op_strictly_dominates_op(
        &mut self,
        ctx: &Context,
        a: Ptr<Operation>,
        b: Ptr<Operation>,
    ) -> bool {
        let Some(block_a) = a.deref(ctx).get_parent_block() else {
            return false;
        };
        let region_a = block_a
            .deref(ctx)
            .get_parent_region()
            .expect("A block must be in a region");
        let region_a_ssa = region_a.deref(ctx).has_ssa_dominance(ctx);

        let Some(b) = find_ancestor_op_of_op_in_region(ctx, b, region_a) else {
            return false;
        };
        let block_b = b
            .deref(ctx)
            .get_parent_block()
            .expect("B block must be in a region");

        if block_a == block_b {
            return !region_a_ssa || strictly_precedes_in_block(ctx, a, b);
        }

        let dom_tree = self.get_dom_tree(ctx, region_a);
        dom_tree.dominates(&block_a, &block_b)
    }

    /// Does value `a` strictly dominate operation `b`?
    ///
    /// Caches region dominator trees as a side effect.
    ///
    /// See `op_strictly_dominates_op` for the definition of "strictly dominate"
    /// in the context of nested regions. Block arguments are considered to be defined
    /// at the start of their block, so they dominate all operations in their block and
    /// blocks dominated by their block.
    pub fn value_strictly_dominates_op(
        &mut self,
        ctx: &Context,
        a: Value,
        b: Ptr<Operation>,
    ) -> bool {
        match a.defining_entity() {
            DefiningEntity::Op(op) => self.op_strictly_dominates_op(ctx, op, b),
            DefiningEntity::Block(a_block) => {
                if let Some(b_parent_block) = b.deref(ctx).get_parent_block() {
                    let a_block_region = a_block
                        .deref(ctx)
                        .get_parent_region()
                        .expect("Block not in any region");
                    let b_ancestor_in_a_region =
                        find_ancestor_block_of_block_in_region(ctx, b_parent_block, a_block_region);
                    b_ancestor_in_a_region.is_some_and(|b_ancestor| {
                        let dom_tree = self.get_dom_tree(ctx, a_block_region);
                        dom_tree.dominates(&a_block, &b_ancestor)
                    })
                } else {
                    false
                }
            }
        }
    }

    /// Find the nearest common dominator of `a` and `b`, if it exists.
    ///
    /// Caches region dominator trees as a side effect.
    ///
    /// See `block_strictly_dominates_block` for the definition of "dominate" in the context of nested regions.
    pub fn nearest_common_dominator(
        &mut self,
        ctx: &Context,
        a: Ptr<BasicBlock>,
        b: Ptr<BasicBlock>,
    ) -> Option<Ptr<BasicBlock>> {
        if a == b {
            return Some(a);
        }

        let (a, b) = try_get_blocks_in_same_region(ctx, a, b)?;

        if a == b {
            return Some(a);
        }

        let region = a
            .deref(ctx)
            .get_parent_region()
            .expect("A block must be in a region");
        let dom_tree = self.get_dom_tree(ctx, region);
        Some(dom_tree.nearest_common_dominator(&a, &b))
    }
}

impl Analysis for DomInfo {
    fn name(&self) -> &'static str {
        "dom_info"
    }

    fn compute(op: Ptr<Operation>, ctx: &Context, _analyses: &mut AnalysisManager) -> Result<Self>
    where
        Self: Sized,
    {
        let mut dom_info = DomInfo::default();
        for region in op.deref(ctx).regions() {
            dom_info.get_dom_tree(ctx, region);
        }
        Ok(dom_info)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        boxed::Box,
        string::{String, ToString},
    };

    use super::*;
    use crate::{
        deps::hash::HashSet,
        graph::{ControlFlowGraph, HasLabel},
    };
    use rustc_hash::FxHashSet;

    #[derive(Clone, Debug)]
    struct Node {
        succs: Vec<usize>,
    }

    #[derive(Clone, Copy, Debug)]
    struct ArenaGraph;

    impl HasLabel<Vec<Node>> for usize {
        fn label(&self, _ctx: &Vec<Node>) -> String {
            self.to_string()
        }
    }

    impl ControlFlowGraph<Vec<Node>> for ArenaGraph {
        type Node = usize;

        fn num_successors(&self, ctx: &Vec<Node>, node: &Self::Node) -> usize {
            ctx[*node].succs.len()
        }

        fn get_successor(&self, ctx: &Vec<Node>, node: &Self::Node, i: usize) -> Self::Node {
            ctx[*node].succs[i]
        }

        fn num_predecessors(&self, ctx: &Vec<Node>, node: &Self::Node) -> usize {
            ctx.iter().filter(|n| n.succs.contains(node)).count()
        }

        fn get_predecessor(&self, ctx: &Vec<Node>, node: &Self::Node, i: usize) -> Self::Node {
            ctx.iter()
                .enumerate()
                .filter_map(|(idx, n)| n.succs.contains(node).then_some(idx))
                .nth(i)
                .expect("Predecessor index out of bounds")
        }

        fn entry_node(&self, ctx: &Vec<Node>) -> Option<Self::Node> {
            if ctx.is_empty() { None } else { Some(0) }
        }

        fn nodes<'a>(&'a self, ctx: &'a Vec<Node>) -> Box<dyn Iterator<Item = Self::Node> + 'a> {
            Box::new(0..ctx.len())
        }
    }

    fn n(succs: &[usize]) -> Node {
        Node {
            succs: succs.to_vec(),
        }
    }

    #[test]
    fn dominator_tree_empty_graph() {
        let ctx: Vec<Node> = vec![];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.root(), None);
    }

    #[test]
    fn dominator_tree_single_node() {
        let ctx = vec![n(&[])];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.root(), Some(0));
    }

    #[test]
    fn dominator_tree_linear_chain() {
        // 0 -> 1 -> 2
        let ctx = vec![
            /* 0 */ n(&[1]),
            /* 1 */ n(&[2]),
            /* 2 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.num_nodes(), 3);
        assert_eq!(dom.idom(&0), None);
        assert_eq!(dom.idom(&1), Some(0));
        assert_eq!(dom.idom(&2), Some(1));
    }

    #[test]
    fn dominator_tree_diamond() {
        //      0
        //     / \
        //    1   2
        //     \ /
        //      3
        let ctx = vec![
            /* 0 */ n(&[1, 2]),
            /* 1 */ n(&[3]),
            /* 2 */ n(&[3]),
            /* 3 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.num_nodes(), 4);
        assert_eq!(dom.idom(&0), None);
        assert_eq!(dom.idom(&1), Some(0));
        assert_eq!(dom.idom(&2), Some(0));
        assert_eq!(dom.idom(&3), Some(0));

        assert_eq!(dom.children(&0).collect::<HashSet<_>>(), [1, 2, 3].into());
        assert_eq!(dom.nearest_common_dominator(&1, &2), 0);
    }

    #[test]
    fn dominator_tree_loop() {
        //            +--------+
        //            v        |
        //  0 -> 1 (header) -> 2 (body)
        //            |
        //            v
        //            3 (exit)
        let ctx = vec![
            /* 0 */ n(&[1]),
            /* 1 */ n(&[2, 3]),
            /* 2 */ n(&[1]),
            /* 3 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.num_nodes(), 4);
        assert_eq!(dom.idom(&0), None);
        assert_eq!(dom.idom(&1), Some(0));
        assert_eq!(dom.idom(&2), Some(1));
        assert_eq!(dom.idom(&3), Some(1));

        assert!(dom.dominates(&1, &3));
        assert!(!dom.dominates(&2, &3));

        assert_eq!(dom.children(&0).collect::<HashSet<_>>(), [1].into());
        assert_eq!(dom.children(&1).collect::<HashSet<_>>(), [2, 3].into());
        assert_eq!(dom.children(&2).collect::<HashSet<_>>(), [].into());
        assert_eq!(dom.children(&3).collect::<HashSet<_>>(), [].into());

        assert_eq!(dom.nearest_common_dominator(&2, &3), 1);
        assert_eq!(dom.nearest_common_dominator(&3, &2), 1);
        assert_eq!(dom.nearest_common_dominator(&2, &1), 1);
        assert_eq!(dom.nearest_common_dominator(&1, &2), 1);
        assert_eq!(dom.nearest_common_dominator(&0, &1), 0);
        assert_eq!(dom.nearest_common_dominator(&1, &0), 0);
    }

    #[test]
    fn dominator_tree_cooper_fig4() {
        // From "A Simple, Fast Dominance Algorithm" by Cooper et al.
        //
        //         0 (entry)
        //        / \
        //       v   v
        //      1     2
        //      |    / \
        //      v   v   v
        //      3 ⇄ 4 ⇄ 5
        let ctx = vec![
            /* 0 */ n(&[1, 2]),
            /* 1 */ n(&[3]),
            /* 2 */ n(&[4, 5]),
            /* 3 */ n(&[4]),
            /* 4 */ n(&[3, 5]),
            /* 5 */ n(&[4]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.num_nodes(), 6);
        assert_eq!(dom.idom(&0), None);
        assert_eq!(dom.idom(&1), Some(0));
        assert_eq!(dom.idom(&2), Some(0));
        assert_eq!(dom.idom(&3), Some(0));
        assert_eq!(dom.idom(&4), Some(0));
        assert_eq!(dom.idom(&5), Some(0));

        assert!(dom.dominates(&0, &1));
        assert!(!dom.dominates(&2, &4));
    }

    #[test]
    fn dominator_tree_dragon() {
        // From the Dragon Book, second edition, pg 657
        let ctx = vec![
            /* 0 */ n(&[1, 2]),
            /* 1 */ n(&[2]),
            /* 2 */ n(&[3]),
            /* 3 */ n(&[4, 5, 2]),
            /* 4 */ n(&[6]),
            /* 5 */ n(&[6]),
            /* 6 */ n(&[3, 7]),
            /* 7 */ n(&[8, 9, 2]),
            /* 8 */ n(&[0]),
            /* 9 */ n(&[6]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.num_nodes(), 10);
        assert_eq!(dom.idom(&0), None);
        assert_eq!(dom.idom(&1), Some(0));
        assert_eq!(dom.idom(&2), Some(0));
        assert_eq!(dom.idom(&3), Some(2));
        assert_eq!(dom.idom(&4), Some(3));
        assert_eq!(dom.idom(&5), Some(3));
        assert_eq!(dom.idom(&6), Some(3));
        assert_eq!(dom.idom(&7), Some(6));
        assert_eq!(dom.idom(&8), Some(7));
        assert_eq!(dom.idom(&9), Some(7));

        assert_eq!(dom.children(&3).collect::<HashSet<_>>(), [4, 5, 6].into());
    }

    #[test]
    fn dominator_tree_disconnected_components() {
        // 0 -> 1    2 -> 3
        let ctx = vec![
            /* 0 */ n(&[1]),
            /* 1 */ n(&[]),
            /* 2 */ n(&[3]),
            /* 3 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        assert_eq!(dom.num_nodes(), 2);
        assert_eq!(dom.idom(&0), None);
        assert_eq!(dom.idom(&1), Some(0));

        assert_eq!(dom.children(&0).collect::<HashSet<_>>(), [1].into());
    }

    #[test]
    fn dom_frontier_single() {
        let ctx = vec![n(&[])];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        let df = DomFrontierMap::new(&ctx, &ArenaGraph, &dom);
        assert_eq!(*df.frontier(&0), FxHashSet::from_iter([]));
    }

    #[test]
    fn dom_frontier_diamond() {
        //      0
        //     / \
        //    1   2
        //     \ /
        //      3
        let ctx = vec![
            /* 0 */ n(&[1, 2]),
            /* 1 */ n(&[3]),
            /* 2 */ n(&[3]),
            /* 3 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        let df = DomFrontierMap::new(&ctx, &ArenaGraph, &dom);

        assert_eq!(*df.frontier(&0), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&1), FxHashSet::from_iter([3]));
        assert_eq!(*df.frontier(&2), FxHashSet::from_iter([3]));
        assert_eq!(*df.frontier(&3), FxHashSet::from_iter([]));
    }

    #[test]
    fn dom_frontier_diamond_unreachable() {
        //      0      4     6    7
        //     / \     |          |
        //    1   2 <- 5          8
        //     \ /
        //      3
        let ctx = vec![
            /* 0 */ n(&[1, 2]),
            /* 1 */ n(&[3]),
            /* 2 */ n(&[3]),
            /* 3 */ n(&[]),
            /* 4 */ n(&[5]),
            /* 5 */ n(&[2]),
            /* 6 */ n(&[]),
            /* 7 */ n(&[8]),
            /* 8 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        let df = DomFrontierMap::new(&ctx, &ArenaGraph, &dom);

        assert_eq!(*df.frontier(&0), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&1), FxHashSet::from_iter([3]));
        assert_eq!(*df.frontier(&2), FxHashSet::from_iter([3]));
        assert_eq!(*df.frontier(&3), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&4), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&5), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&6), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&7), FxHashSet::from_iter([]));
        assert_eq!(*df.frontier(&8), FxHashSet::from_iter([]));
    }

    #[test]
    fn dom_frontier_appel() {
        // Figure 19.5 from Andrew Appel's "Modern Compiler Implementation in ML"
        let ctx = vec![
            /* 0  */ n(&[1, 4, 8]),
            /* 1  */ n(&[2]),
            /* 2  */ n(&[2, 3]),
            /* 3  */ n(&[12]),
            /* 4  */ n(&[5, 6]),
            /* 5  */ n(&[3, 7]),
            /* 6  */ n(&[7, 11]),
            /* 7  */ n(&[4, 12]),
            /* 8  */ n(&[9, 10]),
            /* 9  */ n(&[11]),
            /* 10 */ n(&[11]),
            /* 11 */ n(&[12]),
            /* 12 */ n(&[]),
        ];
        let dom = compute_dominator_tree(&ctx, &ArenaGraph);
        let df = DomFrontierMap::new(&ctx, &ArenaGraph, &dom);
        assert_eq!(*df.frontier(&4), FxHashSet::from_iter([3, 4, 11, 12]));
    }

    // --- Operation-level dominance tests ---

    use crate::{
        basic_block::BasicBlock,
        builtin::{
            op_interfaces::{
                IsTerminatorInterface, NRegionsInterface, OneRegionInterface,
                SingleBlockRegionInterface,
            },
            ops::{FuncOp, ModuleOp},
            types::{FunctionType, IntegerType, Signedness},
        },
        context::{Context, Ptr},
        derive::pliron_op,
        linked_list::ContainsLinkedList,
        op::Op,
        operation::Operation,
    };

    /// A test-only terminator operation for setting up CFG edges.
    #[pliron_op(
        name = "test.branch",
        format,
        interfaces = [IsTerminatorInterface],
        verifier = "succ",
    )]
    struct BranchOp;

    /// A test-only operation with one region that is NOT IsolatedFromAbove.
    /// Represents something like a loop or scope construct.
    #[pliron_op(
        name = "test.scope",
        format = "`{` region($0) `}`",
        interfaces = [NRegionsInterface<1>, OneRegionInterface],
        verifier = "succ",
    )]
    struct ScopeOp;
    impl ScopeOp {
        fn new(ctx: &mut Context) -> ScopeOp {
            let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
            // Create an entry block in the region.
            let region = op.deref(ctx).get_region(0);
            let block = BasicBlock::new(ctx, None, vec![]);
            block.insert_at_front(region, ctx);
            ScopeOp { op }
        }

        fn get_entry_block(&self, ctx: &Context) -> Ptr<BasicBlock> {
            self.get_region(ctx).deref(ctx).get_head().unwrap()
        }
    }

    #[test]
    fn op_dominates_same_block() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());
        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);

        let bb = func.get_entry_block(ctx);

        let op_a = Operation::new(
            ctx,
            FuncOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_a.insert_at_back(bb, ctx);

        let op_b = Operation::new(
            ctx,
            FuncOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_b.insert_at_back(bb, ctx);

        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.op_strictly_dominates_op(ctx, op_a, op_b));
        assert!(!dom_info.op_strictly_dominates_op(ctx, op_b, op_a));
        assert!(!dom_info.op_strictly_dominates_op(ctx, op_a, op_a));
    }

    #[test]
    fn op_dominates_graph_region() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func_a = FuncOp::new(ctx, "a".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func_a.get_operation(), 0);
        let func_b = FuncOp::new(ctx, "b".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func_b.get_operation(), 0);

        let mut dom_info = super::DomInfo::default();

        let op_a = func_a.get_operation();
        let op_b = func_b.get_operation();

        assert!(dom_info.op_strictly_dominates_op(ctx, op_a, op_b));
        assert!(dom_info.op_strictly_dominates_op(ctx, op_b, op_a));
    }

    #[test]
    fn ssa_op_does_not_dominate_own_body() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let outer_func = FuncOp::new(ctx, "outer".try_into().unwrap(), func_ty);
        module.append_operation(ctx, outer_func.get_operation(), 0);

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        func.get_operation()
            .insert_at_back(outer_func.get_entry_block(ctx), ctx);

        let inner_op = Operation::new(
            ctx,
            FuncOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        inner_op.insert_at_back(func.get_entry_block(ctx), ctx);

        let mut dom_info = super::DomInfo::default();

        assert!(!dom_info.op_strictly_dominates_op(ctx, func.get_operation(), inner_op));
    }

    #[test]
    fn graph_op_dominates_own_body() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);

        let inner_op = Operation::new(
            ctx,
            FuncOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        inner_op.insert_at_back(func.get_entry_block(ctx), ctx);

        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.op_strictly_dominates_op(ctx, func.get_operation(), inner_op));
    }

    #[test]
    fn graph_op_dominates_self() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);

        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.op_strictly_dominates_op(ctx, func.get_operation(), func.get_operation()));
    }

    #[test]
    fn block_does_not_dominate_self_in_ssa_region() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);

        let mut dom_info = super::DomInfo::default();
        let entry = func.get_entry_block(ctx);

        assert!(!dom_info.block_strictly_dominates_block(ctx, entry, entry));
    }

    #[test]
    fn block_dominates_self_in_graph_region() {
        let ctx = &mut Context::new();
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());
        let module_entry = module.get_region(ctx).deref(ctx).get_head().unwrap();

        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.block_strictly_dominates_block(ctx, module_entry, module_entry));
    }

    #[test]
    fn op_dominates_nested_region() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);
        let bb = func.get_entry_block(ctx);

        let op_a = Operation::new(
            ctx,
            FuncOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_a.insert_at_back(bb, ctx);

        let scope = ScopeOp::new(ctx);
        scope.get_operation().insert_at_back(bb, ctx);

        let op_b = Operation::new(
            ctx,
            FuncOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_b.insert_at_back(scope.get_entry_block(ctx), ctx);

        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.op_strictly_dominates_op(ctx, op_a, scope.get_operation()));
        assert!(dom_info.op_strictly_dominates_op(ctx, op_a, op_b));
    }

    #[test]
    fn op_dominates_cross_block() {
        // CFG:
        //     entry (opA)
        //      / \
        //    b1    b2
        //   (opB)  (opC)
        //
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);
        let func_region = func.get_region(ctx);
        let entry = func.get_entry_block(ctx);

        let b1 = BasicBlock::new(ctx, None, vec![]);
        b1.insert_at_back(func_region, ctx);
        let b2 = BasicBlock::new(ctx, None, vec![]);
        b2.insert_at_back(func_region, ctx);

        let op_a = Operation::new(
            ctx,
            ScopeOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_a.insert_at_back(entry, ctx);
        let branch = Operation::new(
            ctx,
            BranchOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![b1, b2],
            0,
        );
        branch.insert_at_back(entry, ctx);

        let op_b = Operation::new(
            ctx,
            ScopeOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_b.insert_at_back(b1, ctx);

        let op_c = Operation::new(
            ctx,
            ScopeOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_c.insert_at_back(b2, ctx);

        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.op_strictly_dominates_op(ctx, op_a, op_b));
        assert!(dom_info.op_strictly_dominates_op(ctx, op_a, op_c));
        assert!(!dom_info.op_strictly_dominates_op(ctx, op_b, op_c));

        assert!(dom_info.block_strictly_dominates_block(ctx, entry, b1));
        assert!(dom_info.block_strictly_dominates_block(ctx, entry, b2));
        assert!(!dom_info.block_strictly_dominates_block(ctx, b1, b2));
        assert!(!dom_info.block_strictly_dominates_block(ctx, b1, b1));

        assert_eq!(dom_info.nearest_common_dominator(ctx, b1, b2), Some(entry));
        assert_eq!(
            dom_info.nearest_common_dominator(ctx, entry, b1),
            Some(entry)
        );
    }

    #[test]
    fn value_op_result_dominates_op() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);
        let bb = func.get_entry_block(ctx);

        let def_op = Operation::new(
            ctx,
            ScopeOp::get_concrete_op_info(),
            vec![i64_ty.into()],
            vec![],
            vec![],
            0,
        );
        def_op.insert_at_back(bb, ctx);

        let use_op = Operation::new(
            ctx,
            ScopeOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        use_op.insert_at_back(bb, ctx);

        let val = def_op.deref(ctx).get_result(0);
        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.value_strictly_dominates_op(ctx, val, use_op));
        assert!(!dom_info.value_strictly_dominates_op(ctx, val, def_op));
    }

    #[test]
    fn value_block_argument_dominates_ops() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);
        let func_region = func.get_region(ctx);
        let entry = func.get_entry_block(ctx);

        let arg_block = BasicBlock::new(ctx, None, vec![i64_ty.into()]);
        arg_block.insert_at_back(func_region, ctx);

        let branch = Operation::new(
            ctx,
            BranchOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![arg_block],
            0,
        );
        branch.insert_at_back(entry, ctx);

        let op_in_arg_block = Operation::new(
            ctx,
            ScopeOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![],
            0,
        );
        op_in_arg_block.insert_at_back(arg_block, ctx);

        let val = arg_block.deref(ctx).get_argument(0);
        let mut dom_info = super::DomInfo::default();

        assert!(dom_info.value_strictly_dominates_op(ctx, val, op_in_arg_block));
        assert!(!dom_info.value_strictly_dominates_op(ctx, val, branch));
    }

    #[test]
    fn nearest_common_dominator_nested_regions() {
        let ctx = &mut Context::new();
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
        let module = ModuleOp::new(ctx, "test_mod".try_into().unwrap());

        let func = FuncOp::new(ctx, "f".try_into().unwrap(), func_ty);
        module.append_operation(ctx, func.get_operation(), 0);
        let entry = func.get_entry_block(ctx);

        let scope1 = ScopeOp::new(ctx);
        scope1.get_operation().insert_at_back(entry, ctx);
        let scope2 = ScopeOp::new(ctx);
        scope2.get_operation().insert_at_back(entry, ctx);

        let inner1 = scope1.get_entry_block(ctx);
        let inner2 = scope2.get_entry_block(ctx);

        let mut dom_info = super::DomInfo::default();

        assert_eq!(
            dom_info.nearest_common_dominator(ctx, inner1, inner2),
            Some(entry)
        );
    }

    #[test]
    fn nearest_common_dominator_different_modules_none() {
        let ctx = &mut Context::new();
        let module1 = ModuleOp::new(ctx, "mod1".try_into().unwrap());
        let module2 = ModuleOp::new(ctx, "mod2".try_into().unwrap());

        let b1 = module1.get_region(ctx).deref(ctx).get_head().unwrap();
        let b2 = module2.get_region(ctx).deref(ctx).get_head().unwrap();

        let mut dom_info = super::DomInfo::default();

        assert_eq!(dom_info.nearest_common_dominator(ctx, b1, b2), None);
    }
}
