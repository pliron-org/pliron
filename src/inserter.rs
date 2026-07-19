// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! A utility for inserting [Operation]s from a specified insertion point.
//! Similar in spirit to LLVM's IRBuilder, but does not build operations.

use crate::{
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    op::Op,
    operation::Operation,
    printable::{self, Printable},
};

/// Insertion point specification for inserting [Operation]s using [OpInserter].
#[derive(Clone, Copy)]
pub enum OpInsertionPoint {
    AtBlockStart(Ptr<BasicBlock>),
    AtBlockEnd(Ptr<BasicBlock>),
    AfterOperation(Ptr<Operation>),
    BeforeOperation(Ptr<Operation>),
}

impl Printable for OpInsertionPoint {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            OpInsertionPoint::AtBlockStart(block) => {
                write!(
                    f,
                    "At start of BasicBlock {}",
                    block.deref(ctx).unique_name(ctx)
                )
            }
            OpInsertionPoint::AtBlockEnd(block) => {
                write!(
                    f,
                    "At end of BasicBlock {}",
                    block.deref(ctx).unique_name(ctx)
                )
            }
            OpInsertionPoint::AfterOperation(op) => {
                write!(f, "After Operation {}", op.disp(ctx))
            }
            OpInsertionPoint::BeforeOperation(op) => {
                write!(f, "Before Operation {}", op.disp(ctx))
            }
        }
    }
}

/// A utility for inserting [Operation]s from a specified insertion point.
pub struct OpInserter {
    insertion_point: OpInsertionPoint,
}

impl OpInserter {
    /// Creates a new [OpInserter] that inserts the next operation
    /// at the start of the given [BasicBlock].
    pub fn new_at_block_start(block: Ptr<BasicBlock>) -> Self {
        Self {
            insertion_point: OpInsertionPoint::AtBlockStart(block),
        }
    }

    /// Creates a new [OpInserter] that inserts the next operation
    /// at the end of the given [BasicBlock].
    pub fn new_at_block_end(block: Ptr<BasicBlock>) -> Self {
        Self {
            insertion_point: OpInsertionPoint::AtBlockEnd(block),
        }
    }

    /// Creates a new [OpInserter] that inserts the next operation
    /// after the given [Operation].
    pub fn new_after_operation(op: Ptr<Operation>) -> Self {
        Self {
            insertion_point: OpInsertionPoint::AfterOperation(op),
        }
    }

    /// Creates a new [OpInserter] that inserts the next operation
    /// before the given [Operation].
    pub fn new_before_operation(op: Ptr<Operation>) -> Self {
        Self {
            insertion_point: OpInsertionPoint::BeforeOperation(op),
        }
    }

    /// Appends an [Operation] at the current insertion point.
    /// The insertion point is updated to be after this newly inserted [Operation].
    pub fn append_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        // Insert the operation at the current insertion point
        self.insert_operation(ctx, operation);
        // Update the insertion point to be after the newly inserted operation
        self.insertion_point = OpInsertionPoint::AfterOperation(operation);
    }

    /// Appends an [Op] at the current insertion point.
    /// The insertion point is updated to be after this newly inserted [Op].
    pub fn append_op(&mut self, ctx: &Context, op: impl Op) {
        let operation = op.get_operation();
        self.append_operation(ctx, operation);
    }

    /// Inserts an [Operation] at the current insertion point.
    /// To insert a sequence in-order, use [append_operation](Self::append_operation).
    pub fn insert_operation(&self, ctx: &Context, operation: Ptr<Operation>) {
        match self.insertion_point {
            OpInsertionPoint::AtBlockStart(block) => {
                // Insert operation at the start of the block
                operation.insert_at_front(block, ctx);
            }
            OpInsertionPoint::AtBlockEnd(block) => {
                // Insert operation at the end of the block
                operation.insert_at_back(block, ctx);
            }
            OpInsertionPoint::AfterOperation(op) => {
                // Insert operation after the specified operation
                operation.insert_after(ctx, op);
            }
            OpInsertionPoint::BeforeOperation(op) => {
                // Insert operation before the specified operation
                operation.insert_before(ctx, op);
            }
        }
    }

    /// Inserts an [Op] at the current insertion point.
    /// To insert a sequence in-order, use [append_op](Self::append_op).
    pub fn insert_op(&self, ctx: &Context, op: impl Op) {
        let operation = op.get_operation();
        self.insert_operation(ctx, operation);
    }

    /// Gets the current insertion point.
    pub fn insertion_point(&self) -> OpInsertionPoint {
        self.insertion_point
    }

    /// Get the current insertion block.
    pub fn get_insertion_block(&self, ctx: &Context) -> Ptr<BasicBlock> {
        match self.insertion_point {
            OpInsertionPoint::AtBlockStart(block) => block,
            OpInsertionPoint::AtBlockEnd(block) => block,
            OpInsertionPoint::AfterOperation(op) => op
                .deref(ctx)
                .get_parent_block()
                .expect("Insertion point Operation must have parent block"),
            OpInsertionPoint::BeforeOperation(op) => op
                .deref(ctx)
                .get_parent_block()
                .expect("Insertion point Operation must have parent block"),
        }
    }
}
