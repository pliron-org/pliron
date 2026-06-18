//! Listen to IR building / rewriting events.

use alloc::vec::Vec;

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    irbuild::inserter::{BlockInsertionPoint, OpInsertionPoint},
    linked_list::{ContainsLinkedList as _, LinkedList},
    operation::Operation,
    region::Region,
    r#type::{TypeHandle, Typed},
    value::Value,
};

/// Listener interface for insertion events.
pub trait InsertionListener: Default {
    /// Notify that an operation has been inserted.
    fn notify_operation_inserted(&mut self, ctx: &Context, operation: Ptr<Operation>);
    /// Notify that a block has been inserted.
    fn notify_block_inserted(&mut self, ctx: &Context, block: Ptr<BasicBlock>);
}

/// Listener interface for rewrite events.
pub trait RewriteListener: InsertionListener {
    /// Notify that an operation is about to be erased.
    fn notify_operation_erasure(&mut self, ctx: &Context, op: Ptr<Operation>);
    /// Notify that all uses of a value are about to be replaced.
    fn notify_value_use_replacement(&mut self, ctx: &Context, old_value: Value, new_value: Value);
    /// Notify that a value type changed.
    fn notify_value_type_change(
        &mut self,
        ctx: &Context,
        value: Value,
        old_type: TypeHandle,
        new_type: TypeHandle,
    );
    /// Notify that a block is about to be erased.
    fn notify_block_erasure(&mut self, ctx: &Context, block: Ptr<BasicBlock>);
    /// Notify that a region is about to be erased.
    fn notify_region_erasure(&mut self, ctx: &Context, region: Ptr<Region>);
    /// Notify that an operation is about to be unlinked.
    fn notify_operation_unlinking(&mut self, ctx: &Context, op: Ptr<Operation>);
    /// Notify that a block is about to be unlinked.
    fn notify_block_unlinking(&mut self, ctx: &Context, block: Ptr<BasicBlock>);
}

/// A listener that doesn't do anything on being notified.
#[derive(Default)]
pub struct DummyListener;
impl InsertionListener for DummyListener {
    fn notify_operation_inserted(&mut self, _ctx: &Context, _operation: Ptr<Operation>) {}
    fn notify_block_inserted(&mut self, _ctx: &Context, _block: Ptr<BasicBlock>) {}
}
impl RewriteListener for DummyListener {
    fn notify_operation_erasure(&mut self, _ctx: &Context, _op: Ptr<Operation>) {}
    fn notify_value_use_replacement(
        &mut self,
        _ctx: &Context,
        _old_value: Value,
        _new_value: Value,
    ) {
    }
    fn notify_block_erasure(&mut self, _ctx: &Context, _block: Ptr<BasicBlock>) {}
    fn notify_region_erasure(&mut self, _ctx: &Context, _region: Ptr<Region>) {}
    fn notify_operation_unlinking(&mut self, _ctx: &Context, _op: Ptr<Operation>) {}
    fn notify_block_unlinking(&mut self, _ctx: &Context, _block: Ptr<BasicBlock>) {}
    fn notify_value_type_change(
        &mut self,
        _ctx: &Context,
        _value: Value,
        _old_type: TypeHandle,
        _new_type: TypeHandle,
    ) {
    }
}

/// Events recorded by the [Recorder] listener.
#[derive(Debug)]
pub enum RecorderEvent {
    InsertedOperation(Ptr<Operation>),
    InsertedBlock(Ptr<BasicBlock>),
    ErasedOperation(Ptr<Operation>),
    ReplacedValueUses {
        old_value: Value,
        old_type: TypeHandle,
        new_value: Value,
    },
    ValueTypeChanged {
        value: Value,
        old_type: TypeHandle,
        new_type: TypeHandle,
    },
    ErasedBlock(Ptr<BasicBlock>),
    ErasedRegion(Ptr<Region>),
    UnlinkedOperation(Ptr<Operation>, OpInsertionPoint),
    UnlinkedBlock(Ptr<BasicBlock>, BlockInsertionPoint),
}

/// A listener that records events in the order they occur.
#[derive(Default)]
pub struct Recorder {
    pub events: Vec<RecorderEvent>,
}

impl InsertionListener for Recorder {
    fn notify_operation_inserted(&mut self, _ctx: &Context, operation: Ptr<Operation>) {
        self.events
            .push(RecorderEvent::InsertedOperation(operation));
    }

    fn notify_block_inserted(&mut self, _ctx: &Context, block: Ptr<BasicBlock>) {
        self.events.push(RecorderEvent::InsertedBlock(block));
    }
}

impl RewriteListener for Recorder {
    fn notify_operation_erasure(&mut self, _ctx: &Context, op: Ptr<Operation>) {
        self.events.push(RecorderEvent::ErasedOperation(op));
    }

    fn notify_value_use_replacement(&mut self, ctx: &Context, old_value: Value, new_value: Value) {
        self.events.push(RecorderEvent::ReplacedValueUses {
            old_value,
            old_type: old_value.get_type(ctx),
            new_value,
        });
    }

    fn notify_block_erasure(&mut self, _ctx: &Context, block: Ptr<BasicBlock>) {
        self.events.push(RecorderEvent::ErasedBlock(block));
    }

    fn notify_value_type_change(
        &mut self,
        _ctx: &Context,
        value: Value,
        old_type: TypeHandle,
        new_type: TypeHandle,
    ) {
        self.events.push(RecorderEvent::ValueTypeChanged {
            value,
            old_type,
            new_type,
        });
    }

    fn notify_region_erasure(&mut self, _ctx: &Context, region: Ptr<Region>) {
        self.events.push(RecorderEvent::ErasedRegion(region));
    }

    fn notify_operation_unlinking(&mut self, ctx: &Context, op: Ptr<Operation>) {
        let previous_position = {
            let operation_ref = op.deref(ctx);
            let parent_ptr = operation_ref
                .get_parent_block()
                .expect("Operation must have a parent block if linked");
            let parent = parent_ptr.deref(ctx);
            if let Some(head) = parent.get_head()
                && head == op
            {
                OpInsertionPoint::AtBlockStart(parent_ptr)
            } else if let Some(tail) = parent.get_tail()
                && tail == op
            {
                OpInsertionPoint::AtBlockEnd(parent_ptr)
            } else {
                let prev_op = operation_ref.get_prev().expect(
                    "Operation must have a previous operation if linked and is not the head",
                );
                OpInsertionPoint::AfterOperation(prev_op)
            }
        };
        self.events
            .push(RecorderEvent::UnlinkedOperation(op, previous_position));
    }

    fn notify_block_unlinking(&mut self, _ctx: &Context, block: Ptr<BasicBlock>) {
        let previous_position = {
            let block_ref = block.deref(_ctx);
            let parent_region_ptr = block_ref
                .get_parent_region()
                .expect("Block must have a parent region if linked");
            let parent_region = parent_region_ptr.deref(_ctx);
            if let Some(head) = parent_region.get_head()
                && head == block
            {
                BlockInsertionPoint::AtRegionStart(parent_region_ptr)
            } else if let Some(tail) = parent_region.get_tail()
                && tail == block
            {
                BlockInsertionPoint::AtRegionEnd(parent_region_ptr)
            } else {
                let prev_block = block_ref
                    .get_prev()
                    .expect("Block must have a previous block if linked and is not the head");
                BlockInsertionPoint::AfterBlock(prev_block)
            }
        };
        self.events
            .push(RecorderEvent::UnlinkedBlock(block, previous_position));
    }
}

impl Recorder {
    /// Clear all recorded events.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Get the number of recorded events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if there are no recorded events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
