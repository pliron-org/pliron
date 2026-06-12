//! Walk the IR graph.
//!
//! At each node:
//! - Call a callback to process the node.
//! - Walk over the children.
//!
//!  The relative orders of performing the above is configurable.
//!
//! Throughout this module, only parent-child relations are considered,
//!   i.e., operations->regions, region->blocks and block->operations.
//!   Successor/predecessor relations b/w blocks (control-flow-graph) is out-of-scope.
//!
//! Four types of walkers are provided. The ones in [interruptible] can be interrupted
//! during the walk, or have the walk over unvisited children skipped. The ones in
//! [uninterruptible] cannot be interrupted and always complete the full walk.
//! Within each, the `mutable` variant take a mutable reference to [Context],
//! while the `immutable` variant take an immutable reference.
//!
//! Care must be taken if modifications are made to the graph during the walk.
//! Safety:
//!    Node deletions: A node can be deleted from its parent only when the
//!        parent is visited in a PostOrder walk, after all children are processed.
//!        This ensures that deleted nodes aren't visited.
//!    Node additions: A node can be inserted into its parent only when the
//!        parent is visited in a PreOrder walk, before any child is processed.
//!        This ensures that the new node gets visited.
//!    Other graph modifications such as changing the order b/w siblings are not safe.
//!        (i.e., doing so may lead to multiple visits of the same node or
//!         some nodes remaining unvisited).
//!    Non-graph modifications are safe.

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    operation::Operation,
    region::Region,
};

#[derive(Clone, Copy, Debug)]
/// Should the callback for a node be called before or after processing its children.
pub enum Order {
    /// Call the callback for a node before processing its children.
    PreOrder,
    /// Call the callback for a node after processing its children.
    PostOrder,
}

#[derive(Clone, Copy, Debug)]
/// The order in which children nodes are walked over.
pub enum Direction {
    /// Process nodes in lexicographic order.
    Forward,
    /// Process nodes in reverse lexicographic order.
    Reverse,
}

/// For an IR walk, specify the order of visiting different kinds of nodes.
pub struct WalkConfig {
    /// Order of processing [Region]s in an [Operation].
    pub regions_in_op: (Order, Direction),
    /// Order of processing [Operation]s in a [BasicBlock]
    pub ops_in_block: (Order, Direction),
    ///Order of processing [BasicBlock]s in a [Region]
    pub blocks_in_region: (Order, Direction),
}

/// The walker calls the callback for any of these IR entities.
pub enum IRNode {
    Operation(Ptr<Operation>),
    BasicBlock(Ptr<BasicBlock>),
    Region(Ptr<Region>),
}

/// Walkers that can be interrupted.
pub mod interruptible {
    use core::ops::ControlFlow;

    use crate::{
        basic_block::BasicBlock,
        context::{Context, Ptr},
        linked_list::{ContainsLinkedList, LinkedList},
        operation::Operation,
        region::Region,
    };

    use super::*;

    /// How to continue an ongoing walk.
    pub enum WalkContinue {
        /// The walk will simply continue.
        Advance,
        /// The walk of the current operation, region or block and their nested elements
        /// that haven't been visited already will be skipped and will continue with the
        /// next operation, region or block.
        Skip,
    }

    impl WalkContinue {
        /// Is this [Self::Advance]?
        pub fn is_advance(&self) -> bool {
            matches!(self, Self::Advance)
        }

        /// Is this [Self::Skip]?
        pub fn is_skip(&self) -> bool {
            matches!(self, Self::Skip)
        }
    }

    /// Break or continue an ongoing walk. Breaks can be short-circuited using `?`.
    /// See [mlir::WalkResult](https://mlir.llvm.org/doxygen/classmlir_1_1WalkResult.html).
    pub type WalkResult<B> = ControlFlow<B, WalkContinue>;

    /// Utility to build WalkResult::Break
    pub fn walk_break<B>(b: B) -> WalkResult<B> {
        WalkResult::Break(b)
    }

    /// Utility to build WalkResult::Continue(WalkContinue::Advance)
    pub fn walk_advance<B>() -> WalkResult<B> {
        WalkResult::Continue(WalkContinue::Advance)
    }

    /// Utility to build WalkResult::Continue(WalkContinue::Skip)
    pub fn walk_skip<B>() -> WalkResult<B> {
        WalkResult::Continue(WalkContinue::Skip)
    }

    /// Interruptible walkers that take a mutable reference to [Context].
    pub mod mutable {
        use super::*;
        /// The walker functions calls back to functions of this type.
        pub type WalkerCallback<State, B> = fn(&mut Context, &mut State, IRNode) -> WalkResult<B>;

        /// Visit an [Operation] and walk its children [Region]s.
        pub fn walk_op<State, B>(
            ctx: &mut Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Operation>,
            callback: WalkerCallback<State, B>,
        ) -> WalkResult<B> {
            if let Order::PreOrder = config.regions_in_op.0 {
                let c = callback(ctx, state, IRNode::Operation(root))?;
                if c.is_skip() {
                    return walk_advance();
                }
            }

            let num_regions = root.deref(ctx).num_regions();
            match config.regions_in_op.1 {
                Direction::Forward => {
                    for reg_idx in 0..num_regions {
                        let region = {
                            let root_ref = &*root.deref(ctx);
                            assert!(
                                reg_idx < root_ref.num_regions(),
                                "Region deleted during walk"
                            );
                            root_ref.get_region(reg_idx)
                        };
                        walk_region(ctx, state, config, region, callback)?;
                    }
                }
                Direction::Reverse => {
                    for i in (0..num_regions).rev() {
                        let region = {
                            let root_ref = &*root.deref(ctx);
                            assert!(i < root_ref.num_regions(), "Region deleted during walk");
                            root_ref.get_region(i)
                        };
                        walk_region(ctx, state, config, region, callback)?;
                    }
                }
            }

            if let Order::PostOrder = config.regions_in_op.0 {
                callback(ctx, state, IRNode::Operation(root))?;
            }

            walk_advance()
        }

        /// Visit a [Region] and walk its children [BasicBlock]s.
        pub fn walk_region<State, B>(
            ctx: &mut Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Region>,
            callback: WalkerCallback<State, B>,
        ) -> WalkResult<B> {
            if let Order::PreOrder = config.blocks_in_region.0 {
                let c = callback(ctx, state, IRNode::Region(root))?;
                if c.is_skip() {
                    return walk_advance();
                }
            }

            match config.blocks_in_region.1 {
                Direction::Forward => {
                    let mut cur_block_opt = root.deref(ctx).get_head();
                    while let Some(cur_block) = cur_block_opt {
                        walk_block(ctx, state, config, cur_block, callback)?;
                        cur_block_opt = cur_block.deref(ctx).get_next();
                    }
                }
                Direction::Reverse => {
                    let mut cur_block_opt = root.deref(ctx).get_tail();
                    while let Some(cur_block) = cur_block_opt {
                        walk_block(ctx, state, config, cur_block, callback)?;
                        cur_block_opt = cur_block.deref(ctx).get_prev();
                    }
                }
            }

            if let Order::PostOrder = config.blocks_in_region.0 {
                callback(ctx, state, IRNode::Region(root))?;
            }

            walk_advance()
        }

        /// Visit a [BasicBlock] and walk its children [Operation]s.
        pub fn walk_block<State, B>(
            ctx: &mut Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<BasicBlock>,
            callback: WalkerCallback<State, B>,
        ) -> WalkResult<B> {
            if let Order::PreOrder = config.ops_in_block.0 {
                let c = callback(ctx, state, IRNode::BasicBlock(root))?;
                if c.is_skip() {
                    return walk_advance();
                }
            }

            match config.ops_in_block.1 {
                Direction::Forward => {
                    let mut cur_op_opt = root.deref(ctx).get_head();
                    while let Some(cur_op) = cur_op_opt {
                        walk_op(ctx, state, config, cur_op, callback)?;
                        cur_op_opt = cur_op.deref(ctx).get_next();
                    }
                }
                Direction::Reverse => {
                    let mut cur_op_opt = root.deref(ctx).get_tail();
                    while let Some(cur_op) = cur_op_opt {
                        walk_op(ctx, state, config, cur_op, callback)?;
                        cur_op_opt = cur_op.deref(ctx).get_prev();
                    }
                }
            }

            if let Order::PostOrder = config.ops_in_block.0 {
                callback(ctx, state, IRNode::BasicBlock(root))?;
            }

            walk_advance()
        }
    }

    /// Interruptible walkers that take an immutable reference to [Context].
    pub mod immutable {
        use super::*;
        /// The walker functions calls back to functions of this type.
        pub type WalkerCallback<State, B> = fn(&Context, &mut State, IRNode) -> WalkResult<B>;

        /// Visit an [Operation] and walk its children [Region]s.
        pub fn walk_op<State, B>(
            ctx: &Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Operation>,
            callback: WalkerCallback<State, B>,
        ) -> WalkResult<B> {
            if let Order::PreOrder = config.regions_in_op.0 {
                let c = callback(ctx, state, IRNode::Operation(root))?;
                if c.is_skip() {
                    return walk_advance();
                }
            }

            let num_regions = root.deref(ctx).num_regions();
            match config.regions_in_op.1 {
                Direction::Forward => {
                    for reg_idx in 0..num_regions {
                        let region = {
                            let root_ref = &*root.deref(ctx);
                            assert!(
                                reg_idx < root_ref.num_regions(),
                                "Region deleted during walk"
                            );
                            root_ref.get_region(reg_idx)
                        };
                        walk_region(ctx, state, config, region, callback)?;
                    }
                }
                Direction::Reverse => {
                    for i in (0..num_regions).rev() {
                        let region = {
                            let root_ref = &*root.deref(ctx);
                            assert!(i < root_ref.num_regions(), "Region deleted during walk");
                            root_ref.get_region(i)
                        };
                        walk_region(ctx, state, config, region, callback)?;
                    }
                }
            }

            if let Order::PostOrder = config.regions_in_op.0 {
                callback(ctx, state, IRNode::Operation(root))?;
            }

            walk_advance()
        }

        /// Visit a [Region] and walk its children [BasicBlock]s.
        pub fn walk_region<State, B>(
            ctx: &Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Region>,
            callback: WalkerCallback<State, B>,
        ) -> WalkResult<B> {
            if let Order::PreOrder = config.blocks_in_region.0 {
                let c = callback(ctx, state, IRNode::Region(root))?;
                if c.is_skip() {
                    return walk_advance();
                }
            }

            match config.blocks_in_region.1 {
                Direction::Forward => {
                    let mut cur_block_opt = root.deref(ctx).get_head();
                    while let Some(cur_block) = cur_block_opt {
                        walk_block(ctx, state, config, cur_block, callback)?;
                        cur_block_opt = cur_block.deref(ctx).get_next();
                    }
                }
                Direction::Reverse => {
                    let mut cur_block_opt = root.deref(ctx).get_tail();
                    while let Some(cur_block) = cur_block_opt {
                        walk_block(ctx, state, config, cur_block, callback)?;
                        cur_block_opt = cur_block.deref(ctx).get_prev();
                    }
                }
            }

            if let Order::PostOrder = config.blocks_in_region.0 {
                callback(ctx, state, IRNode::Region(root))?;
            }

            walk_advance()
        }

        /// Visit a [BasicBlock] and walk its children [Operation]s.
        pub fn walk_block<State, B>(
            ctx: &Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<BasicBlock>,
            callback: WalkerCallback<State, B>,
        ) -> WalkResult<B> {
            if let Order::PreOrder = config.ops_in_block.0 {
                let c = callback(ctx, state, IRNode::BasicBlock(root))?;
                if c.is_skip() {
                    return walk_advance();
                }
            }

            match config.ops_in_block.1 {
                Direction::Forward => {
                    let mut cur_op_opt = root.deref(ctx).get_head();
                    while let Some(cur_op) = cur_op_opt {
                        walk_op(ctx, state, config, cur_op, callback)?;
                        cur_op_opt = cur_op.deref(ctx).get_next();
                    }
                }
                Direction::Reverse => {
                    let mut cur_op_opt = root.deref(ctx).get_tail();
                    while let Some(cur_op) = cur_op_opt {
                        walk_op(ctx, state, config, cur_op, callback)?;
                        cur_op_opt = cur_op.deref(ctx).get_prev();
                    }
                }
            }

            if let Order::PostOrder = config.ops_in_block.0 {
                callback(ctx, state, IRNode::BasicBlock(root))?;
            }

            walk_advance()
        }
    }
}

/// Walkers that always complete the full walk.
pub mod uninterruptible {
    use super::*;

    /// Uninterruptible walkers that take a mutable reference to [Context].
    pub mod mutable {
        use super::*;
        /// The walker functions calls back to functions of this type.
        pub type WalkerCallback<State> = fn(&mut Context, &mut State, IRNode);

        /// Visit an [Operation] and walk its children [Region]s.
        pub fn walk_op<State>(
            ctx: &mut Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Operation>,
            callback: WalkerCallback<State>,
        ) {
            struct S<'a, State> {
                state: &'a mut State,
                callback: WalkerCallback<State>,
            }

            let mut s = S { state, callback };

            let _ = interruptible::mutable::walk_op(ctx, &mut s, config, root, |ctx, s, node| {
                (s.callback)(ctx, s.state, node);
                interruptible::walk_advance::<()>()
            });
        }

        /// Visit a [Region] and walk its children [BasicBlock]s.
        pub fn walk_region<State>(
            ctx: &mut Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Region>,
            callback: WalkerCallback<State>,
        ) {
            struct S<'a, State> {
                state: &'a mut State,
                callback: WalkerCallback<State>,
            }

            let mut s = S { state, callback };

            let _ =
                interruptible::mutable::walk_region(ctx, &mut s, config, root, |ctx, s, node| {
                    (s.callback)(ctx, s.state, node);
                    interruptible::walk_advance::<()>()
                });
        }

        /// Visit a [BasicBlock] and walk its children [Operation]s.
        pub fn walk_block<State>(
            ctx: &mut Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<BasicBlock>,
            callback: WalkerCallback<State>,
        ) {
            struct S<'a, State> {
                state: &'a mut State,
                callback: WalkerCallback<State>,
            }

            let mut s = S { state, callback };

            let _ =
                interruptible::mutable::walk_block(ctx, &mut s, config, root, |ctx, s, node| {
                    (s.callback)(ctx, s.state, node);
                    interruptible::walk_advance::<()>()
                });
        }
    }

    /// Uninterruptible walkers that take an immutable reference to [Context].
    pub mod immutable {
        use super::*;
        /// The walker functions calls back to functions of this type.
        pub type WalkerCallback<State> = fn(&Context, &mut State, IRNode);

        /// Visit an [Operation] and walk its children [Region]s.
        pub fn walk_op<State>(
            ctx: &Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Operation>,
            callback: WalkerCallback<State>,
        ) {
            struct S<'a, State> {
                state: &'a mut State,
                callback: WalkerCallback<State>,
            }

            let mut s = S { state, callback };

            let _ = interruptible::immutable::walk_op(ctx, &mut s, config, root, |ctx, s, node| {
                (s.callback)(ctx, s.state, node);
                interruptible::walk_advance::<()>()
            });
        }

        /// Visit a [Region] and walk its children [BasicBlock]s.
        pub fn walk_region<State>(
            ctx: &Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<Region>,
            callback: WalkerCallback<State>,
        ) {
            struct S<'a, State> {
                state: &'a mut State,
                callback: WalkerCallback<State>,
            }

            let mut s = S { state, callback };

            let _ =
                interruptible::immutable::walk_region(ctx, &mut s, config, root, |ctx, s, node| {
                    (s.callback)(ctx, s.state, node);
                    interruptible::walk_advance::<()>()
                });
        }

        /// Visit a [BasicBlock] and walk its children [Operation]s.
        pub fn walk_block<State>(
            ctx: &Context,
            state: &mut State,
            config: &WalkConfig,
            root: Ptr<BasicBlock>,
            callback: WalkerCallback<State>,
        ) {
            struct S<'a, State> {
                state: &'a mut State,
                callback: WalkerCallback<State>,
            }

            let mut s = S { state, callback };

            let _ =
                interruptible::immutable::walk_block(ctx, &mut s, config, root, |ctx, s, node| {
                    (s.callback)(ctx, s.state, node);
                    interruptible::walk_advance::<()>()
                });
        }
    }
}

/// A preset config with [Order::PreOrder] and [Direction::Forward]
/// for all IR node kinds.
pub const WALKCONFIG_PREORDER_FORWARD: WalkConfig = WalkConfig {
    regions_in_op: (Order::PreOrder, Direction::Forward),
    ops_in_block: (Order::PreOrder, Direction::Forward),
    blocks_in_region: (Order::PreOrder, Direction::Forward),
};

/// A preset config with [Order::PostOrder] and [Direction::Forward]
/// for all IR node kinds.
pub const WALKCONFIG_POSTORDER_FORWARD: WalkConfig = WalkConfig {
    regions_in_op: (Order::PostOrder, Direction::Forward),
    ops_in_block: (Order::PostOrder, Direction::Forward),
    blocks_in_region: (Order::PostOrder, Direction::Forward),
};

/// A preset config with [Order::PreOrder] and [Direction::Reverse]
/// for all IR node kinds.
pub const WALKCONFIG_PREORDER_REVERSE: WalkConfig = WalkConfig {
    regions_in_op: (Order::PreOrder, Direction::Reverse),
    ops_in_block: (Order::PreOrder, Direction::Reverse),
    blocks_in_region: (Order::PreOrder, Direction::Reverse),
};

/// A preset config with [Order::PostOrder] and [Direction::Reverse]
/// for all IR node kinds.
pub const WALKCONFIG_POSTORDER_REVERSE: WalkConfig = WalkConfig {
    regions_in_op: (Order::PostOrder, Direction::Reverse),
    ops_in_block: (Order::PostOrder, Direction::Reverse),
    blocks_in_region: (Order::PostOrder, Direction::Reverse),
};
