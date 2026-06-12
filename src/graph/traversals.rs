//! Control-flow-graph traversals

use super::ControlFlowGraph;
use rustc_hash::{FxHashMap, FxHashSet};

/// Region traversal utilities
pub mod region {
    use alloc::{vec, vec::Vec};

    use crate::graph::HasLabel;

    use super::*;

    fn post_order_walk_component<G, GraphContext>(
        ctx: &GraphContext,
        graph: &G,
        node: G::Node,
        seen_nodes: &mut FxHashSet<G::Node>,
        po: &mut Vec<G::Node>,
    ) where
        G: ControlFlowGraph<GraphContext>,
    {
        if seen_nodes.contains(&node) {
            // node already visited.
            return;
        }

        let mut stack = Vec::<(G::Node, usize)>::new();

        // Start from `node`.
        seen_nodes.insert(node.clone());
        // The second element of the pair is the index of the next successor to visit for this node.
        stack.push((node, 0));
        while let Some((node, succ_idx)) = stack.pop() {
            if succ_idx < graph.num_successors(ctx, &node) {
                // Push the current node back to stack with next successor index.
                stack.push((node.clone(), succ_idx + 1));

                let succ = graph.get_successor(ctx, &node, succ_idx);
                if seen_nodes.contains(&succ) {
                    // successor already visited.
                    continue;
                }

                // Visit this successor next.
                seen_nodes.insert(succ.clone());
                stack.push((succ, 0));
            } else {
                // All successors of this node have been visited. We can visit this node now.
                po.push(node);
            }
        }
    }

    /// Compute post-order of the nodes in a graph.
    /// *Note*: Assumes that for each disconnected component the entry node appears
    /// before all other nodes in the component in the input.
    pub fn post_order<G, GraphContext>(ctx: &GraphContext, graph: &G) -> Vec<G::Node>
    where
        G: ControlFlowGraph<GraphContext>,
    {
        post_order_by_component(ctx, graph)
            .into_iter()
            .flatten()
            .collect::<Vec<G::Node>>()
    }

    /// Compute the post-order of the nodes in a graph,
    /// providing result for each connected component separately.
    /// *Note*: Assumes that for each disconnected component the entry node appears
    /// before all other nodes in the component in the input.
    pub fn post_order_by_component<G, GraphContext>(
        ctx: &GraphContext,
        graph: &G,
    ) -> Vec<Vec<G::Node>>
    where
        G: ControlFlowGraph<GraphContext>,
    {
        let seen_nodes = &mut FxHashSet::<G::Node>::default();
        let mut po_by_component = Vec::<Vec<G::Node>>::new();

        // Walk every node (not just entry) since we may have unreachable nodes.
        for node in graph.nodes(ctx) {
            let mut po = Vec::<G::Node>::new();
            post_order_walk_component(ctx, graph, node, seen_nodes, &mut po);
            if !po.is_empty() {
                po_by_component.push(po);
            }
        }
        po_by_component
    }

    /// Compute reverse-post-order of the nodes in a graph.
    /// *Note*: Assumes that for each disconnected component the entry node appears
    /// before all other nodes in the component in the input.
    pub fn topological_order<G, GraphContext>(ctx: &GraphContext, graph: &G) -> Vec<G::Node>
    where
        G: ControlFlowGraph<GraphContext>,
    {
        topological_order_by_component(ctx, graph)
            .into_iter()
            .flatten()
            .collect::<Vec<G::Node>>()
    }

    /// Compute the reverse-post-order of the nodes in a graph,
    /// providing result for each connected component separately.
    /// *Note*:
    ///   1. Component wise order remains the same as the input.
    ///   2. Each component must have its entry node as the first node.
    pub fn topological_order_by_component<G, GraphContext>(
        ctx: &GraphContext,
        graph: &G,
    ) -> Vec<Vec<G::Node>>
    where
        G: ControlFlowGraph<GraphContext>,
    {
        let seen_nodes = &mut FxHashSet::<G::Node>::default();
        let mut rpo_by_component = Vec::<Vec<G::Node>>::new();

        // Walk every node (not just entry) since we may have unreachable nodes.
        for node in graph.nodes(ctx) {
            let mut po = Vec::<G::Node>::new();
            post_order_walk_component(ctx, graph, node, seen_nodes, &mut po);
            if !po.is_empty() {
                rpo_by_component.push(po.into_iter().rev().collect());
            }
        }
        rpo_by_component
    }

    /// The pre-order, post-order and rpo numbers of a node in a DFS traversal.
    struct DFSNumber {
        pre_order_number: usize,
        post_order_number: usize,
        reverse_post_order_number: usize,
    }

    /// Edge kind in a DFS traversal.
    /// Conventionally DFS traversal classifies edges into tree, back, forward and cross edges.
    /// Since we don't explicitly maintain the DFS tree, we can't distinguish between
    /// tree and forward edges. So we classify them both as forward edges.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum DFSEdgeKind {
        Forward,
        Back,
        Cross,
    }

    /// Performs a DFS traversal of the graph reachable from the entry node.
    /// Answers queries related to the traversal.
    pub struct DFSTraversal<G, GraphContext>
    where
        G: ControlFlowGraph<GraphContext>,
    {
        tree: FxHashMap<G::Node, DFSNumber>,
    }

    fn dfs_walk<G, GraphContext>(
        ctx: &GraphContext,
        graph: &G,
        node: G::Node,
        pre_order_counter: &mut usize,
        post_order_counter: &mut usize,
        result: &mut FxHashMap<G::Node, DFSNumber>,
    ) where
        G: ControlFlowGraph<GraphContext>,
    {
        let mut stack = Vec::<(G::Node, usize)>::new();

        // Assign pre-order number to the entry node and start DFS from it.
        let pre_order_number = *pre_order_counter;
        *pre_order_counter += 1;
        let newly_inserted = result.insert(
            node.clone(),
            DFSNumber {
                pre_order_number,
                post_order_number: 0,         // to be filled later
                reverse_post_order_number: 0, // to be filled later
            },
        );
        assert!(
            newly_inserted.is_none(),
            "Node {} visited multiple times during DFS traversal",
            node.label(ctx)
        );
        stack.push((node, 0));

        while let Some((node, succ_idx)) = stack.pop() {
            if succ_idx < graph.num_successors(ctx, &node) {
                // Re-visit this node after attempting this successor.
                stack.push((node.clone(), succ_idx + 1));

                let succ = graph.get_successor(ctx, &node, succ_idx);
                if result.contains_key(&succ) {
                    // successor already visited.
                    continue;
                }

                // First time we discover this successor: assign pre-order and descend.
                let pre_order_number = *pre_order_counter;
                *pre_order_counter += 1;
                let newly_inserted = result.insert(
                    succ.clone(),
                    DFSNumber {
                        pre_order_number,
                        post_order_number: 0,         // to be filled later
                        reverse_post_order_number: 0, // to be filled later
                    },
                );
                assert!(
                    newly_inserted.is_none(),
                    "Node {} visited multiple times during DFS traversal",
                    succ.label(ctx)
                );
                stack.push((succ, 0));
            } else {
                // All successors are processed, so we can assign post-order now.
                let post_order_number = *post_order_counter;
                *post_order_counter += 1;
                result
                    .get_mut(&node)
                    .expect("Node missing during DFS traversal")
                    .post_order_number = post_order_number;
            }
        }
    }

    impl<G, GraphContext> DFSTraversal<G, GraphContext>
    where
        G: ControlFlowGraph<GraphContext>,
    {
        /// Perform DFS traversal of the graph and compute pre-order,
        /// post-order and reverse-post-order numbers for each node.
        pub fn new(ctx: &GraphContext, graph: &G) -> Self {
            let mut result = FxHashMap::<G::Node, DFSNumber>::default();
            let mut pre_order_counter = 0;
            let mut post_order_counter = 0;

            if let Some(entry) = graph.entry_node(ctx) {
                dfs_walk(
                    ctx,
                    graph,
                    entry,
                    &mut pre_order_counter,
                    &mut post_order_counter,
                    &mut result,
                );
            }

            let num_nodes = result.len();
            for dfs_number in result.values_mut() {
                dfs_number.reverse_post_order_number = num_nodes - 1 - dfs_number.post_order_number;
            }
            Self { tree: result }
        }

        /// Returns the pre-order number of a node.
        pub fn pre_order_number(&self, node: &G::Node) -> usize {
            self.tree
                .get(node)
                .expect("pre-order number requested for unreachable node in graph")
                .pre_order_number
        }

        /// Returns the post-order number of a node.
        pub fn post_order_number(&self, node: &G::Node) -> usize {
            self.tree
                .get(node)
                .expect("post-order number requested for unreachable node in graph")
                .post_order_number
        }

        /// Returns the reverse-post-order number of a node.
        pub fn reverse_post_order_number(&self, node: &G::Node) -> usize {
            self.tree
                .get(node)
                .expect("reverse-post-order number requested for unreachable node in graph")
                .reverse_post_order_number
        }

        /// Returns the kind of edge (tree, back, forward or cross) between two nodes.
        pub fn edge_kind(&self, from: &G::Node, to: &G::Node) -> DFSEdgeKind {
            let from_pre = self.pre_order_number(from);
            let from_post = self.post_order_number(from);
            let to_pre = self.pre_order_number(to);
            let to_post = self.post_order_number(to);

            // If the pre-order numbers are the same, the post-order
            // numbers must also be the same, and vice versa.
            assert!((from_pre == to_pre) == (from_post == to_post));
            if from_pre == to_pre {
                // Self-loop edge; classify as back edge.
                return DFSEdgeKind::Back;
            }

            if from_pre < to_pre {
                assert!(
                    from_post > to_post,
                    "Can't pop a node before finishing its subtree in DFS traversal"
                );
                // Descendant edge in DFS forest: tree or forward.
                DFSEdgeKind::Forward
            } else {
                if from_post < to_post {
                    // Edge to an ancestor in DFS tree.
                    DFSEdgeKind::Back
                } else {
                    DFSEdgeKind::Cross
                }
            }
        }

        /// Get all nodes reachable from entry in post-order.
        pub fn post_order(&self) -> impl Iterator<Item = G::Node> {
            let mut nodes = vec![None; self.tree.len()];
            for (node, dfs_number) in &self.tree {
                nodes[dfs_number.post_order_number] = Some(node.clone());
            }
            nodes
                .into_iter()
                .map(|n| n.expect("Node missing in post-order"))
        }

        /// Get all nodes reachable from entry in reverse post-order.
        pub fn reverse_post_order(&self) -> impl Iterator<Item = G::Node> {
            let mut nodes = vec![None; self.tree.len()];
            for (node, dfs_number) in &self.tree {
                nodes[dfs_number.reverse_post_order_number] = Some(node.clone());
            }
            nodes
                .into_iter()
                .map(|n| n.expect("Node missing in reverse post-order"))
        }

        /// Get all nodes reachable from entry in pre-order.
        pub fn pre_order(&self) -> impl Iterator<Item = G::Node> {
            let mut nodes = vec![None; self.tree.len()];
            for (node, dfs_number) in &self.tree {
                nodes[dfs_number.pre_order_number] = Some(node.clone());
            }
            nodes
                .into_iter()
                .map(|n| n.expect("Node missing in pre-order"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::region::{DFSEdgeKind, DFSTraversal, post_order, topological_order};
    use crate::graph::{ControlFlowGraph, HasLabel};
    use alloc::{
        boxed::Box,
        string::{String, ToString},
        vec,
        vec::Vec,
    };

    #[derive(Clone, Debug)]
    struct Node {
        data: u32,
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

        fn entry_node(&self, ctx: &Vec<Node>) -> Option<Self::Node> {
            (!ctx.is_empty()).then_some(0)
        }

        fn nodes<'a>(&'a self, ctx: &'a Vec<Node>) -> Box<dyn Iterator<Item = Self::Node> + 'a> {
            Box::new(0..ctx.len())
        }

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
                .expect("Node doesn't have that many predecessors")
        }
    }

    fn n(data: u32, succs: &[usize]) -> Node {
        Node {
            data,
            succs: succs.to_vec(),
        }
    }

    fn data_order(ctx: &[Node], order: &[usize]) -> Vec<u32> {
        order.iter().map(|&idx| ctx[idx].data).collect()
    }

    fn assert_is_permutation_of_all_nodes(ctx: &[Node], order: &[usize]) {
        assert_eq!(order.len(), ctx.len());
        let mut seen = vec![false; ctx.len()];
        for &idx in order {
            assert!(idx < ctx.len(), "index {idx} out of bounds");
            assert!(!seen[idx], "duplicate node index {idx}");
            seen[idx] = true;
        }
        assert!(seen.into_iter().all(|v| v));
    }

    fn assert_post_order_edge_property_for_dag(ctx: &[Node], po: &[usize]) {
        let mut pos = vec![usize::MAX; ctx.len()];
        for (i, &idx) in po.iter().enumerate() {
            pos[idx] = i;
        }
        for (u, node) in ctx.iter().enumerate() {
            for &v in &node.succs {
                assert!(
                    pos[v] < pos[u],
                    "expected successor {v} to appear before predecessor {u} in post-order"
                );
            }
        }
    }

    fn assert_topological_edge_property_for_dag(ctx: &[Node], rpo: &[usize]) {
        let mut pos = vec![usize::MAX; ctx.len()];
        for (i, &idx) in rpo.iter().enumerate() {
            pos[idx] = i;
        }
        for (u, node) in ctx.iter().enumerate() {
            for &v in &node.succs {
                assert!(
                    pos[u] < pos[v],
                    "expected predecessor {u} to appear before successor {v} in reverse post-order"
                );
            }
        }
    }

    #[test]
    fn post_order_linear_chain() {
        let ctx = vec![n(10, &[1]), n(20, &[2]), n(30, &[])];

        let po = post_order(&ctx, &ArenaGraph);
        let rpo = topological_order(&ctx, &ArenaGraph);

        assert_eq!(data_order(&ctx, &po), vec![30, 20, 10]);
        assert_eq!(data_order(&ctx, &rpo), vec![10, 20, 30]);
        assert_is_permutation_of_all_nodes(&ctx, &po);
        assert_post_order_edge_property_for_dag(&ctx, &po);
        assert_topological_edge_property_for_dag(&ctx, &rpo);
    }

    #[test]
    fn post_order_diamond_dag() {
        let ctx = vec![n(0, &[1, 2]), n(1, &[3]), n(2, &[3]), n(3, &[])];

        let po = post_order(&ctx, &ArenaGraph);
        let rpo = topological_order(&ctx, &ArenaGraph);

        assert_eq!(data_order(&ctx, &po), vec![3, 1, 2, 0]);
        assert_eq!(data_order(&ctx, &rpo), vec![0, 2, 1, 3]);
        assert_is_permutation_of_all_nodes(&ctx, &po);
        assert_post_order_edge_property_for_dag(&ctx, &po);
        assert_topological_edge_property_for_dag(&ctx, &rpo);
    }

    #[test]
    fn post_order_handles_cycle_and_isolated_node() {
        let ctx = vec![n(0, &[1]), n(1, &[2]), n(2, &[0]), n(99, &[])];

        let po = post_order(&ctx, &ArenaGraph);
        let rpo = topological_order(&ctx, &ArenaGraph);

        assert_eq!(data_order(&ctx, &po), vec![2, 1, 0, 99]);
        assert_eq!(data_order(&ctx, &rpo), vec![0, 1, 2, 99]);
        assert_is_permutation_of_all_nodes(&ctx, &po);
    }

    #[test]
    fn post_order_covers_disconnected_components() {
        let ctx = vec![n(0, &[1]), n(1, &[]), n(2, &[3]), n(3, &[]), n(4, &[])];

        let po = post_order(&ctx, &ArenaGraph);
        let rpo = topological_order(&ctx, &ArenaGraph);

        assert_eq!(data_order(&ctx, &po), vec![1, 0, 3, 2, 4]);
        assert_eq!(data_order(&ctx, &rpo), vec![0, 1, 2, 3, 4]);
        assert_is_permutation_of_all_nodes(&ctx, &po);
        assert_post_order_edge_property_for_dag(&ctx, &po);
        assert_topological_edge_property_for_dag(&ctx, &rpo);
    }

    #[test]
    fn dfs_edge_kind_classification() {
        let ctx = vec![n(0, &[1, 2]), n(1, &[3]), n(2, &[3]), n(3, &[0, 3])];
        let dfs = DFSTraversal::<ArenaGraph, Vec<Node>>::new(&ctx, &ArenaGraph);

        assert_eq!(dfs.edge_kind(&0, &1), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&1, &3), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&0, &2), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&3, &0), DFSEdgeKind::Back);
        assert_eq!(dfs.edge_kind(&3, &3), DFSEdgeKind::Back);
        assert_eq!(dfs.edge_kind(&2, &3), DFSEdgeKind::Cross);
    }

    #[test]
    fn dfs_edge_kind_classification_bob_morgan_fig31() {
        // This graph is from Figure 3.1 in Bob Morgan's "Building an Optimizing Compiler".
        let ctx = vec![
            n(0, &[1, 5]),
            n(1, &[2, 4]),
            n(2, &[3, 6]),
            n(3, &[4, 2]),
            n(4, &[5, 1]),
            n(5, &[]),
            n(6, &[3]),
        ];
        let dfs = DFSTraversal::<ArenaGraph, Vec<Node>>::new(&ctx, &ArenaGraph);

        // Table 3.1 in Bob Morgan's "Building an Optimizing Compiler" classifies edges in this graph as follows:
        // Tree edges: 0->1, 1->2, 2->3, 2->6, 3->4, 4->5
        // Forward edges: 0->5, 1->4
        assert_eq!(dfs.edge_kind(&0, &1), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&1, &2), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&2, &3), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&2, &6), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&3, &4), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&4, &5), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&0, &5), DFSEdgeKind::Forward);
        assert_eq!(dfs.edge_kind(&1, &4), DFSEdgeKind::Forward);

        // Back edges: 3->2, 4->1
        assert_eq!(dfs.edge_kind(&3, &2), DFSEdgeKind::Back);
        assert_eq!(dfs.edge_kind(&4, &1), DFSEdgeKind::Back);

        // Cross edges: 6->3
        assert_eq!(dfs.edge_kind(&6, &3), DFSEdgeKind::Cross);

        // Test pre-order, post-order and reverse-post-order numbers for a few nodes.
        assert_eq!(dfs.pre_order_number(&0), 0);
        assert_eq!(dfs.pre_order_number(&1), 1);
        assert_eq!(dfs.pre_order_number(&2), 2);
        assert_eq!(dfs.pre_order_number(&3), 3);
        assert_eq!(dfs.pre_order_number(&4), 4);
        assert_eq!(dfs.pre_order_number(&5), 5);
        assert_eq!(dfs.pre_order_number(&6), 6);

        assert_eq!(dfs.post_order_number(&5), 0);
        assert_eq!(dfs.post_order_number(&4), 1);
        assert_eq!(dfs.post_order_number(&3), 2);
        assert_eq!(dfs.post_order_number(&6), 3);
        assert_eq!(dfs.post_order_number(&2), 4);
        assert_eq!(dfs.post_order_number(&1), 5);
        assert_eq!(dfs.post_order_number(&0), 6);

        assert_eq!(dfs.reverse_post_order_number(&0), 0);
        assert_eq!(dfs.reverse_post_order_number(&1), 1);
        assert_eq!(dfs.reverse_post_order_number(&2), 2);
        assert_eq!(dfs.reverse_post_order_number(&6), 3);
        assert_eq!(dfs.reverse_post_order_number(&3), 4);
        assert_eq!(dfs.reverse_post_order_number(&4), 5);
        assert_eq!(dfs.reverse_post_order_number(&5), 6);
    }
}
