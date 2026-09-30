// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! The reverse of a control-flow-graph.
//!
//! A control-flow-graph may have multiple exit nodes, or nodes that do
//! not have a path to any exit node. Since it is convenient to have a single
//! unified exit node, we introduce a virtual node, called the **sentinel**.
//! The sentinel serves as a dedicated "entry" for the reverse graph.
//!
//! We also define **pre-sentinels** to be the following nodes:
//! 1. Every exit node of the control-flow-graph (a node that has no CFG successors).
//! 2. One node from each strongly connected component that cannot reach an exit node
//!    and has no edges to nodes outside the component (typically an infinite loop).
//!
//! Pre-sentinels are assumed to have the sentinel as their virtual CFG successor.

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::{
    graph::{ControlFlowGraph, HasLabel, traversals},
    utils::table::{HMap, ISet},
};

/// Finds the pre-sentinels of `graph`.
//
// Strategy:
// - Definition: A sink SCC is a strongly connected component that has no edge to another SCC.
// - Every node reaches at least one sink SCC.
// - A sink SCC is either a single exit node, or a set of nodes that cannot reach an exit node
//   (an infinite loop, for example).
// - So, one node from each sink SCC is picked as a pre-sentinel.
fn find_pre_sentinels<G, GraphContext>(ctx: &GraphContext, graph: &G) -> Vec<G::Node>
where
    G: ControlFlowGraph<GraphContext>,
{
    let sccs = traversals::region::sccs_in_topological_order(ctx, graph);
    // Map each node to the index of its SCC.
    let scc_of: HMap<G::Node, usize> = sccs
        .iter()
        .enumerate()
        .flat_map(|(i, scc)| scc.nodes.iter().map(move |node| (node.clone(), i)))
        .collect();
    // An SCC is a sink SCC if all successors of its nodes are within the SCC.
    let is_sink_scc: Vec<bool> = sccs
        .iter()
        .enumerate()
        .map(|(i, scc)| {
            scc.nodes.iter().all(|node| {
                graph
                    .successors(ctx, node)
                    .iter()
                    .all(|succ| scc_of[succ] == i)
            })
        })
        .collect();

    // Select the last node (in graph order) from each sink SCC. This makes it
    //   - Independent of successor order.
    //   - Different from LLVM's "furthest away" (last node in a forward DFS) strategy.
    // The latter may result in a different (but still correct) choice in infinite loops.
    let mut selected: Vec<Option<G::Node>> = vec![None; sccs.len()];
    for node in graph.nodes(ctx) {
        let scc = scc_of[&node];
        if is_sink_scc[scc] {
            selected[scc] = Some(node);
        }
    }
    selected.into_iter().flatten().collect()
}

/// A node of [ReverseGraph]: either the sentinel or a node of the original graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReverseNode<N> {
    /// The virtual unified exit of the original graph
    /// (the entry of the reverse graph).
    Sentinel,
    /// A node of the original graph.
    Real(N),
}

impl<N> ReverseNode<N> {
    /// Is this the sentinel?
    pub fn is_sentinel(&self) -> bool {
        matches!(self, ReverseNode::Sentinel)
    }

    /// Returns a reference to the node of the original graph,
    /// or `None` for the sentinel.
    pub fn as_real(&self) -> Option<&N> {
        match self {
            ReverseNode::Sentinel => None,
            ReverseNode::Real(node) => Some(node),
        }
    }

    /// Returns the node of the original graph, or `None` for the sentinel.
    pub fn into_real(self) -> Option<N> {
        match self {
            ReverseNode::Sentinel => None,
            ReverseNode::Real(node) => Some(node),
        }
    }
}

impl<N: HasLabel<GraphContext>, GraphContext> HasLabel<GraphContext> for ReverseNode<N> {
    fn label(&self, ctx: &GraphContext) -> String {
        match self {
            ReverseNode::Sentinel => "sentinel".to_string(),
            ReverseNode::Real(n) => n.label(ctx),
        }
    }
}

/// The reverse of a graph.
/// The sentinel forms the entry, with an edge to each pre-sentinel.
///
/// The original graph must not be modified during the lifetime of this reverse graph.
pub struct ReverseGraph<'a, G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// The original graph that is reversed.
    graph: &'a G,
    /// The pre-sentinels of [Self::graph].
    pre_sentinels: ISet<G::Node>,
    /// Precomputed predecessors of each node in [Self::graph].
    predecessors: HMap<G::Node, Vec<G::Node>>,
}

impl<'a, G, GraphContext> ReverseGraph<'a, G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// Build the reverse of `graph`.
    pub fn new(ctx: &GraphContext, graph: &'a G) -> Self {
        let pre_sentinels = find_pre_sentinels(ctx, graph).into_iter().collect();
        let predecessors = graph
            .nodes(ctx)
            .map(|node| {
                let preds = graph.predecessors(ctx, &node);
                (node, preds)
            })
            .collect();
        ReverseGraph {
            graph,
            pre_sentinels,
            predecessors,
        }
    }

    /// Get the original graph.
    pub fn original(&self) -> &'a G {
        self.graph
    }

    /// Get an iterator over the pre-sentinels.
    pub fn pre_sentinels(&self) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.pre_sentinels.iter().cloned()
    }
}

impl<G, GraphContext> ControlFlowGraph<GraphContext> for ReverseGraph<'_, G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    type Node = ReverseNode<G::Node>;

    fn num_successors(&self, _ctx: &GraphContext, node: &Self::Node) -> usize {
        match node {
            ReverseNode::Sentinel => self.pre_sentinels.len(),
            ReverseNode::Real(n) => self.predecessors[n].len(),
        }
    }

    fn get_successor(&self, _ctx: &GraphContext, node: &Self::Node, i: usize) -> Self::Node {
        match node {
            ReverseNode::Sentinel => ReverseNode::Real(
                self.pre_sentinels
                    .get_index(i)
                    .expect("Pre-sentinel index out of bounds")
                    .clone(),
            ),
            ReverseNode::Real(n) => ReverseNode::Real(self.predecessors[n][i].clone()),
        }
    }

    fn num_predecessors(&self, ctx: &GraphContext, node: &Self::Node) -> usize {
        match node {
            ReverseNode::Sentinel => 0,
            ReverseNode::Real(n) => {
                self.graph.num_successors(ctx, n) + usize::from(self.pre_sentinels.contains(n))
            }
        }
    }

    fn get_predecessor(&self, ctx: &GraphContext, node: &Self::Node, i: usize) -> Self::Node {
        let ReverseNode::Real(n) = node else {
            panic!("The sentinel has no predecessors");
        };
        if i < self.graph.num_successors(ctx, n) {
            ReverseNode::Real(self.graph.get_successor(ctx, n, i))
        } else {
            assert!(self.pre_sentinels.contains(n) && i == self.graph.num_successors(ctx, n));
            ReverseNode::Sentinel
        }
    }

    fn entry_node(&self, _ctx: &GraphContext) -> Option<Self::Node> {
        Some(ReverseNode::Sentinel)
    }

    fn nodes<'a>(&'a self, ctx: &'a GraphContext) -> Box<dyn Iterator<Item = Self::Node> + 'a> {
        Box::new(
            core::iter::once(ReverseNode::Sentinel)
                .chain(self.graph.nodes(ctx).map(ReverseNode::Real)),
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        boxed::Box,
        string::{String, ToString},
        vec,
        vec::Vec,
    };

    use expect_test::expect;

    use super::*;
    use crate::{graph::print_cfg, printable::State};

    struct ArenaGraph;

    impl HasLabel<Vec<Vec<usize>>> for usize {
        fn label(&self, _ctx: &Vec<Vec<usize>>) -> String {
            self.to_string()
        }
    }

    impl ControlFlowGraph<Vec<Vec<usize>>> for ArenaGraph {
        type Node = usize;

        fn num_successors(&self, ctx: &Vec<Vec<usize>>, node: &Self::Node) -> usize {
            ctx[*node].len()
        }

        fn get_successor(&self, ctx: &Vec<Vec<usize>>, node: &Self::Node, i: usize) -> Self::Node {
            ctx[*node][i]
        }

        fn num_predecessors(&self, ctx: &Vec<Vec<usize>>, node: &Self::Node) -> usize {
            ctx.iter().filter(|succs| succs.contains(node)).count()
        }

        fn get_predecessor(
            &self,
            ctx: &Vec<Vec<usize>>,
            node: &Self::Node,
            i: usize,
        ) -> Self::Node {
            ctx.iter()
                .enumerate()
                .filter_map(|(idx, succs)| succs.contains(node).then_some(idx))
                .nth(i)
                .expect("Predecessor index out of bounds")
        }

        fn entry_node(&self, ctx: &Vec<Vec<usize>>) -> Option<Self::Node> {
            if ctx.is_empty() { None } else { Some(0) }
        }

        fn nodes<'a>(
            &'a self,
            ctx: &'a Vec<Vec<usize>>,
        ) -> Box<dyn Iterator<Item = Self::Node> + 'a> {
            Box::new(0..ctx.len())
        }
    }

    fn print(ctx: &Vec<Vec<usize>>, graph: &impl ControlFlowGraph<Vec<Vec<usize>>>) -> String {
        let mut output = String::new();
        print_cfg(ctx, graph, &State::default(), &mut output).unwrap();
        output
    }

    #[test]
    fn reverse_graph_empty() {
        let ctx: Vec<Vec<usize>> = vec![];
        let reverse = ReverseGraph::new(&ctx, &ArenaGraph);
        expect![[r#"
            digraph cfg {
              n0 [label="sentinel"];
            }"#]]
        .assert_eq(&print(&ctx, &reverse));
    }

    #[test]
    fn reverse_graph_exit_and_infinite_loop() {
        // 0 -> 1 -> 3 (exit)
        // 0 -> 2 <-> 4 (infinite loop)
        // 5 -> 3 (5 is unreachable from the entry)
        let ctx = vec![
            /* 0 */ vec![1, 2],
            /* 1 */ vec![3],
            /* 2 */ vec![4],
            /* 3 */ vec![],
            /* 4 */ vec![2],
            /* 5 */ vec![3],
        ];
        expect![[r#"
            digraph cfg {
              n0 [label="0"];
              n1 [label="1"];
              n2 [label="2"];
              n3 [label="3"];
              n4 [label="4"];
              n5 [label="5"];
              n0 -> n1;
              n0 -> n2;
              n1 -> n3;
              n2 -> n4;
              n4 -> n2;
              n5 -> n3;
            }"#]]
        .assert_eq(&print(&ctx, &ArenaGraph));

        let reverse = ReverseGraph::new(&ctx, &ArenaGraph);
        expect![[r#"
            digraph cfg {
              n0 [label="sentinel"];
              n1 [label="0"];
              n2 [label="1"];
              n3 [label="2"];
              n4 [label="3"];
              n5 [label="4"];
              n6 [label="5"];
              n0 -> n5;
              n0 -> n4;
              n2 -> n1;
              n3 -> n1;
              n3 -> n5;
              n4 -> n2;
              n4 -> n6;
              n5 -> n3;
            }"#]]
        .assert_eq(&print(&ctx, &reverse));
    }
}
