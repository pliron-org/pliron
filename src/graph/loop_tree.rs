// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Natural loops and their loop tree.
//!
//! This module uses the algorithm in Section 3.7.3 of Robert Morgan's
//! "Building an Optimizing Compiler", restricted to single-entry (reducible) loops.
//!   - Each loop entry (also called header) dominates every block in its loop.
//!   - Only blocks reachable from the graph entry are included.
//!   - Cycles with multiple entries (irreducible loops) are not represented,
//!     but natural loops within them are included.

use alloc::vec::Vec;

use crate::{
    context::{Context, Ptr},
    graph::{
        ControlFlowGraph,
        dominance::{DomInfo, DomTree},
        traversals::region::DFSTraversal,
    },
    operation::Operation,
    pass::{Analysis, AnalysisManager},
    region::Region,
    result::Result,
    utils::table::{HMap, ISet},
};

/// Identifies a loop within one [LoopTree].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopId(usize);

/// An immediate child of a loop or of the loop tree root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoopTreeNode<N> {
    Block(N),
    Loop(LoopId),
}

/// A natural loop and its position in the loop tree.
struct Loop<N> {
    /// The header block that dominates every block in the loop.
    header: N,
    /// The immediate enclosing loop, or `None` for a top-level loop.
    parent: Option<LoopId>,
    /// Blocks and subloops directly contained in the loop, in RPO.
    contents: Vec<LoopTreeNode<N>>,
    /// Sources of back edges to the header block, in RPO.
    latches: ISet<N>,
    /// All blocks in the loop and its subloops, with the header first.
    blocks: ISet<N>,
    /// The loop depth; top-level loops have depth one.
    depth: usize,
}

/// The loop tree of a control-flow graph.
///
/// The root represents the reachable part of the graph. Its children are
/// top-level loops and blocks outside all loops. Each block is a leaf, with
/// its innermost loop as its parent. Each loop has blocks or loops as children.
pub struct LoopTree<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// Loops in construction order; each LoopId indexes this vector.
    loops: Vec<Loop<G::Node>>,
    /// Loop IDs in RPO of their header blocks.
    loop_order: Vec<LoopId>,
    /// Immediate children of the graph root, in RPO.
    root_contents: Vec<LoopTreeNode<G::Node>>,
    /// Maps blocks in loops to their innermost loop.
    block_parent: HMap<G::Node, LoopId>,
}

// ## Algorithm Description
//
// The pseudocode below uses the book's terminology.
//   - `Generators` are the sources of back edges to a loop entry (also called latches).
//   - `LoopContains` holds immediate children: blocks or completed loops.
//   - `LoopEntry` is the header of a loop, or the block itself for a block node.
//   - `NIL` means that a node has no loop parent; it becomes a child of the graph root.
//
// ```text
// CALCULATE_LOOP_TREE(Graph, DomTree):
//     DFS = depth_first_search(Graph)
//     initialize all LoopParent links to NIL
//
//     for each block B in DFS postorder:
//         FIND_LOOP(B)
//
//     - Attach all parentless reachable blocks and loops to the graph root.
//     - Compute inclusive block sets and loop depths.
//
// FIND_LOOP(B):
//     Generators = { P in predecessors(B)
//                    where P is reachable and B dominates P }
//     if Generators is not empty:
//         FIND_BODY(Generators, B)
//
// FIND_BODY(Generators, Head):
//     Body = empty set
//     Queue = empty worklist
//
//     for each G in Generators:
//         if G != Head:
//             ADD_TO_BODY(LOOP_ANCESTOR(G))
//
//     while Queue is not empty:
//         X = remove one node from Queue
//         for each P in predecessors(LoopEntry(X)):
//             if P is reachable and P != Head:
//                 ADD_TO_BODY(LOOP_ANCESTOR(P))
//
//     add Head to Body
//     L = new loop node
//     LoopEntry(L) = Head
//     LoopParent(L) = NIL
//     LoopContains(L) = Body
//     Generators(L) = Generators
//     for each X in Body:
//         LoopParent(X) = L
//
// ADD_TO_BODY(X):
//     if X is not already in Body:
//         add X to Body
//         add X to Queue
//
// LOOP_ANCESTOR(X):
//     while LoopParent(X) != NIL:
//         X = LoopParent(X)
//     return X
// ```
//
// The dominance test selects natural-loop back edges, including self-edges.
// DFS postorder ensures that inner loops are constructed first, so
// `LOOP_ANCESTOR` can treat each completed inner loop as one unit.

/// Computes the natural-loop tree for `graph`.
pub fn compute_loop_tree<G, GraphContext>(
    ctx: &GraphContext,
    graph: &G,
    dom_tree: &DomTree<G, GraphContext>,
) -> LoopTree<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    assert!(
        dom_tree.root() == graph.entry_node(ctx),
        "The dominator tree must have the graph entry as its root"
    );

    // Dominator-tree DFS intervals give constant-time dominance tests:
    // A dominates B if and only if start(A) <= start(B) < end(A).
    let mut intervals = HMap::<G::Node, (usize, usize)>::default();
    let mut dom_stack = Vec::new();
    if let Some(root) = dom_tree.root() {
        dom_stack.push((root, false));
    }
    // Iterative DFS over the dominator tree rooted at root.
    let mut clock = 0;
    while let Some((node, finished)) = dom_stack.pop() {
        if finished {
            // Record the end time. It is larger than all start times in this subtree.
            intervals.get_mut(&node).unwrap().1 = clock;
        } else {
            // Record the visit start time.
            intervals.insert(node.clone(), (clock, 0));
            clock += 1;
            // Push the node again (below its children), for it to finish after its subtree.
            dom_stack.push((node.clone(), true));
            // Add tree children to the stack.
            dom_stack.extend(dom_tree.children(&node).map(|child| (child, false)));
        }
    }

    // Start with an empty loop tree.
    let mut tree = LoopTree {
        loops: Vec::new(),
        loop_order: Vec::new(),
        root_contents: Vec::new(),
        block_parent: HMap::default(),
    };
    // DFS over the control-flow-graph.
    let dfs = DFSTraversal::new(ctx, graph);

    // FIND_LOOP: find latches and construct loop bodies in DFS postorder.
    for header in dfs.post_order() {
        let &(header_start, header_end) = intervals
            .get(&header)
            .expect("The dominator tree must contain every reachable block");
        let latches: ISet<_> = graph
            .predecessors(ctx, &header)
            .into_iter()
            .filter(|pred| {
                // Ignore predecessors that are not reachable from the graph entry.
                let Some(&(pred_start, _)) = intervals.get(pred) else {
                    return false;
                };
                // If the header dominates a predecessor, its edge is a back edge.
                header_start <= pred_start && pred_start < header_end
            })
            .collect();
        if !latches.is_empty() {
            tree.find_body(ctx, graph, dom_tree, header, latches);
        }
    }

    // Collect reachable blocks in CFG reverse postorder.
    let nodes: ISet<_> = dfs.reverse_post_order().collect();
    // Each block's set index gives its RPO position.
    let rpo_position = |node: &G::Node| {
        nodes
            .get_index_of(node)
            .expect("Reachable block must have an RPO position")
    };
    // Record each loop header's RPO position, indexed by LoopId.
    let header_positions: Vec<_> = tree.loops.iter().map(|l| rpo_position(&l.header)).collect();
    // Order a block by its RPO position and a loop by its header's position.
    let child_position = |node: &LoopTreeNode<G::Node>| match node {
        LoopTreeNode::Block(block) => rpo_position(block),
        LoopTreeNode::Loop(id) => header_positions[id.0],
    };
    // List all loops in RPO of their headers.
    tree.loop_order = (0..tree.loops.len()).map(LoopId).collect();
    tree.loop_order.sort_by_key(|id| header_positions[id.0]);
    // Order the immediate children of each loop by RPO position.
    for l in &mut tree.loops {
        l.contents.sort_by_key(&child_position);
        // Put the latches in RPO.
        l.latches.sort_by_key(|node| rpo_position(node));
    }

    // Populate inclusive block sets; each header is already first.
    for node in &nodes {
        let mut parent = tree.block_loop_parent(node);
        // Blocks outside all loops are immediate children of the graph root.
        if parent.is_none() {
            tree.root_contents.push(LoopTreeNode::Block(node.clone()));
        }
        // Add the block to its innermost loop and every enclosing loop.
        while let Some(id) = parent {
            tree.loops[id.0].blocks.insert(node.clone());
            parent = tree.loops[id.0].parent;
        }
    }
    // Attach top-level loops to the graph root and compute loop depths.
    for &id in &tree.loop_order {
        if tree.loops[id.0].parent.is_none() {
            tree.root_contents.push(LoopTreeNode::Loop(id));
        }
        // Count this loop and all its enclosing loops.
        let mut depth = 1;
        let mut parent = tree.loops[id.0].parent;
        while let Some(id) = parent {
            depth += 1;
            parent = tree.loops[id.0].parent;
        }
        tree.loops[id.0].depth = depth;
    }
    // Put the root's blocks and loops in RPO.
    tree.root_contents.sort_by_key(child_position);
    tree
}

impl<G, GraphContext> LoopTree<G, GraphContext>
where
    G: ControlFlowGraph<GraphContext>,
{
    /// LOOP_ANCESTOR: finds the outermost constructed loop containing `block`.
    /// Returns the block itself if it has no loop parent.
    fn loop_ancestor(&self, block: G::Node) -> LoopTreeNode<G::Node> {
        let Some(mut id) = self.block_loop_parent(&block) else {
            return LoopTreeNode::Block(block);
        };
        while let Some(parent) = self.loops[id.0].parent {
            id = parent;
        }
        LoopTreeNode::Loop(id)
    }

    /// FIND_BODY: walks backward from latches and sets parent links.
    fn find_body(
        &mut self,
        ctx: &GraphContext,
        graph: &G,
        dom_tree: &DomTree<G, GraphContext>,
        header: G::Node,
        latches: ISet<G::Node>,
    ) {
        let mut body = ISet::default();
        let mut queue = Vec::new();
        for latch in &latches {
            // A self-edge creates a loop, but must not start a walk out of it.
            if *latch != header {
                let ancestor = self.loop_ancestor(latch.clone());
                if body.insert(ancestor.clone()) {
                    queue.push(ancestor);
                }
            }
        }
        while let Some(node) = queue.pop() {
            let block = match node {
                LoopTreeNode::Block(block) => block,
                LoopTreeNode::Loop(id) => self.loops[id.0].header.clone(),
            };
            for pred in graph.predecessors(ctx, &block) {
                if pred != header && dom_tree.contains(&pred) {
                    let ancestor = self.loop_ancestor(pred);
                    if body.insert(ancestor.clone()) {
                        queue.push(ancestor);
                    }
                }
            }
        }
        body.insert(LoopTreeNode::Block(header.clone()));
        let id = LoopId(self.loops.len());
        for node in &body {
            match node {
                LoopTreeNode::Block(block) => {
                    self.block_parent.insert(block.clone(), id);
                }
                LoopTreeNode::Loop(child) => {
                    self.loops[child.0].parent = Some(id);
                }
            }
        }
        self.loops.push(Loop {
            header: header.clone(),
            parent: None,
            contents: body.into_iter().collect(),
            latches,
            blocks: core::iter::once(header).collect(),
            depth: 0,
        });
    }

    /// Returns all loops in RPO of their headers, with parents before children.
    pub fn loops(&self) -> impl Iterator<Item = LoopId> + Clone + '_ {
        self.loop_order.iter().copied()
    }

    /// Returns the number of loops.
    pub fn num_loops(&self) -> usize {
        self.loops.len()
    }

    /// Returns the top-level loops in RPO of their headers.
    pub fn top_level_loops(&self) -> impl Iterator<Item = LoopId> + Clone + '_ {
        self.loops().filter(|id| self.loop_parent(*id).is_none())
    }

    /// Returns the immediate children of the graph root in RPO.
    pub fn root_contents(&self) -> impl Iterator<Item = LoopTreeNode<G::Node>> + Clone + '_ {
        self.root_contents.iter().cloned()
    }

    /// Returns the header block of a loop.
    pub fn loop_header(&self, id: LoopId) -> G::Node {
        self.loops[id.0].header.clone()
    }

    /// Returns the immediate loop parent. `None` means the graph root.
    pub fn loop_parent(&self, id: LoopId) -> Option<LoopId> {
        self.loops[id.0].parent
    }

    /// Returns the blocks and loops directly contained in a loop,
    /// in RPO (with subloops ordered by their headers).
    /// Blocks inside subloops are represented by those subloops and not repeated.
    pub fn loop_contents(
        &self,
        id: LoopId,
    ) -> impl Iterator<Item = LoopTreeNode<G::Node>> + Clone + '_ {
        self.loops[id.0].contents.iter().cloned()
    }

    /// Returns the immediate subloops in RPO of their headers.
    pub fn subloops(&self, id: LoopId) -> impl Iterator<Item = LoopId> + Clone + '_ {
        self.loop_contents(id).filter_map(|node| match node {
            LoopTreeNode::Block(_) => None,
            LoopTreeNode::Loop(id) => Some(id),
        })
    }

    /// Returns all blocks in a loop, including blocks in its subloops.
    /// The loop header is first, followed by the other blocks in RPO.
    pub fn blocks(&self, id: LoopId) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.loops[id.0].blocks.iter().cloned()
    }

    /// Returns the sources of back edges to a loop header, in RPO.
    pub fn latches(&self, id: LoopId) -> impl Iterator<Item = G::Node> + Clone + '_ {
        self.loops[id.0].latches.iter().cloned()
    }

    /// Returns the immediate loop parent of a block (its innermost loop).
    /// Returns `None` for blocks outside loops, including unreachable blocks.
    pub fn block_loop_parent(&self, block: &G::Node) -> Option<LoopId> {
        self.block_parent.get(block).copied()
    }

    /// Returns whether a loop contains a block, directly or within a subloop.
    pub fn contains(&self, id: LoopId, block: &G::Node) -> bool {
        self.loops[id.0].blocks.contains(block)
    }

    /// Returns the loop depth. Top-level loops have depth one.
    pub fn loop_depth(&self, id: LoopId) -> usize {
        self.loops[id.0].depth
    }

    /// Returns the depth of a block's innermost loop, or zero for blocks not in any loop.
    pub fn block_loop_depth(&self, block: &G::Node) -> usize {
        self.block_loop_parent(block)
            .map_or(0, |id| self.loop_depth(id))
    }

    /// Returns blocks inside a loop that have a successor outside it.
    pub fn exiting_blocks<'a>(
        &'a self,
        ctx: &'a GraphContext,
        graph: &'a G,
        id: LoopId,
    ) -> impl Iterator<Item = G::Node> + 'a {
        self.blocks(id).filter(move |block| {
            graph
                .successors(ctx, block)
                .iter()
                .any(|succ| !self.contains(id, succ))
        })
    }

    /// Returns distinct successors outside a loop, in block and successor order.
    pub fn exit_blocks(
        &self,
        ctx: &GraphContext,
        graph: &G,
        id: LoopId,
    ) -> impl Iterator<Item = G::Node> {
        let mut exits = ISet::default();
        for block in self.blocks(id) {
            exits.extend(
                graph
                    .successors(ctx, &block)
                    .into_iter()
                    .filter(|succ| !self.contains(id, succ)),
            );
        }
        exits.into_iter()
    }
}

/// Caches loop trees for regions in a program.
#[derive(Default)]
pub struct LoopInfo(HMap<Ptr<Region>, LoopTree<Ptr<Region>, Context>>);

impl LoopInfo {
    /// Returns a region's loop tree, computing and caching it if required.
    /// Uses `dom_info` to compute or reuse the region's dominator tree.
    pub fn get_loop_tree(
        &mut self,
        ctx: &Context,
        region: Ptr<Region>,
        dom_info: &mut DomInfo,
    ) -> &LoopTree<Ptr<Region>, Context> {
        self.0
            .entry(region)
            .or_insert_with(|| compute_loop_tree(ctx, &region, dom_info.get_dom_tree(ctx, region)))
    }
}

impl Analysis for LoopInfo {
    fn name(&self) -> &'static str {
        "loop_info"
    }

    fn compute(op: Ptr<Operation>, ctx: &Context, analyses: &mut AnalysisManager) -> Result<Self> {
        let mut dom_info = analyses.get_analysis_mut::<DomInfo>(op, ctx)?;
        let mut loop_info = Self::default();
        for region in op.deref(ctx).regions() {
            loop_info.get_loop_tree(ctx, region, &mut dom_info);
        }
        Ok(loop_info)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{boxed::Box, format, string::String, string::ToString, vec};
    use core::fmt::Write as _;
    use expect_test::expect;

    use super::*;
    use crate::{
        basic_block::BasicBlock,
        builtin::{
            op_interfaces::{IsTerminatorInterface, OneRegionInterface},
            ops::FuncOp,
            types::FunctionType,
        },
        derive::pliron_op,
        graph::{HasLabel, dominance::compute_dominator_tree},
        ident,
        op::Op,
    };

    struct TestGraph {
        successors: Vec<Vec<usize>>,
        predecessors: Vec<Vec<usize>>,
    }

    impl TestGraph {
        fn new(successors: Vec<Vec<usize>>) -> Self {
            let mut predecessors = vec![Vec::new(); successors.len()];
            for (node, succs) in successors.iter().enumerate() {
                for &succ in succs {
                    predecessors[succ].push(node);
                }
            }
            Self {
                successors,
                predecessors,
            }
        }

        fn loop_tree(&self) -> LoopTree<Self, ()> {
            compute_loop_tree(&(), self, &compute_dominator_tree(&(), self))
        }
    }

    impl HasLabel<()> for usize {
        fn label(&self, _ctx: &()) -> String {
            self.to_string()
        }
    }

    impl ControlFlowGraph<()> for TestGraph {
        type Node = usize;

        fn num_successors(&self, _ctx: &(), node: &usize) -> usize {
            self.successors[*node].len()
        }

        fn get_successor(&self, _ctx: &(), node: &usize, i: usize) -> usize {
            self.successors[*node][i]
        }

        fn num_predecessors(&self, _ctx: &(), node: &usize) -> usize {
            self.predecessors[*node].len()
        }

        fn get_predecessor(&self, _ctx: &(), node: &usize, i: usize) -> usize {
            self.predecessors[*node][i]
        }

        fn entry_node(&self, _ctx: &()) -> Option<usize> {
            (!self.successors.is_empty()).then_some(0)
        }

        fn nodes<'a>(&'a self, _ctx: &'a ()) -> Box<dyn Iterator<Item = usize> + 'a> {
            Box::new(0..self.successors.len())
        }
    }

    fn graph(successors: &[&[usize]]) -> TestGraph {
        TestGraph::new(successors.iter().map(|s| s.to_vec()).collect())
    }

    fn loop_at(tree: &LoopTree<TestGraph, ()>, header: usize) -> LoopId {
        tree.loops()
            .find(|&id| tree.loop_header(id) == header)
            .expect("Expected a loop with this header")
    }

    /// Prints the full tree and its query results with stable block labels.
    fn print_tree<G, GraphContext>(
        ctx: &GraphContext,
        graph: &G,
        tree: &LoopTree<G, GraphContext>,
    ) -> String
    where
        G: ControlFlowGraph<GraphContext>,
    {
        let labels = |nodes: Vec<G::Node>| {
            nodes
                .iter()
                .map(|node| node.label(ctx))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let parent_label = |parent: Option<LoopId>| {
            parent.map_or_else(|| "root".to_string(), |id| tree.loop_header(id).label(ctx))
        };
        let mut output = String::new();
        writeln!(
            output,
            "loops ({}): [{}]",
            tree.num_loops(),
            labels(tree.loops().map(|id| tree.loop_header(id)).collect())
        )
        .unwrap();
        writeln!(
            output,
            "top-level loops: [{}]",
            labels(
                tree.top_level_loops()
                    .map(|id| tree.loop_header(id))
                    .collect()
            )
        )
        .unwrap();
        writeln!(
            output,
            "block parents: [{}]",
            graph
                .nodes(ctx)
                .map(|node| {
                    format!(
                        "{}: {}",
                        node.label(ctx),
                        parent_label(tree.block_loop_parent(&node))
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        writeln!(
            output,
            "block depths: [{}]",
            graph
                .nodes(ctx)
                .map(|node| { format!("{}: {}", node.label(ctx), tree.block_loop_depth(&node)) })
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        writeln!(output, "root:").unwrap();
        let mut queue: Vec<_> = tree.root_contents().map(|node| (node, 1)).collect();
        queue.reverse();
        while let Some((node, depth)) = queue.pop() {
            let indent = "  ".repeat(depth);
            match node {
                LoopTreeNode::Block(block) => {
                    writeln!(output, "{indent}block {}", block.label(ctx)).unwrap();
                }
                LoopTreeNode::Loop(id) => {
                    writeln!(
                        output,
                        "{indent}loop {} (parent: {}, depth: {})",
                        tree.loop_header(id).label(ctx),
                        parent_label(tree.loop_parent(id)),
                        tree.loop_depth(id)
                    )
                    .unwrap();
                    writeln!(
                        output,
                        "{indent}  latches: [{}]",
                        labels(tree.latches(id).collect())
                    )
                    .unwrap();
                    writeln!(
                        output,
                        "{indent}  blocks: [{}]",
                        labels(tree.blocks(id).collect())
                    )
                    .unwrap();
                    writeln!(
                        output,
                        "{indent}  exiting blocks: [{}]; exit blocks: [{}]",
                        labels(tree.exiting_blocks(ctx, graph, id).collect()),
                        labels(tree.exit_blocks(ctx, graph, id).collect())
                    )
                    .unwrap();
                    let mut children: Vec<_> = tree.loop_contents(id).collect();
                    children.reverse();
                    queue.extend(children.into_iter().map(|node| (node, depth + 1)));
                }
            }
        }
        output
    }

    #[test]
    fn empty_and_acyclic_graphs() {
        for (g, expected) in [
            (
                graph(&[]),
                expect![[r#"
                loops (0): []
                top-level loops: []
                block parents: []
                block depths: []
                root:
            "#]],
            ),
            (
                graph(&[&[]]),
                expect![[r#"
                loops (0): []
                top-level loops: []
                block parents: [0: root]
                block depths: [0: 0]
                root:
                  block 0
            "#]],
            ),
            (
                graph(&[&[1, 2], &[3], &[3], &[]]),
                expect![[r#"
                    loops (0): []
                    top-level loops: []
                    block parents: [0: root, 1: root, 2: root, 3: root]
                    block depths: [0: 0, 1: 0, 2: 0, 3: 0]
                    root:
                      block 0
                      block 2
                      block 1
                      block 3
                "#]],
            ),
        ] {
            expected.assert_eq(&print_tree(&(), &g, &g.loop_tree()));
        }
    }

    #[test]
    fn self_edge_at_graph_entry() {
        let g = graph(&[&[0, 1], &[]]);
        expect![[r#"
            loops (1): [0]
            top-level loops: [0]
            block parents: [0: 0, 1: root]
            block depths: [0: 1, 1: 0]
            root:
              loop 0 (parent: root, depth: 1)
                latches: [0]
                blocks: [0]
                exiting blocks: [0]; exit blocks: [1]
                block 0
              block 1
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn several_latches_and_duplicate_edges() {
        let g = graph(&[&[1], &[1, 2, 3], &[1, 1], &[1]]);
        expect![[r#"
            loops (1): [1]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 1, 3: 1]
            block depths: [0: 0, 1: 1, 2: 1, 3: 1]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [1, 3, 2]
                blocks: [1, 3, 2]
                exiting blocks: []; exit blocks: []
                block 1
                block 3
                block 2
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn nested_loops_use_loop_ancestors() {
        let g = graph(&[&[1], &[2, 5], &[3, 4], &[2], &[1], &[]]);
        let tree = g.loop_tree();
        expect![[r#"
            loops (2): [1, 2]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 2, 3: 2, 4: 1, 5: root]
            block depths: [0: 0, 1: 1, 2: 2, 3: 2, 4: 1, 5: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [4]
                blocks: [1, 2, 4, 3]
                exiting blocks: [1]; exit blocks: [5]
                block 1
                loop 2 (parent: 1, depth: 2)
                  latches: [3]
                  blocks: [2, 3]
                  exiting blocks: [2]; exit blocks: [4]
                  block 2
                  block 3
                block 4
              block 5
        "#]]
        .assert_eq(&print_tree(&(), &g, &tree));
        assert!(tree.contains(loop_at(&tree, 1), &3));
        assert!(!tree.contains(loop_at(&tree, 2), &4));
    }

    #[test]
    fn latch_within_a_subloop() {
        let g = graph(&[&[1], &[2, 4], &[3], &[2, 1], &[]]);
        expect![[r#"
            loops (2): [1, 2]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 2, 3: 2, 4: root]
            block depths: [0: 0, 1: 1, 2: 2, 3: 2, 4: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [3]
                blocks: [1, 2, 3]
                exiting blocks: [1]; exit blocks: [4]
                block 1
                loop 2 (parent: 1, depth: 2)
                  latches: [3]
                  blocks: [2, 3]
                  exiting blocks: [3]; exit blocks: [1]
                  block 2
                  block 3
              block 4
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn loop_ancestor_crosses_several_parents() {
        let g = graph(&[&[1], &[2, 7], &[3, 6], &[4, 5], &[3], &[2], &[1], &[]]);
        let tree = g.loop_tree();
        expect![[r#"
            loops (3): [1, 2, 3]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 2, 3: 3, 4: 3, 5: 2, 6: 1, 7: root]
            block depths: [0: 0, 1: 1, 2: 2, 3: 3, 4: 3, 5: 2, 6: 1, 7: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [6]
                blocks: [1, 2, 6, 3, 5, 4]
                exiting blocks: [1]; exit blocks: [7]
                block 1
                loop 2 (parent: 1, depth: 2)
                  latches: [5]
                  blocks: [2, 3, 5, 4]
                  exiting blocks: [2]; exit blocks: [6]
                  block 2
                  loop 3 (parent: 2, depth: 3)
                    latches: [4]
                    blocks: [3, 4]
                    exiting blocks: [3]; exit blocks: [5]
                    block 3
                    block 4
                  block 5
                block 6
              block 7
        "#]]
        .assert_eq(&print_tree(&(), &g, &tree));
        assert_eq!(tree.loop_ancestor(4), LoopTreeNode::Loop(loop_at(&tree, 1)));
    }

    #[test]
    fn sibling_loops_and_public_order() {
        let g = graph(&[&[1, 3], &[2, 5], &[1], &[4, 5], &[3], &[]]);
        expect![[r#"
            loops (2): [3, 1]
            top-level loops: [3, 1]
            block parents: [0: root, 1: 1, 2: 1, 3: 3, 4: 3, 5: root]
            block depths: [0: 0, 1: 1, 2: 1, 3: 1, 4: 1, 5: 0]
            root:
              block 0
              loop 3 (parent: root, depth: 1)
                latches: [4]
                blocks: [3, 4]
                exiting blocks: [3]; exit blocks: [5]
                block 3
                block 4
              loop 1 (parent: root, depth: 1)
                latches: [2]
                blocks: [1, 2]
                exiting blocks: [1]; exit blocks: [5]
                block 1
                block 2
              block 5
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn sibling_subloops_follow_rpo() {
        let g = graph(&[&[1], &[2, 4, 6], &[3, 6], &[2], &[5, 6], &[4], &[1, 7], &[]]);
        expect![[r#"
            loops (3): [1, 4, 2]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 2, 3: 2, 4: 4, 5: 4, 6: 1, 7: root]
            block depths: [0: 0, 1: 1, 2: 2, 3: 2, 4: 2, 5: 2, 6: 1, 7: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [6]
                blocks: [1, 4, 5, 2, 6, 3]
                exiting blocks: [6]; exit blocks: [7]
                block 1
                loop 4 (parent: 1, depth: 2)
                  latches: [5]
                  blocks: [4, 5]
                  exiting blocks: [4]; exit blocks: [6]
                  block 4
                  block 5
                loop 2 (parent: 1, depth: 2)
                  latches: [3]
                  blocks: [2, 3]
                  exiting blocks: [2]; exit blocks: [6]
                  block 2
                  block 3
                block 6
              block 7
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn rpo_puts_parents_before_children_despite_graph_order() {
        let g = graph(&[&[2], &[3], &[1, 5], &[1, 4], &[2], &[]]);
        expect![[r#"
            loops (2): [2, 1]
            top-level loops: [2]
            block parents: [0: root, 1: 1, 2: 2, 3: 1, 4: 2, 5: root]
            block depths: [0: 0, 1: 2, 2: 1, 3: 2, 4: 1, 5: 0]
            root:
              block 0
              loop 2 (parent: root, depth: 1)
                latches: [4]
                blocks: [2, 1, 3, 4]
                exiting blocks: [2]; exit blocks: [5]
                block 2
                loop 1 (parent: 2, depth: 2)
                  latches: [3]
                  blocks: [1, 3]
                  exiting blocks: [3]; exit blocks: [4]
                  block 1
                  block 3
                block 4
              block 5
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn unreachable_predecessors_and_cycles() {
        let g = graph(&[&[1], &[2, 3], &[1], &[], &[1, 2, 5], &[4]]);
        expect![[r#"
            loops (1): [1]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 1, 3: root, 4: root, 5: root]
            block depths: [0: 0, 1: 1, 2: 1, 3: 0, 4: 0, 5: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [2]
                blocks: [1, 2]
                exiting blocks: [1]; exit blocks: [3]
                block 1
                block 2
              block 3
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn cycles_with_multiple_entries() {
        let g = graph(&[&[1, 2], &[2], &[1]]);
        expect![[r#"
            loops (0): []
            top-level loops: []
            block parents: [0: root, 1: root, 2: root]
            block depths: [0: 0, 1: 0, 2: 0]
            root:
              block 0
              block 1
              block 2
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
        // Header 1 dominates 3, but does not dominate latch candidate 2.
        let g = graph(&[&[1, 2], &[3], &[1], &[1, 2]]);
        expect![[r#"
            loops (1): [1]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: root, 3: 1]
            block depths: [0: 0, 1: 1, 2: 0, 3: 1]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [3]
                blocks: [1, 3]
                exiting blocks: [3]; exit blocks: [2]
                block 1
                block 3
              block 2
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn infinite_loop_and_distinct_exits() {
        let g = graph(&[&[1], &[2], &[1]]);
        expect![[r#"
            loops (1): [1]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 1]
            block depths: [0: 0, 1: 1, 2: 1]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [2]
                blocks: [1, 2]
                exiting blocks: []; exit blocks: []
                block 1
                block 2
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
        let g = graph(&[&[1], &[2, 3, 3], &[1, 3], &[]]);
        expect![[r#"
            loops (1): [1]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 1, 3: root]
            block depths: [0: 0, 1: 1, 2: 1, 3: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [2]
                blocks: [1, 2]
                exiting blocks: [1, 2]; exit blocks: [3]
                block 1
                block 2
              block 3
        "#]]
        .assert_eq(&print_tree(&(), &g, &g.loop_tree()));
    }

    #[test]
    fn predecessor_order_does_not_change_the_tree() {
        let mut g = graph(&[&[1], &[2, 5], &[3, 4], &[2, 1], &[2, 1], &[]]);
        let expected = expect![[r#"
            loops (2): [1, 2]
            top-level loops: [1]
            block parents: [0: root, 1: 1, 2: 2, 3: 2, 4: 2, 5: root]
            block depths: [0: 0, 1: 1, 2: 2, 3: 2, 4: 2, 5: 0]
            root:
              block 0
              loop 1 (parent: root, depth: 1)
                latches: [4, 3]
                blocks: [1, 2, 4, 3]
                exiting blocks: [1]; exit blocks: [5]
                block 1
                loop 2 (parent: 1, depth: 2)
                  latches: [4, 3]
                  blocks: [2, 4, 3]
                  exiting blocks: [4, 3]; exit blocks: [1]
                  block 2
                  block 4
                  block 3
              block 5
        "#]];
        let first = print_tree(&(), &g, &g.loop_tree());
        for preds in &mut g.predecessors {
            preds.reverse();
        }
        assert_eq!(first, print_tree(&(), &g, &g.loop_tree()));
        expected.assert_eq(&first);
    }

    fn reachable_from(g: &TestGraph, start: usize, allowed: usize) -> usize {
        let mut seen = 0;
        let mut queue = vec![start];
        while let Some(node) = queue.pop() {
            let bit = 1 << node;
            if allowed & bit == 0 || seen & bit != 0 {
                continue;
            }
            seen |= bit;
            queue.extend(g.successors[node].iter().copied());
        }
        seen
    }

    /// Checks public list ordering against CFG reverse postorder.
    fn check_rpo_order(g: &TestGraph, tree: &LoopTree<TestGraph, ()>) {
        let dfs = DFSTraversal::new(&(), g);
        let rpo_position = |node| dfs.reverse_post_order_number(&node);
        let child_position = |node| match node {
            LoopTreeNode::Block(block) => rpo_position(block),
            LoopTreeNode::Loop(id) => rpo_position(tree.loop_header(id)),
        };
        let loops: ISet<_> = tree.loops().collect();
        assert!(
            loops
                .iter()
                .map(|&id| rpo_position(tree.loop_header(id)))
                .is_sorted()
        );
        assert!(tree.root_contents().map(&child_position).is_sorted());
        for &id in &loops {
            assert_eq!(tree.blocks(id).next(), Some(tree.loop_header(id)));
            assert!(tree.blocks(id).map(&rpo_position).is_sorted());
            assert!(tree.loop_contents(id).map(&child_position).is_sorted());
            assert!(tree.latches(id).map(&rpo_position).is_sorted());
            for child in tree.subloops(id) {
                assert!(loops.get_index_of(&id).unwrap() < loops.get_index_of(&child).unwrap());
            }
        }
    }

    /// Checks loop membership with forward reachability and list order with DFS.
    fn check_with_reachability(g: &TestGraph) {
        let tree = g.loop_tree();
        let all = (1 << g.successors.len()) - 1;
        let reachable = reachable_from(g, 0, all);
        let mut expected = Vec::new();
        for header in 0..g.successors.len() {
            if reachable & (1 << header) == 0 {
                continue;
            }
            // Removing the header makes its strictly dominated blocks unreachable.
            let dominated = reachable & !reachable_from(g, 0, all & !(1 << header));
            let forward = reachable_from(g, header, dominated);
            let body: Vec<_> = (0..g.successors.len())
                .filter(|&node| {
                    forward & (1 << node) != 0
                        && reachable_from(g, node, dominated) & (1 << header) != 0
                })
                .collect();
            if body.len() == 1 && !g.successors[header].contains(&header) {
                continue;
            }
            expected.push(header);
            let id = loop_at(&tree, header);
            let mut actual: Vec<_> = tree.blocks(id).collect();
            actual.sort_unstable();
            assert_eq!(actual, body, "Graph {:?}, header {header}", g.successors);
            let latches: Vec<_> = body
                .iter()
                .copied()
                .filter(|&node| g.successors[node].contains(&header))
                .collect();
            let mut actual_latches: Vec<_> = tree.latches(id).collect();
            actual_latches.sort_unstable();
            assert_eq!(actual_latches, latches);
        }
        let mut actual_headers: Vec<_> = tree.loops().map(|id| tree.loop_header(id)).collect();
        actual_headers.sort_unstable();
        assert_eq!(actual_headers, expected);
        // Every reachable block occurs exactly once as a tree leaf.
        let mut leaves = Vec::new();
        let mut queue: Vec<_> = tree.root_contents().collect();
        while let Some(node) = queue.pop() {
            match node {
                LoopTreeNode::Block(block) => leaves.push(block),
                LoopTreeNode::Loop(id) => queue.extend(tree.loop_contents(id)),
            }
        }
        leaves.sort_unstable();
        assert_eq!(
            leaves,
            (0..g.successors.len())
                .filter(|node| reachable & (1 << node) != 0)
                .collect::<Vec<_>>()
        );
        for id in tree.loops() {
            for child in tree.subloops(id) {
                assert_eq!(tree.loop_parent(child), Some(id));
                assert_eq!(tree.loop_depth(child), tree.loop_depth(id) + 1);
            }
        }
        check_rpo_order(g, &tree);
    }

    #[test]
    fn all_three_block_graphs_match_reachability() {
        for edges in 0usize..(1 << 9) {
            let successors = (0..3)
                .map(|node| {
                    (0..3)
                        .filter(|succ| edges & (1 << (node * 3 + succ)) != 0)
                        .collect()
                })
                .collect();
            check_with_reachability(&TestGraph::new(successors));
        }
    }

    #[test]
    fn four_block_graphs_match_reachability() {
        // The odd multiplier visits distinct edge masks, including sparse masks.
        for i in 0usize..1024 {
            let edges = i.wrapping_mul(40503) & 0xffff;
            let successors = (0..4)
                .map(|node| {
                    (0..4)
                        .filter(|succ| edges & (1 << (node * 4 + succ)) != 0)
                        .collect()
                })
                .collect();
            check_with_reachability(&TestGraph::new(successors));
        }
    }

    #[test]
    fn sampled_larger_graphs_match_reachability() {
        let mut state = 0x4d59_5df4_d0f3_3173u64;
        for size in [6, 7] {
            for sample in 0..1024 {
                let mut successors = vec![Vec::new(); size];
                for (node, succs) in successors.iter_mut().enumerate() {
                    for succ in 0..size {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        // Vary the edge density. Half the graphs have an entry path
                        // through every block; the others can have unreachable blocks.
                        if state & 7 < 1 + sample % 4 || (sample % 2 == 0 && succ == node + 1) {
                            succs.push(succ);
                        }
                    }
                }
                check_with_reachability(&TestGraph::new(successors));
            }
        }
        // Include a three-level nest in a cycle with multiple entries.
        check_with_reachability(&graph(&[
            &[1, 6],
            &[2],
            &[3, 5],
            &[4],
            &[3, 2],
            &[1, 6],
            &[1],
        ]));
    }

    #[pliron_op(
        name = "test.loop_tree_branch",
        format,
        interfaces = [IsTerminatorInterface],
        verifier = "succ",
    )]
    struct BranchOp;

    #[test]
    fn region_analysis_reuses_dominance_and_caches_the_tree() {
        let ctx = &mut Context::new();
        let ty = FunctionType::get(ctx, vec![], vec![]);
        let func = FuncOp::new(ctx, ident!("loop_test"), ty);
        let region = func.get_region(ctx);
        let entry = func.get_entry_block(ctx);
        let header = BasicBlock::new(ctx, Some(ident!("header")), vec![]);
        header.insert_at_back(region, ctx);
        let tail = BasicBlock::new(ctx, Some(ident!("tail")), vec![]);
        tail.insert_at_back(region, ctx);
        for (block, succ) in [(entry, header), (header, tail), (tail, header)] {
            Operation::new(
                ctx,
                BranchOp::get_concrete_op_info(),
                vec![],
                vec![],
                vec![succ],
                0,
            )
            .insert_at_back(block, ctx);
        }
        let mut analyses = AnalysisManager::default();
        analyses
            .compute_analysis::<LoopInfo>(func.get_operation(), ctx)
            .unwrap();
        let mut dom_info = analyses
            .try_get_analysis_mut::<DomInfo>(func.get_operation())
            .expect("LoopInfo must cache DomInfo");
        let mut info = analyses
            .try_get_analysis_mut::<LoopInfo>(func.get_operation())
            .unwrap();
        let first = info.get_loop_tree(ctx, region, &mut dom_info) as *const _;
        let tree = info.get_loop_tree(ctx, region, &mut dom_info);
        assert_eq!(first, tree as *const _);
        let expected = expect![[r#"
            loops (1): [header_block2v1]
            top-level loops: [header_block2v1]
            block parents: [entry_block1v1: root, header_block2v1: header_block2v1, tail_block3v1: header_block2v1]
            block depths: [entry_block1v1: 0, header_block2v1: 1, tail_block3v1: 1]
            root:
              block entry_block1v1
              loop header_block2v1 (parent: root, depth: 1)
                latches: [tail_block3v1]
                blocks: [header_block2v1, tail_block3v1]
                exiting blocks: []; exit blocks: []
                block header_block2v1
                block tail_block3v1
        "#]];
        let printed = print_tree(ctx, &region, tree);
        let mut direct_info = LoopInfo::default();
        assert_eq!(
            printed,
            print_tree(
                ctx,
                &region,
                direct_info.get_loop_tree(ctx, region, &mut dom_info),
            )
        );
        expected.assert_eq(&printed);
    }

    #[test]
    fn nested_region_uses_cached_dominance() {
        let ctx = &mut Context::new();
        let ty = FunctionType::get(ctx, vec![], vec![]);
        let outer = FuncOp::new(ctx, ident!("outer"), ty);
        let inner = FuncOp::new(ctx, ident!("inner"), ty);
        inner
            .get_operation()
            .insert_at_back(outer.get_entry_block(ctx), ctx);
        let region = inner.get_region(ctx);
        let entry = inner.get_entry_block(ctx);
        Operation::new(
            ctx,
            BranchOp::get_concrete_op_info(),
            vec![],
            vec![],
            vec![entry],
            0,
        )
        .insert_at_back(entry, ctx);
        let mut analyses = AnalysisManager::default();
        analyses
            .compute_analysis::<LoopInfo>(outer.get_operation(), ctx)
            .unwrap();
        let mut dom_info = analyses
            .try_get_analysis_mut::<DomInfo>(outer.get_operation())
            .unwrap();
        let dom_tree = dom_info.get_dom_tree(ctx, region) as *const _;
        let mut info = analyses
            .try_get_analysis_mut::<LoopInfo>(outer.get_operation())
            .unwrap();
        let tree = info.get_loop_tree(ctx, region, &mut dom_info);
        expect![[r#"
            loops (1): [entry_block2v1]
            top-level loops: [entry_block2v1]
            block parents: [entry_block2v1: entry_block2v1]
            block depths: [entry_block2v1: 1]
            root:
              loop entry_block2v1 (parent: root, depth: 1)
                latches: [entry_block2v1]
                blocks: [entry_block2v1]
                exiting blocks: []; exit blocks: []
                block entry_block2v1
        "#]]
        .assert_eq(&print_tree(ctx, &region, tree));
        assert_eq!(dom_tree, dom_info.get_dom_tree(ctx, region) as *const _);
    }
}
