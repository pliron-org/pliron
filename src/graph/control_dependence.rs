// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Control dependence between CFG edges and blocks.
//!
//! This module uses the definition and algorithm in Section 3.6 of Robert
//! Morgan's "Building an Optimizing Compiler".
//!
//! A block `X` depends on edge `(B, S)` if
//!   - `X` post-dominates `S`, and
//!   - either `X == B` or `X` does not post-dominate `B`.
//!
//! The second condition can equivalently be stated as: `X` does not strictly post-dominate `B`.

use alloc::vec::Vec;

use crate::{
    context::{Context, Ptr},
    graph::{
        ControlFlowGraph, HasLabel,
        dominance::{PDomTree, compute_post_dominator_tree},
    },
    operation::Operation,
    pass::{Analysis, AnalysisManager},
    printable::{Printable, State, indented_nl},
    region::Region,
    result::Result,
    utils::table::{HMap, HSet, IMap, ISet},
};

/// A CFG edge, identified by its source and successor index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CfgEdge<N> {
    /// The source block of the edge.
    pub source: N,
    /// The index of the edge in the source block's successor list.
    pub successor_index: usize,
}

/// Control dependence for a CFG.
pub struct ControlDependence<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// Dependent block lists for each source block. Each mapped vector
    /// has one dependent list per real successor, in real successor order.
    edge_dependents: IMap<G::Node, Vec<Vec<G::Node>>>,
    /// Controlling edges for each block, in construction order.
    controlling_edges: HMap<G::Node, Vec<CfgEdge<G::Node>>>,
}

/// Computes direct control dependence with Morgan's post-dominator tree walk.
//
// For each real edge `(B, S)`, the walk starts at `S` and stops before `ipdom(B)`.
// It can include `B`, signifying self-dependence. `None` represents the sentinel.
//
// The algorithm stores each relation in both directions:
//
// ```text
// for each real edge (B, S):
//     X = S
//     while X != ipdom(B):
//         add X to EdgeDependents(B, S)
//         add (B, S) to ControllingEdges(X)
//         X = ipdom(X)
// ```
//
// Computation of direct control dependence takes `O(V + E * H)` time:
// - initialize `V` CFG nodes,
// - then walk at most `H` tree levels for each of the `E` CFG edges.
pub fn compute_control_dependence<G, GraphContext>(
    ctx: &GraphContext,
    graph: &G,
    post_dom_tree: &PDomTree<G, GraphContext>,
) -> ControlDependence<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    // Initialization ...
    let mut dependence = ControlDependence {
        edge_dependents: IMap::default(),
        controlling_edges: HMap::default(),
    };
    for node in graph.nodes(ctx) {
        assert!(
            post_dom_tree.contains(&node),
            "The post-dominator tree must contain every CFG node"
        );
        dependence
            .controlling_edges
            .insert(node.clone(), Vec::new());
        dependence.edge_dependents.insert(node, Vec::new());
    }

    assert_eq!(
        post_dom_tree.num_nodes(),
        dependence.edge_dependents.len(),
        "The post-dominator tree must have the same nodes as the CFG"
    );

    // Compute dependence for the outgoing edges of each CFG node.
    for source in graph.nodes(ctx) {
        // Stop before the source's immediate post-dominator.
        let stop = post_dom_tree.ipdom(&source);
        // Process all real edges.
        for successor_index in 0..graph.num_successors(ctx, &source) {
            let edge = CfgEdge {
                source: source.clone(),
                successor_index,
            };
            // Collect the blocks directly dependent on this edge.
            let mut dependents = Vec::new();
            let mut runner = Some(graph.get_successor(ctx, &source, successor_index));
            while runner != stop {
                // Get the current real block; the walk must not pass the sentinel.
                let node = runner.expect("The tree walk must reach the source's post-dominator");
                dependence
                    .controlling_edges
                    .get_mut(&node)
                    .expect("Every CFG successor must be a graph node")
                    .push(edge.clone());
                runner = post_dom_tree.ipdom(&node);
                dependents.push(node);
            }
            dependence
                .edge_dependents
                .get_mut(&source)
                .unwrap()
                .push(dependents);
        }
    }
    dependence
}

enum Direction {
    Controllers,
    Dependents,
}

impl<G, GraphContext> ControlDependence<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// Returns whether the analysis contains `block`.
    pub fn contains(&self, block: &G::Node) -> bool {
        self.edge_dependents.contains_key(block)
    }

    /// Returns the number of CFG nodes.
    pub fn num_nodes(&self) -> usize {
        self.edge_dependents.len()
    }

    /// Returns CFG nodes in graph order.
    pub fn nodes(&self) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.edge_dependents.keys().cloned()
    }

    /// Returns blocks directly dependent on an edge.
    pub fn dependents_of_edge(
        &self,
        source: &G::Node,
        successor_index: usize,
    ) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.edge_dependents[source][successor_index]
            .iter()
            .cloned()
    }

    /// Returns the CFG edges that directly control `block`.
    pub fn direct_controlling_edges(
        &self,
        block: &G::Node,
    ) -> impl Iterator<Item = CfgEdge<G::Node>> + Clone + '_ {
        self.controlling_edges[block].iter().cloned()
    }

    /// Returns the distinct blocks that directly control `block`.
    pub fn direct_controllers(&self, block: &G::Node) -> ISet<G::Node> {
        self.direct_controlling_edges(block)
            .map(|edge| edge.source)
            .collect()
    }

    /// Returns the distinct blocks directly dependent on `controller`.
    pub fn direct_dependents(&self, controller: &G::Node) -> ISet<G::Node> {
        self.edge_dependents[controller]
            .iter()
            .flat_map(|dependents| dependents.iter().cloned())
            .collect()
    }

    /// Returns all blocks that control `block`, directly or through other blocks.
    ///
    /// Includes `block` only if it controls itself, directly or through other blocks.
    ///
    /// The result is computed for each call.
    pub fn transitive_controllers(&self, block: &G::Node) -> ISet<G::Node> {
        self.transitive_nodes(block, Direction::Controllers)
    }

    /// Returns all blocks controlled by `controller`, directly or through other blocks.
    ///
    /// Includes `controller` only if it controls itself, directly or through other blocks.
    ///
    /// The result is computed for each call.
    pub fn transitive_dependents(&self, controller: &G::Node) -> ISet<G::Node> {
        self.transitive_nodes(controller, Direction::Dependents)
    }

    /// Returns whether `dependent` is directly control dependent on `controller`.
    pub fn is_directly_control_dependent(&self, dependent: &G::Node, controller: &G::Node) -> bool {
        assert!(
            self.contains(controller),
            "The controller must be a CFG node"
        );
        self.direct_controlling_edges(dependent)
            .any(|edge| edge.source == *controller)
    }

    /// Returns whether `dependent` is directly or indirectly control dependent on `controller`.
    ///
    /// The result is computed for each call.
    pub fn is_transitively_control_dependent(
        &self,
        dependent: &G::Node,
        controller: &G::Node,
    ) -> bool {
        assert!(
            self.contains(controller),
            "The controller must be a CFG node"
        );
        // Search direct and indirect controllers, visiting each block at most once.
        let mut visited = HSet::default();
        let mut pending = Vec::new();
        visited.insert(dependent.clone());
        pending.push(dependent.clone());
        while let Some(block) = pending.pop() {
            for edge in &self.controlling_edges[&block] {
                // Check each relation before skipping visited blocks to include cycles.
                if edge.source == *controller {
                    return true;
                }
                if visited.insert(edge.source.clone()) {
                    pending.push(edge.source.clone());
                }
            }
        }
        false
    }

    /// Collects direct and indirect controllers or dependents of `start`.
    fn transitive_nodes(&self, start: &G::Node, direction: Direction) -> ISet<G::Node> {
        let mut reached = match direction {
            Direction::Controllers => self.direct_controllers(start),
            Direction::Dependents => self.direct_dependents(start),
        };
        // The set is also a queue. Each inserted block is processed once.
        let mut index = 0;
        while let Some(node) = reached.get_index(index).cloned() {
            match direction {
                Direction::Controllers => reached.extend(self.direct_controllers(&node)),
                Direction::Dependents => reached.extend(self.direct_dependents(&node)),
            }
            index += 1;
        }
        reached
    }
}

/// Prints direct control dependence, with block labels and successor indices.
pub fn print_control_dependence<G, GraphContext>(
    ctx: &GraphContext,
    dependence: &ControlDependence<G, GraphContext>,
    state: &State,
    f: &mut impl core::fmt::Write,
) -> core::fmt::Result
where
    G: ControlFlowGraph<GraphContext>,
{
    f.write_str("control dependence:")?;
    let _indent = state.indent();
    for block in dependence.nodes() {
        write!(f, "{}block {}:", indented_nl(state), block.label(ctx))?;
        let _block_indent = state.indent();
        write!(f, "{}controlling edges: [", indented_nl(state))?;
        for (i, edge) in dependence.direct_controlling_edges(&block).enumerate() {
            if i != 0 {
                f.write_str(", ")?;
            }
            write!(f, "({}, {})", edge.source.label(ctx), edge.successor_index)?;
        }
        f.write_str("]")?;
        for (index, dependents) in dependence.edge_dependents[&block].iter().enumerate() {
            write!(f, "{}edge {index}: [", indented_nl(state))?;
            for (i, dependent) in dependents.iter().enumerate() {
                if i != 0 {
                    f.write_str(", ")?;
                }
                f.write_str(&dependent.label(ctx))?;
            }
            f.write_str("]")?;
        }
    }
    Ok(())
}

impl<G> Printable for ControlDependence<G, Context>
where
    G: ControlFlowGraph<Context>,
{
    fn fmt(
        &self,
        ctx: &Context,
        state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        print_control_dependence(ctx, self, state, f)
    }
}

/// Caches direct control dependence for regions in a program.
#[derive(Default)]
pub struct ControlDependenceInfo(HMap<Ptr<Region>, ControlDependence<Ptr<Region>, Context>>);

impl ControlDependenceInfo {
    /// Returns a region's control dependence, computing and caching it if required.
    pub fn get_control_dependence(
        &mut self,
        ctx: &Context,
        region: Ptr<Region>,
    ) -> &ControlDependence<Ptr<Region>, Context> {
        self.0.entry(region).or_insert_with(|| {
            let post_dom_tree = compute_post_dominator_tree(ctx, &region);
            compute_control_dependence(ctx, &region, &post_dom_tree)
        })
    }
}

impl Analysis for ControlDependenceInfo {
    fn name(&self) -> &'static str {
        "control_dependence_info"
    }

    fn compute(op: Ptr<Operation>, ctx: &Context, _analyses: &mut AnalysisManager) -> Result<Self> {
        let mut info = Self::default();
        for region in op.deref(ctx).regions() {
            info.get_control_dependence(ctx, region);
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{boxed::Box, string::String, string::ToString, vec};
    use core::fmt::Write as _;
    use expect_test::expect;

    use super::*;

    struct TestContext {
        successors: Vec<Vec<usize>>,
        predecessors: Vec<Vec<usize>>,
    }

    impl TestContext {
        fn new(successors: &[&[usize]]) -> Self {
            let successors: Vec<_> = successors.iter().map(|s| s.to_vec()).collect();
            let mut predecessors = vec![Vec::new(); successors.len()];
            for (source, succs) in successors.iter().enumerate() {
                for &successor in succs {
                    predecessors[successor].push(source);
                }
            }
            Self {
                successors,
                predecessors,
            }
        }

        fn dependence(&self) -> ControlDependence<TestGraph, Self> {
            let tree = compute_post_dominator_tree(self, &TestGraph);
            compute_control_dependence(self, &TestGraph, &tree)
        }
    }

    struct TestGraph;

    impl HasLabel<TestContext> for usize {
        fn label(&self, _ctx: &TestContext) -> String {
            self.to_string()
        }
    }

    impl ControlFlowGraph<TestContext> for TestGraph {
        type Node = usize;

        fn num_successors(&self, ctx: &TestContext, node: &usize) -> usize {
            ctx.successors[*node].len()
        }

        fn get_successor(&self, ctx: &TestContext, node: &usize, i: usize) -> usize {
            ctx.successors[*node][i]
        }

        fn num_predecessors(&self, ctx: &TestContext, node: &usize) -> usize {
            ctx.predecessors[*node].len()
        }

        fn get_predecessor(&self, ctx: &TestContext, node: &usize, i: usize) -> usize {
            ctx.predecessors[*node][i]
        }

        fn entry_node(&self, ctx: &TestContext) -> Option<usize> {
            (!ctx.successors.is_empty()).then_some(0)
        }

        fn nodes<'a>(&'a self, ctx: &'a TestContext) -> Box<dyn Iterator<Item = usize> + 'a> {
            Box::new(0..ctx.successors.len())
        }
    }

    fn sorted(nodes: impl IntoIterator<Item = usize>) -> Vec<usize> {
        let mut nodes: Vec<_> = nodes.into_iter().collect();
        nodes.sort_unstable();
        assert!(nodes.windows(2).all(|pair| pair[0] != pair[1]));
        nodes
    }

    fn edge(source: usize, successor_index: usize) -> CfgEdge<usize> {
        CfgEdge {
            source,
            successor_index,
        }
    }

    /// Prints the direct relations and query results with stable block labels.
    fn print_dependence<G, GraphContext>(
        ctx: &GraphContext,
        dependence: &ControlDependence<G, GraphContext>,
    ) -> String
    where
        G: ControlFlowGraph<GraphContext>,
    {
        let labels = |nodes: ISet<G::Node>| {
            nodes
                .iter()
                .map(|node| node.label(ctx))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut output = String::new();
        print_control_dependence(ctx, dependence, &State::default(), &mut output).unwrap();
        writeln!(output).unwrap();
        for block in dependence.nodes() {
            writeln!(
                output,
                "block {}: controllers: [{}]; dependents: [{}]; transitive controllers: [{}]; transitive dependents: [{}]",
                block.label(ctx),
                labels(dependence.direct_controllers(&block)),
                labels(dependence.direct_dependents(&block)),
                labels(dependence.transitive_controllers(&block)),
                labels(dependence.transitive_dependents(&block)),
            )
            .unwrap();
        }
        output
    }

    #[test]
    fn empty_graph_and_linear_chains() {
        for (successors, expected) in [
            (
                &[][..],
                expect![[r#"
                control dependence:
            "#]],
            ),
            (
                &[&[][..]][..],
                expect![[r#"
                control dependence:
                  block 0:
                    controlling edges: []
                block 0: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
            "#]],
            ),
            (
                &[&[1][..], &[2][..], &[][..]][..],
                expect![[r#"
                control dependence:
                  block 0:
                    controlling edges: []
                    edge 0: []
                  block 1:
                    controlling edges: []
                    edge 0: []
                  block 2:
                    controlling edges: []
                block 0: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
                block 1: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
                block 2: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
            "#]],
            ),
        ] {
            let ctx = TestContext::new(successors);
            let dependence = ctx.dependence();
            assert_eq!(dependence.num_nodes(), successors.len());
            assert_eq!(
                dependence.nodes().collect::<Vec<_>>(),
                (0..successors.len()).collect::<Vec<_>>()
            );
            assert!(!dependence.contains(&successors.len()));
            assert!(dependence.nodes().all(|node| dependence.contains(&node)));
            expected.assert_eq(&print_dependence(&ctx, &dependence));
        }
    }

    #[test]
    fn diamond_excludes_the_join() {
        let ctx = TestContext::new(&[&[1, 2], &[3], &[3], &[]]);
        let dependence = ctx.dependence();
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1]
                edge 1: [2]
              block 1:
                controlling edges: [(0, 0)]
                edge 0: []
              block 2:
                controlling edges: [(0, 1)]
                edge 0: []
              block 3:
                controlling edges: []
            block 0: controllers: []; dependents: [1, 2]; transitive controllers: []; transitive dependents: [1, 2]
            block 1: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 2: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 3: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &dependence));
        assert!(!dependence.is_transitively_control_dependent(&0, &0));
    }

    #[test]
    fn nested_branches_distinguish_direct_and_transitive_dependence() {
        let ctx = TestContext::new(&[&[1, 4], &[2, 3], &[3], &[4], &[]]);
        let dependence = ctx.dependence();
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1, 3]
                edge 1: []
              block 1:
                controlling edges: [(0, 0)]
                edge 0: [2]
                edge 1: []
              block 2:
                controlling edges: [(1, 0)]
                edge 0: []
              block 3:
                controlling edges: [(0, 0)]
                edge 0: []
              block 4:
                controlling edges: []
            block 0: controllers: []; dependents: [1, 3]; transitive controllers: []; transitive dependents: [1, 3, 2]
            block 1: controllers: [0]; dependents: [2]; transitive controllers: [0]; transitive dependents: [2]
            block 2: controllers: [1]; dependents: []; transitive controllers: [1, 0]; transitive dependents: []
            block 3: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 4: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &dependence));
        assert!(dependence.is_directly_control_dependent(&2, &1));
        assert!(!dependence.is_directly_control_dependent(&2, &0));
        assert!(dependence.is_transitively_control_dependent(&2, &0));
        assert!(!dependence.is_transitively_control_dependent(&0, &2));
        let first = print_dependence(&ctx, &dependence);
        assert_eq!(first, print_dependence(&ctx, &dependence));
    }

    #[test]
    fn a_block_can_have_multiple_direct_controllers() {
        let ctx = TestContext::new(&[&[1, 2], &[3, 4], &[3, 4], &[4], &[]]);
        let dependence = ctx.dependence();
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1]
                edge 1: [2]
              block 1:
                controlling edges: [(0, 0)]
                edge 0: [3]
                edge 1: []
              block 2:
                controlling edges: [(0, 1)]
                edge 0: [3]
                edge 1: []
              block 3:
                controlling edges: [(1, 0), (2, 0)]
                edge 0: []
              block 4:
                controlling edges: []
            block 0: controllers: []; dependents: [1, 2]; transitive controllers: []; transitive dependents: [1, 2, 3]
            block 1: controllers: [0]; dependents: [3]; transitive controllers: [0]; transitive dependents: [3]
            block 2: controllers: [0]; dependents: [3]; transitive controllers: [0]; transitive dependents: [3]
            block 3: controllers: [1, 2]; dependents: []; transitive controllers: [1, 2, 0]; transitive dependents: []
            block 4: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &dependence));
        assert!(!dependence.is_directly_control_dependent(&3, &0));
    }

    #[test]
    fn parallel_edges_keep_their_indices() {
        let ctx = TestContext::new(&[&[1, 1, 2], &[2], &[]]);
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1]
                edge 1: [1]
                edge 2: []
              block 1:
                controlling edges: [(0, 0), (0, 1)]
                edge 0: []
              block 2:
                controlling edges: []
            block 0: controllers: []; dependents: [1]; transitive controllers: []; transitive dependents: [1]
            block 1: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 2: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &ctx.dependence()));
    }

    #[test]
    fn loop_header_depends_on_its_continue_edge() {
        let ctx = TestContext::new(&[&[1, 2], &[0], &[]]);
        let dependence = ctx.dependence();
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: [(0, 0)]
                edge 0: [1, 0]
                edge 1: []
              block 1:
                controlling edges: [(0, 0)]
                edge 0: []
              block 2:
                controlling edges: []
            block 0: controllers: [0]; dependents: [1, 0]; transitive controllers: [0]; transitive dependents: [1, 0]
            block 1: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 2: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &dependence));
        assert!(dependence.is_directly_control_dependent(&0, &0));
        assert!(dependence.is_transitively_control_dependent(&0, &0));
        assert!(!dependence.is_transitively_control_dependent(&1, &1));
    }

    #[test]
    fn transitive_queries_handle_cycles_without_direct_self_dependence() {
        let ctx = TestContext::new(&[&[1, 2], &[2, 3], &[1, 3], &[]]);
        let dependence = ctx.dependence();
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1]
                edge 1: [2]
              block 1:
                controlling edges: [(0, 0), (2, 0)]
                edge 0: [2]
                edge 1: []
              block 2:
                controlling edges: [(0, 1), (1, 0)]
                edge 0: [1]
                edge 1: []
              block 3:
                controlling edges: []
            block 0: controllers: []; dependents: [1, 2]; transitive controllers: []; transitive dependents: [1, 2]
            block 1: controllers: [0, 2]; dependents: [2]; transitive controllers: [0, 2, 1]; transitive dependents: [2, 1]
            block 2: controllers: [0, 1]; dependents: [1]; transitive controllers: [0, 1, 2]; transitive dependents: [1, 2]
            block 3: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &dependence));
        assert!(!dependence.is_directly_control_dependent(&1, &1));
        assert!(dependence.is_transitively_control_dependent(&1, &1));
        assert!(!dependence.is_transitively_control_dependent(&0, &0));
    }

    #[test]
    fn multiple_exits_stop_at_the_sentinel() {
        let ctx = TestContext::new(&[&[1, 2], &[], &[]]);
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1]
                edge 1: [2]
              block 1:
                controlling edges: [(0, 0)]
              block 2:
                controlling edges: [(0, 1)]
            block 0: controllers: []; dependents: [1, 2]; transitive controllers: []; transitive dependents: [1, 2]
            block 1: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 2: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &ctx.dependence()));
    }

    #[test]
    fn infinite_loops_use_the_post_dominator_exit_policy() {
        let ctx = TestContext::new(&[&[1], &[0]]);
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: [(1, 0)]
                edge 0: []
              block 1:
                controlling edges: [(1, 0)]
                edge 0: [0, 1]
            block 0: controllers: [1]; dependents: []; transitive controllers: [1]; transitive dependents: []
            block 1: controllers: [1]; dependents: [0, 1]; transitive controllers: [1]; transitive dependents: [0, 1]
        "#]].assert_eq(&print_dependence(&ctx, &ctx.dependence()));

        let ctx = TestContext::new(&[&[1, 2], &[], &[3], &[2]]);
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: [1]
                edge 1: [2, 3]
              block 1:
                controlling edges: [(0, 0)]
              block 2:
                controlling edges: [(0, 1), (3, 0)]
                edge 0: []
              block 3:
                controlling edges: [(0, 1), (3, 0)]
                edge 0: [2, 3]
            block 0: controllers: []; dependents: [1, 2, 3]; transitive controllers: []; transitive dependents: [1, 2, 3]
            block 1: controllers: [0]; dependents: []; transitive controllers: [0]; transitive dependents: []
            block 2: controllers: [0, 3]; dependents: []; transitive controllers: [0, 3]; transitive dependents: []
            block 3: controllers: [0, 3]; dependents: [2, 3]; transitive controllers: [0, 3]; transitive dependents: [2, 3]
        "#]].assert_eq(&print_dependence(&ctx, &ctx.dependence()));
    }

    #[test]
    fn includes_blocks_unreachable_from_entry() {
        let ctx = TestContext::new(&[&[1], &[], &[3, 4], &[4], &[]]);
        let dependence = ctx.dependence();
        assert_eq!(dependence.num_nodes(), 5);
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: []
              block 1:
                controlling edges: []
              block 2:
                controlling edges: []
                edge 0: [3]
                edge 1: []
              block 3:
                controlling edges: [(2, 0)]
                edge 0: []
              block 4:
                controlling edges: []
            block 0: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
            block 1: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
            block 2: controllers: []; dependents: [3]; transitive controllers: []; transitive dependents: [3]
            block 3: controllers: [2]; dependents: []; transitive controllers: [2]; transitive dependents: []
            block 4: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&print_dependence(&ctx, &dependence));
    }

    #[test]
    fn predecessor_order_does_not_change_dependence() {
        let mut ctx = TestContext::new(&[&[1], &[2, 5], &[3, 4], &[2, 1], &[2, 1], &[]]);
        let first = print_dependence(&ctx, &ctx.dependence());
        for predecessors in &mut ctx.predecessors {
            predecessors.reverse();
        }
        assert_eq!(first, print_dependence(&ctx, &ctx.dependence()));
        expect![[r#"
            control dependence:
              block 0:
                controlling edges: []
                edge 0: []
              block 1:
                controlling edges: [(1, 0)]
                edge 0: [2, 1]
                edge 1: []
              block 2:
                controlling edges: [(1, 0), (3, 0), (4, 0)]
                edge 0: [3]
                edge 1: [4]
              block 3:
                controlling edges: [(2, 0)]
                edge 0: [2]
                edge 1: []
              block 4:
                controlling edges: [(2, 1)]
                edge 0: [2]
                edge 1: []
              block 5:
                controlling edges: []
            block 0: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
            block 1: controllers: [1]; dependents: [2, 1]; transitive controllers: [1]; transitive dependents: [2, 1, 3, 4]
            block 2: controllers: [1, 3, 4]; dependents: [3, 4]; transitive controllers: [1, 3, 4, 2]; transitive dependents: [3, 4, 2]
            block 3: controllers: [2]; dependents: [2]; transitive controllers: [2, 1, 3, 4]; transitive dependents: [2, 3, 4]
            block 4: controllers: [2]; dependents: [2]; transitive controllers: [2, 1, 3, 4]; transitive dependents: [2, 3, 4]
            block 5: controllers: []; dependents: []; transitive controllers: []; transitive dependents: []
        "#]].assert_eq(&first);
    }

    #[test]
    fn printer_respects_indent() {
        let ctx = TestContext::new(&[&[1, 2], &[2], &[]]);
        let dependence = ctx.dependence();
        let state = State::default();
        state.set_indent_width(4);
        let _indent = state.indent();
        let mut output = String::new();
        print_control_dependence(&ctx, &dependence, &state, &mut output).unwrap();
        expect![[r#"
            control dependence:
                    block 0:
                        controlling edges: []
                        edge 0: [1]
                        edge 1: []
                    block 1:
                        controlling edges: [(0, 0)]
                        edge 0: []
                    block 2:
                        controlling edges: []"#]]
        .assert_eq(&output);
        assert_eq!(state.current_indent(), 4);
    }

    /// Tests post-dominance by searching for an exit path that avoids a block.
    fn reaches_exit_without(
        ctx: &TestContext,
        pre_sentinels: &ISet<usize>,
        start: usize,
        excluded: usize,
    ) -> bool {
        let mut pending = vec![start];
        let mut seen = vec![false; ctx.successors.len()];
        while let Some(node) = pending.pop() {
            if node == excluded || seen[node] {
                continue;
            }
            seen[node] = true;
            if pre_sentinels.contains(&node) {
                return true;
            }
            pending.extend(ctx.successors[node].iter().copied());
        }
        false
    }

    /// Checks all queries against exit reachability and a matrix closure.
    fn check_with_reachability(ctx: &TestContext) {
        let tree = compute_post_dominator_tree(ctx, &TestGraph);
        let dependence = compute_control_dependence(ctx, &TestGraph, &tree);
        let pre_sentinels: ISet<_> = tree.pre_sentinels().collect();
        let count = ctx.successors.len();
        let mut direct = vec![vec![false; count]; count];
        let mut controlling_edges = vec![Vec::new(); count];

        for (source, row) in direct.iter_mut().enumerate() {
            for (index, &successor) in ctx.successors[source].iter().enumerate() {
                let mut expected = Vec::new();
                for (dependent, incoming) in controlling_edges.iter_mut().enumerate() {
                    if !reaches_exit_without(ctx, &pre_sentinels, successor, dependent)
                        && (dependent == source
                            || reaches_exit_without(ctx, &pre_sentinels, source, dependent))
                    {
                        row[dependent] = true;
                        incoming.push(edge(source, index));
                        expected.push(dependent);
                    }
                }
                assert_eq!(
                    sorted(dependence.dependents_of_edge(&source, index)),
                    expected
                );
            }
        }
        let mut closure = direct.clone();
        for via in 0..count {
            for source in 0..count {
                for dependent in 0..count {
                    closure[source][dependent] |= closure[source][via] && closure[via][dependent];
                }
            }
        }
        for node in 0..count {
            assert_eq!(
                dependence
                    .direct_controlling_edges(&node)
                    .collect::<Vec<_>>(),
                controlling_edges[node]
            );
            assert_eq!(
                sorted(dependence.direct_dependents(&node)),
                (0..count).filter(|&n| direct[node][n]).collect::<Vec<_>>()
            );
            assert_eq!(
                sorted(dependence.direct_controllers(&node)),
                (0..count).filter(|&n| direct[n][node]).collect::<Vec<_>>()
            );
            assert_eq!(
                sorted(dependence.transitive_dependents(&node)),
                (0..count).filter(|&n| closure[node][n]).collect::<Vec<_>>()
            );
            assert_eq!(
                sorted(dependence.transitive_controllers(&node)),
                (0..count).filter(|&n| closure[n][node]).collect::<Vec<_>>()
            );
            for controller in 0..count {
                assert_eq!(
                    dependence.is_directly_control_dependent(&node, &controller),
                    direct[controller][node]
                );
                assert_eq!(
                    dependence.is_transitively_control_dependent(&node, &controller),
                    closure[controller][node]
                );
            }
        }
    }

    #[test]
    fn all_graphs_with_up_to_three_nodes_match_exit_reachability() {
        for count in 0..=3 {
            for mask in 0..(1usize << (count * count)) {
                let successors: Vec<Vec<_>> = (0..count)
                    .map(|source| {
                        (0..count)
                            .filter(|&target| mask & (1 << (source * count + target)) != 0)
                            .collect()
                    })
                    .collect();
                let successors: Vec<_> = successors.iter().map(Vec::as_slice).collect();
                check_with_reachability(&TestContext::new(&successors));
            }
        }
        for successors in [
            &[&[1, 2][..], &[3], &[3], &[]][..],
            &[&[1, 4][..], &[2, 3], &[3], &[4], &[]][..],
            &[&[1, 2][..], &[3, 4], &[3, 4], &[4], &[]][..],
            &[&[1, 1, 2][..], &[2], &[]][..],
            &[&[1, 2][..], &[2, 3], &[1, 3], &[]][..],
            &[&[1, 2][..], &[], &[3], &[2]][..],
            &[&[1][..], &[], &[3, 4], &[4], &[]][..],
            &[&[1][..], &[2, 5], &[3, 4], &[2, 1], &[2, 1], &[]][..],
        ] {
            check_with_reachability(&TestContext::new(successors));
        }
    }
}
