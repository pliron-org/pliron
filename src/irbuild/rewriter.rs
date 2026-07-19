// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! [Rewriter] extends [Inserter] with more capabilities, such as replace and erase operations.

use alloc::{vec, vec::Vec};

use crate::{
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    graph::traversals::region::post_order,
    identifier::{Identifier, underscore},
    irbuild::{
        inserter::{BlockInsertionPoint, IRInserter, Inserter, OpInsertionPoint},
        listener::RewriteListener,
    },
    linked_list::{ContainsLinkedList, LinkedList},
    location::Located,
    op::Op,
    operation::Operation,
    region::Region,
    r#type::{TypeHandle, Typed},
    value::Value,
};

/// Rewriter interface for transformations.
pub trait Rewriter: Inserter {
    /// Replace an [Operation] (and delete it) with another operation.
    /// Results of the new operation must match the results of the old operation.
    fn replace_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>, new_op: Ptr<Operation>);

    /// Replace an [Operation] (and delete it) with a list of values.
    /// Results of the new operation must match the list of values.
    fn replace_operation_with_values(
        &mut self,
        ctx: &mut Context,
        op: Ptr<Operation>,
        new_values: Vec<Value>,
    );

    /// Replace all uses of a [Value] with another value.
    fn replace_value_uses_with(&mut self, ctx: &Context, old_value: Value, new_value: Value);

    /// Erase an [Operation]. The operation must have no uses.
    fn erase_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>);

    /// Erase a [BasicBlock]. The block must have no uses.
    fn erase_block(&mut self, ctx: &mut Context, block: Ptr<BasicBlock>);

    /// Erase a [Region]. Affects the index of all regions after it.
    fn erase_region(&mut self, ctx: &mut Context, region: Ptr<Region>);

    /// Unlink an [Operation] from its current position
    fn unlink_operation(&mut self, ctx: &Context, op: Ptr<Operation>);

    /// Unlink a [BasicBlock] from its current position
    fn unlink_block(&mut self, ctx: &Context, block: Ptr<BasicBlock>);

    /// Move an [Operation] to a new insertion point.
    fn move_operation(&mut self, ctx: &Context, op: Ptr<Operation>, new_point: OpInsertionPoint);

    /// Move a [BasicBlock] to a new insertion point.
    fn move_block(&mut self, ctx: &Context, block: Ptr<BasicBlock>, new_point: BlockInsertionPoint);

    /// Split a [BasicBlock] at the given position.
    fn split_block(
        &mut self,
        ctx: &mut Context,
        block: Ptr<BasicBlock>,
        position: OpInsertionPoint,
        new_block_label: Option<Identifier>,
    ) -> Ptr<BasicBlock>;

    /// Inline a [Region] into another [Region] at the given insertion point.
    /// The source region will be empty after this operation. The caller must
    /// take care of transferring control flow and arguments as necessary.
    fn inline_region(
        &mut self,
        ctx: &Context,
        src_region: Ptr<Region>,
        dest_insertion_point: BlockInsertionPoint,
    );

    /// Change the type of a [Value].
    fn set_value_type(&mut self, ctx: &Context, value: Value, new_type: TypeHandle);

    /// Has the IR been modified via this rewriter?
    fn is_modified(&self) -> bool;

    /// Mark that the IR has been modified via this rewriter.
    fn mark_modified(&mut self);
}

/// An implementation of the rewriter trait.
/// Use [DummyListener](super::listener::DummyListener) if no listener is needed.
pub struct IRRewriter<L: RewriteListener> {
    inserter: IRInserter<L>,
    config: IRRewriterConfig,
    _phantom: core::marker::PhantomData<L>,
}

impl<L: RewriteListener> Default for IRRewriter<L>
where
    L: Default,
{
    fn default() -> Self {
        Self {
            inserter: IRInserter::default(),
            config: IRRewriterConfig::default(),
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<L: RewriteListener> IRRewriter<L> {
    /// Get the configuration for this rewriter.
    pub fn get_config(&self) -> &IRRewriterConfig {
        &self.config
    }

    /// Get a mutable reference to the configuration for this rewriter.
    pub fn get_config_mut(&mut self) -> &mut IRRewriterConfig {
        &mut self.config
    }

    /// Sets the listener for insertion events.
    pub fn set_listener(&mut self, listener: L) {
        self.inserter.set_listener(listener);
    }

    /// Gets a reference to the listener for insertion events.
    pub fn get_listener(&self) -> &L {
        self.inserter.get_listener()
    }

    /// Gets a mutable reference to the listener for insertion events.
    pub fn get_listener_mut(&mut self) -> &mut L {
        self.inserter.get_listener_mut()
    }
}

impl<L: RewriteListener> Inserter for IRRewriter<L> {
    fn append_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        self.inserter.append_operation(ctx, operation)
    }

    fn append_op(&mut self, ctx: &Context, op: &dyn Op) {
        self.inserter.append_op(ctx, op)
    }

    fn insert_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        self.inserter.insert_operation(ctx, operation)
    }

    fn insert_op(&mut self, ctx: &Context, op: &dyn Op) {
        self.inserter.insert_op(ctx, op)
    }

    fn insert_block(
        &mut self,
        ctx: &Context,
        insertion_point: BlockInsertionPoint,
        block: Ptr<BasicBlock>,
    ) {
        self.inserter.insert_block(ctx, insertion_point, block)
    }

    fn create_block(
        &mut self,
        ctx: &mut Context,
        insertion_point: BlockInsertionPoint,
        label: Option<Identifier>,
        arg_types: Vec<TypeHandle>,
    ) -> Ptr<BasicBlock> {
        self.inserter
            .create_block(ctx, insertion_point, label, arg_types)
    }

    fn get_insertion_point(&self) -> OpInsertionPoint {
        self.inserter.get_insertion_point()
    }

    fn get_insertion_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        self.inserter.get_insertion_block(ctx)
    }

    fn set_insertion_point(&mut self, point: OpInsertionPoint) {
        self.inserter.set_insertion_point(point)
    }
}

/// Configuration for [IRRewriter].
#[derive(Clone)]
pub struct IRRewriterConfig {
    /// Whether to set the location of the new operation
    /// to the old operation when replacing an operation.
    pub set_loc_on_operation_replacement: bool,
}

impl Default for IRRewriterConfig {
    fn default() -> Self {
        Self {
            set_loc_on_operation_replacement: true,
        }
    }
}

impl<L: RewriteListener> Rewriter for IRRewriter<L> {
    fn replace_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>, new_op: Ptr<Operation>) {
        if op != new_op && self.config.set_loc_on_operation_replacement {
            new_op.deref_mut(ctx).set_loc(op.deref(ctx).loc());
        }
        let new_values = new_op.deref(ctx).results().collect();
        self.replace_operation_with_values(ctx, op, new_values);
    }

    fn replace_operation_with_values(
        &mut self,
        ctx: &mut Context,
        op: Ptr<Operation>,
        new_values: Vec<Value>,
    ) {
        assert!(
            op.deref(ctx).get_num_results() == new_values.len(),
            "Replacement values must match the number of results of the original operation."
        );

        // We need to collect the results first to avoid `RefCell` borrowing issues.
        let results: Vec<_> = op.deref(ctx).results().zip(new_values).collect();
        for (res, new_res) in results {
            self.get_listener_mut()
                .notify_value_use_replacement(ctx, res, new_res);
            res.replace_all_uses_with(ctx, &new_res);
        }
        self.erase_operation(ctx, op);
        self.mark_modified();
    }

    fn replace_value_uses_with(&mut self, ctx: &Context, old_value: Value, new_value: Value) {
        if old_value == new_value {
            return;
        }
        self.get_listener_mut()
            .notify_value_use_replacement(ctx, old_value, new_value);
        old_value.replace_all_uses_with(ctx, &new_value);
        self.mark_modified();
    }

    fn erase_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>) {
        // We don't rely on `Operation::erase` below to erase sub-entities
        // because we want the listener to be notified for each erased sub-entity.
        let regions = op.deref(ctx).regions().collect::<Vec<_>>();
        for region in regions.into_iter().rev() {
            self.erase_region(ctx, region);
        }

        self.get_listener_mut().notify_operation_erasure(ctx, op);

        Operation::erase(op, ctx);
        self.mark_modified();
    }

    fn erase_block(&mut self, ctx: &mut Context, block: Ptr<BasicBlock>) {
        // We don't rely on `BasicBlock::erase` below to erase sub-entities
        // because we want the listener to be notified for each erased sub-entity.
        let operations = block.deref(ctx).iter(ctx).collect::<Vec<_>>();
        // We erase operations in reverse order so that uses are erased before defs.
        for op in operations.into_iter().rev() {
            self.erase_operation(ctx, op);
        }

        self.get_listener_mut().notify_block_erasure(ctx, block);

        BasicBlock::erase(block, ctx);
        self.mark_modified();
    }

    fn erase_region(&mut self, ctx: &mut Context, region: Ptr<Region>) {
        // We don't rely on `Operation::erase_region` below to erase sub-entities
        // because we want the listener to be notified for each erased sub-entity.

        // We erase blocks in post-order so that uses are erased before defs.
        let blocks = post_order(ctx, &region);
        for block in blocks.iter().rev() {
            // We do not erase the block already because its predecessors
            // (which are its uses) haven't already been erased. We erase
            // only the operations now and the blocks later.
            let operations = block.deref(ctx).iter(ctx).collect::<Vec<_>>();
            // We erase operations in reverse order so that uses are erased before defs.
            for op in operations.into_iter().rev() {
                self.erase_operation(ctx, op);
            }
        }
        // Now erase the blocks.
        for block in blocks {
            self.erase_block(ctx, block);
        }

        self.get_listener_mut().notify_region_erasure(ctx, region);

        let index_in_parent = region.deref(ctx).find_index_in_parent(ctx);
        let parent_op = region.deref(ctx).get_parent_op();
        Operation::erase_region(parent_op, ctx, index_in_parent);
        self.mark_modified();
    }

    fn unlink_operation(&mut self, ctx: &Context, op: Ptr<Operation>) {
        self.get_listener_mut().notify_operation_unlinking(ctx, op);
        op.unlink(ctx);
        self.mark_modified();
    }

    fn unlink_block(&mut self, ctx: &Context, block: Ptr<BasicBlock>) {
        self.get_listener_mut().notify_block_unlinking(ctx, block);
        block.unlink(ctx);
        self.mark_modified();
    }

    fn move_operation(&mut self, ctx: &Context, op: Ptr<Operation>, new_point: OpInsertionPoint) {
        self.unlink_operation(ctx, op);
        ScopedRewriter::new(self, new_point).insert_operation(ctx, op);
    }

    fn move_block(
        &mut self,
        ctx: &Context,
        block: Ptr<BasicBlock>,
        new_point: BlockInsertionPoint,
    ) {
        self.unlink_block(ctx, block);
        self.insert_block(ctx, new_point, block);
    }

    fn split_block(
        &mut self,
        ctx: &mut Context,
        block: Ptr<BasicBlock>,
        position: OpInsertionPoint,
        new_block_label: Option<Identifier>,
    ) -> Ptr<BasicBlock> {
        // `create_block` below sets the insert point to the new block, so we save and restore it.
        let mut rewriter = ScopedRewriter::new(self, OpInsertionPoint::Unset);
        let label = new_block_label.or_else(|| {
            block
                .deref(ctx)
                .given_name(ctx)
                .map(|label| label + underscore() + "split".try_into().unwrap())
        });

        let new_block =
            rewriter.create_block(ctx, BlockInsertionPoint::AfterBlock(block), label, vec![]);
        let first_op_opt = match position {
            OpInsertionPoint::AtBlockStart(target_block) => {
                target_block.deref(ctx).iter(ctx).next()
            }
            OpInsertionPoint::AtBlockEnd(_target_block) => None,
            OpInsertionPoint::BeforeOperation(op) => Some(op),
            OpInsertionPoint::AfterOperation(op) => op.deref(ctx).get_next(),
            OpInsertionPoint::Unset => panic!("Cannot split block at unset insertion point."),
        };
        let mut current_op_opt = first_op_opt;
        while let Some(current_op) = current_op_opt {
            let next_op = current_op.deref(ctx).get_next();
            rewriter.move_operation(ctx, current_op, OpInsertionPoint::AtBlockEnd(new_block));
            current_op_opt = next_op;
        }
        new_block
    }

    fn inline_region(
        &mut self,
        ctx: &Context,
        src_region: Ptr<Region>,
        dest_insertion_point: BlockInsertionPoint,
    ) {
        assert!(
            src_region
                != dest_insertion_point
                    .get_insertion_region(ctx)
                    .expect("Insertion point itself is not in a Region"),
            "Cannot inline a region into itself."
        );
        let blocks: Vec<_> = src_region.deref(ctx).iter(ctx).collect();
        let mut insertion_pt = dest_insertion_point;
        for block in blocks {
            self.move_block(ctx, block, insertion_pt);
            insertion_pt = BlockInsertionPoint::AfterBlock(block);
        }
    }

    fn set_value_type(&mut self, ctx: &Context, value: Value, new_type: TypeHandle) {
        let old_type = value.get_type(ctx);
        if old_type == new_type {
            return;
        }
        self.get_listener_mut()
            .notify_value_type_change(ctx, value, old_type, new_type);
        value.set_type(ctx, new_type);
        self.mark_modified();
    }

    fn is_modified(&self) -> bool {
        self.inserter.is_modified()
    }

    fn mark_modified(&mut self) {
        self.inserter.mark_modified();
    }
}

/// A scoped rewriter that sets the insertion point and configuration for the duration of its lifetime.
/// On drop, it restores the previous insertion point and configuration.
/// Implements [Inserter] and [Rewriter] by forwarding calls to the wrapped rewriter.
/// ```rust
/// # use pliron::{context::Context,
/// #   builtin::{ops::ModuleOp, op_interfaces::SingleBlockRegionInterface}};
/// # use pliron::irbuild::{rewriter::{IRRewriter, ScopedRewriter},
/// #   listener::DummyListener,
/// #   inserter::{Inserter, OpInsertionPoint}};
/// let ctx = &mut Context::new();
/// let module = ModuleOp::new(ctx, "test_module".try_into().unwrap());
/// let mut rewriter = IRRewriter::<DummyListener>::default();
/// rewriter.set_insertion_point(OpInsertionPoint::AtBlockEnd(module.get_body(ctx, 0)));
/// {
///     // We can create a scoped rewriter with a different insertion point,
///     // and it will restore the original insertion point after this block.
///     let mut scoped_rewriter = ScopedRewriter::new(&mut rewriter, OpInsertionPoint::Unset);
///     assert!(!scoped_rewriter.get_insertion_point().is_set());
/// }
/// assert!(rewriter.get_insertion_point().is_set());
/// ```
pub struct ScopedRewriter<'a> {
    rewriter: &'a mut dyn Rewriter,
    prev_insertion_point: OpInsertionPoint,
}

impl<'a> ScopedRewriter<'a> {
    pub fn new(rewriter: &'a mut dyn Rewriter, insertion_point: OpInsertionPoint) -> Self {
        let prev_insertion_point = rewriter.get_insertion_point();
        rewriter.set_insertion_point(insertion_point);
        Self {
            rewriter,
            prev_insertion_point,
        }
    }
}

impl<'a> Drop for ScopedRewriter<'a> {
    fn drop(&mut self) {
        self.rewriter.set_insertion_point(self.prev_insertion_point);
    }
}

impl<'a> Inserter for ScopedRewriter<'a> {
    fn append_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        self.rewriter.append_operation(ctx, operation)
    }

    fn append_op(&mut self, ctx: &Context, op: &dyn Op) {
        self.rewriter.append_op(ctx, op)
    }

    fn insert_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        self.rewriter.insert_operation(ctx, operation)
    }

    fn insert_op(&mut self, ctx: &Context, op: &dyn Op) {
        self.rewriter.insert_op(ctx, op)
    }

    fn insert_block(
        &mut self,
        ctx: &Context,
        insertion_point: BlockInsertionPoint,
        block: Ptr<BasicBlock>,
    ) {
        self.rewriter.insert_block(ctx, insertion_point, block)
    }

    fn create_block(
        &mut self,
        ctx: &mut Context,
        insertion_point: BlockInsertionPoint,
        label: Option<Identifier>,
        arg_types: Vec<TypeHandle>,
    ) -> Ptr<BasicBlock> {
        self.rewriter
            .create_block(ctx, insertion_point, label, arg_types)
    }

    fn get_insertion_point(&self) -> OpInsertionPoint {
        self.rewriter.get_insertion_point()
    }

    fn get_insertion_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        self.rewriter.get_insertion_block(ctx)
    }

    fn set_insertion_point(&mut self, point: OpInsertionPoint) {
        self.rewriter.set_insertion_point(point)
    }
}

impl<'a> Rewriter for ScopedRewriter<'a> {
    fn replace_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>, new_op: Ptr<Operation>) {
        self.rewriter.replace_operation(ctx, op, new_op)
    }

    fn replace_operation_with_values(
        &mut self,
        ctx: &mut Context,
        op: Ptr<Operation>,
        new_values: Vec<Value>,
    ) {
        self.rewriter
            .replace_operation_with_values(ctx, op, new_values)
    }

    fn replace_value_uses_with(&mut self, ctx: &Context, old_value: Value, new_value: Value) {
        self.rewriter
            .replace_value_uses_with(ctx, old_value, new_value)
    }

    fn erase_operation(&mut self, ctx: &mut Context, op: Ptr<Operation>) {
        self.rewriter.erase_operation(ctx, op)
    }

    fn erase_block(&mut self, ctx: &mut Context, block: Ptr<BasicBlock>) {
        self.rewriter.erase_block(ctx, block)
    }

    fn erase_region(&mut self, ctx: &mut Context, region: Ptr<Region>) {
        self.rewriter.erase_region(ctx, region)
    }

    fn unlink_operation(&mut self, ctx: &Context, op: Ptr<Operation>) {
        self.rewriter.unlink_operation(ctx, op)
    }

    fn unlink_block(&mut self, ctx: &Context, block: Ptr<BasicBlock>) {
        self.rewriter.unlink_block(ctx, block)
    }

    fn move_operation(&mut self, ctx: &Context, op: Ptr<Operation>, new_point: OpInsertionPoint) {
        self.rewriter.move_operation(ctx, op, new_point)
    }

    fn move_block(
        &mut self,
        ctx: &Context,
        block: Ptr<BasicBlock>,
        new_point: BlockInsertionPoint,
    ) {
        self.rewriter.move_block(ctx, block, new_point)
    }

    fn split_block(
        &mut self,
        ctx: &mut Context,
        block: Ptr<BasicBlock>,
        position: OpInsertionPoint,
        new_block_label: Option<Identifier>,
    ) -> Ptr<BasicBlock> {
        self.rewriter
            .split_block(ctx, block, position, new_block_label)
    }

    fn inline_region(
        &mut self,
        ctx: &Context,
        src_region: Ptr<Region>,
        dest_insertion_point: BlockInsertionPoint,
    ) {
        self.rewriter
            .inline_region(ctx, src_region, dest_insertion_point)
    }

    fn set_value_type(&mut self, ctx: &Context, value: Value, new_type: TypeHandle) {
        self.rewriter.set_value_type(ctx, value, new_type)
    }

    fn is_modified(&self) -> bool {
        self.rewriter.is_modified()
    }

    fn mark_modified(&mut self) {
        self.rewriter.mark_modified()
    }
}
