//! A utility for inserting [Operation]s from a specified insertion point.
//! Similar in spirit to LLVM's IRBuilder, but does not build operations.

use crate::{
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    identifier::Identifier,
    irbuild::listener::InsertionListener,
    op::Op,
    operation::Operation,
    printable::{self, Printable},
    region::Region,
    r#type::TypeObj,
};

/// Insertion point specification for inserting [Operation]s using [IRInserter].
#[derive(Debug, Clone, Copy, Default)]
pub enum OpInsertionPoint {
    #[default]
    Unset,
    AtBlockStart(Ptr<BasicBlock>),
    AtBlockEnd(Ptr<BasicBlock>),
    AfterOperation(Ptr<Operation>),
    BeforeOperation(Ptr<Operation>),
}

/// Insertion point specification for insertion [BasicBlock]s using [IRInserter].
#[derive(Debug, Clone, Copy, Default)]
pub enum BlockInsertionPoint {
    #[default]
    Unset,
    AtRegionStart(Ptr<Region>),
    AtRegionEnd(Ptr<Region>),
    AfterBlock(Ptr<BasicBlock>),
    BeforeBlock(Ptr<BasicBlock>),
}

impl Printable for OpInsertionPoint {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            OpInsertionPoint::Unset => write!(f, "Op Insertion Point not set"),
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

impl Printable for BlockInsertionPoint {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            BlockInsertionPoint::Unset => write!(f, "Block Insertion Point not set"),
            BlockInsertionPoint::AtRegionStart(region) => {
                write!(
                    f,
                    "At start of Region {}",
                    region.deref(ctx).find_index_in_parent(ctx)
                )
            }
            BlockInsertionPoint::AtRegionEnd(region) => {
                write!(
                    f,
                    "At end of Region {}",
                    region.deref(ctx).find_index_in_parent(ctx)
                )
            }
            BlockInsertionPoint::AfterBlock(block) => {
                write!(f, "After BasicBlock {}", block.deref(ctx).unique_name(ctx))
            }
            BlockInsertionPoint::BeforeBlock(block) => {
                write!(f, "Before BasicBlock {}", block.deref(ctx).unique_name(ctx))
            }
        }
    }
}

impl OpInsertionPoint {
    /// Get the insertion block if set.
    pub fn get_insertion_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        match self {
            OpInsertionPoint::AtBlockStart(block) => Some(*block),
            OpInsertionPoint::AtBlockEnd(block) => Some(*block),
            OpInsertionPoint::AfterOperation(op) => op.deref(ctx).get_parent_block(),
            OpInsertionPoint::BeforeOperation(op) => op.deref(ctx).get_parent_block(),
            OpInsertionPoint::Unset => None,
        }
    }

    /// Is the insertion point set?
    pub fn is_set(&self) -> bool {
        !matches!(self, OpInsertionPoint::Unset)
    }
}

impl BlockInsertionPoint {
    /// Get the insertion region if set.
    pub fn get_insertion_region(&self, ctx: &Context) -> Option<Ptr<Region>> {
        match self {
            BlockInsertionPoint::AtRegionStart(region) => Some(*region),
            BlockInsertionPoint::AtRegionEnd(region) => Some(*region),
            BlockInsertionPoint::AfterBlock(block) => block.deref(ctx).get_parent_region(),
            BlockInsertionPoint::BeforeBlock(block) => block.deref(ctx).get_parent_region(),
            BlockInsertionPoint::Unset => None,
        }
    }

    /// Is the insertion point set?
    pub fn is_set(&self) -> bool {
        !matches!(self, BlockInsertionPoint::Unset)
    }
}

/// An interface for insertion of IR entities.
pub trait Inserter {
    /// Appends an [Operation] at the current insertion point.
    /// The insertion point is updated to be after this newly inserted [Operation].
    fn append_operation(&mut self, ctx: &Context, operation: Ptr<Operation>);

    /// Appends an [Op] at the current insertion point.
    /// The insertion point is updated to be after this newly inserted [Op].
    fn append_op(&mut self, ctx: &Context, op: &dyn Op);

    /// Inserts an [Operation] at the current insertion point.
    /// To insert a sequence in-order, use [append_operation](Self::append_operation).
    fn insert_operation(&mut self, ctx: &Context, operation: Ptr<Operation>);

    /// Inserts an [Op] at the current insertion point.
    /// To insert a sequence in-order, use [append_op](Self::append_op).
    fn insert_op(&mut self, ctx: &Context, op: &dyn Op);

    /// Insert [BasicBlock] at the provided insertion point.
    fn insert_block(
        &mut self,
        ctx: &Context,
        insertion_point: BlockInsertionPoint,
        block: Ptr<BasicBlock>,
    );

    /// Create a new [BasicBlock] and insert it at the provided insertion point.
    /// The internal [OpInsertionPoint] is updated to be at the end of the newly created block.
    fn create_block(
        &mut self,
        ctx: &mut Context,
        insertion_point: BlockInsertionPoint,
        label: Option<Identifier>,
        arg_types: Vec<Ptr<TypeObj>>,
    ) -> Ptr<BasicBlock>;

    /// Gets the current insertion point.
    fn get_insertion_point(&self) -> OpInsertionPoint;

    /// Is insertion point set?
    fn is_insertion_point_set(&self) -> bool {
        self.get_insertion_point().is_set()
    }

    /// Get the [BasicBlock], if known, in which the next [Op] insertion will occur.
    fn get_insertion_block(&self, ctx: &Context) -> Option<Ptr<BasicBlock>> {
        self.get_insertion_point().get_insertion_block(ctx)
    }

    /// Set the insertion point.
    fn set_insertion_point(&mut self, point: OpInsertionPoint);

    /// Sets the insertion point to the start of the given block.
    fn set_insertion_point_to_block_start(&mut self, block: Ptr<BasicBlock>) {
        self.set_insertion_point(OpInsertionPoint::AtBlockStart(block));
    }

    /// Sets the insertion point to the end of the given block.
    fn set_insertion_point_to_block_end(&mut self, block: Ptr<BasicBlock>) {
        self.set_insertion_point(OpInsertionPoint::AtBlockEnd(block));
    }

    /// Sets the insertion point to after the given operation.
    fn set_insertion_point_after_operation(&mut self, op: Ptr<Operation>) {
        self.set_insertion_point(OpInsertionPoint::AfterOperation(op));
    }

    /// Sets the insertion point to before the given operation.
    fn set_insertion_point_before_operation(&mut self, op: Ptr<Operation>) {
        self.set_insertion_point(OpInsertionPoint::BeforeOperation(op));
    }
}

/// A utility for inserting [Operation]s from a specified insertion point.
/// Use [DummyListener](super::listener::DummyListener) if no listener is needed.
pub struct IRInserter<L: InsertionListener> {
    op_insertion_point: OpInsertionPoint,
    modified: bool,
    listener: L,
}

impl<L: InsertionListener> Default for IRInserter<L> {
    fn default() -> Self {
        Self {
            op_insertion_point: OpInsertionPoint::default(),
            modified: false,
            listener: L::default(),
        }
    }
}

impl<L: InsertionListener> IRInserter<L> {
    /// Creates a new [Inserter] with insert point set to the provided argument.
    pub fn new(insertion_point: OpInsertionPoint) -> Self {
        Self {
            op_insertion_point: insertion_point,
            modified: false,
            listener: L::default(),
        }
    }

    /// Creates a new [Inserter] that inserts the next operation
    /// at the start of the given [BasicBlock].
    pub fn new_at_block_start(block: Ptr<BasicBlock>) -> Self {
        Self {
            op_insertion_point: OpInsertionPoint::AtBlockStart(block),
            modified: false,
            listener: L::default(),
        }
    }

    /// Creates a new [Inserter] that inserts the next operation
    /// at the end of the given [BasicBlock].
    pub fn new_at_block_end(block: Ptr<BasicBlock>) -> Self {
        Self {
            op_insertion_point: OpInsertionPoint::AtBlockEnd(block),
            modified: false,
            listener: L::default(),
        }
    }

    /// Creates a new [Inserter] that inserts the next operation
    /// after the given [Operation].
    pub fn new_after_operation(op: Ptr<Operation>) -> Self {
        Self {
            op_insertion_point: OpInsertionPoint::AfterOperation(op),
            modified: false,
            listener: L::default(),
        }
    }

    /// Creates a new [Inserter] that inserts the next operation
    /// before the given [Operation].
    pub fn new_before_operation(op: Ptr<Operation>) -> Self {
        Self {
            op_insertion_point: OpInsertionPoint::BeforeOperation(op),
            modified: false,
            listener: L::default(),
        }
    }

    /// Creates a new [Inserter] that inserts the next operation
    /// just before the terminator of the given [BasicBlock].
    pub fn new_before_block_terminator(block: Ptr<BasicBlock>, ctx: &Context) -> Self {
        let terminator_op = block
            .deref(ctx)
            .get_terminator(ctx)
            .expect("BasicBlock must have a terminator operation");
        Self::new_before_operation(terminator_op)
    }

    /// Sets the listener for insertion events.
    pub fn set_listener(&mut self, listener: L) {
        self.listener = listener;
    }

    /// Gets a reference to the listener for insertion events.
    pub fn get_listener(&self) -> &L {
        &self.listener
    }

    /// Gets a mutable reference to the listener for insertion events.
    pub fn get_listener_mut(&mut self) -> &mut L {
        &mut self.listener
    }

    /// Has the IR been modified by this inserter
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Mark the IR as modified by this inserter.
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }
}

impl<L: InsertionListener> Inserter for IRInserter<L> {
    fn append_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        // Insert the operation at the current insertion point
        self.insert_operation(ctx, operation);
        // Update the insertion point to be after the newly inserted operation
        self.op_insertion_point = OpInsertionPoint::AfterOperation(operation);
    }

    fn append_op(&mut self, ctx: &Context, op: &dyn Op) {
        let operation = op.get_operation();
        self.append_operation(ctx, operation);
    }

    fn insert_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        assert!(
            !operation.is_linked(ctx),
            "Cannot insert an already linked operation"
        );
        match self.op_insertion_point {
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
            OpInsertionPoint::Unset => {
                panic!("Insertion point is not set");
            }
        }
        // Notify the listener if present
        self.listener.notify_operation_inserted(ctx, operation);
        self.mark_modified();
    }

    fn insert_op(&mut self, ctx: &Context, op: &dyn Op) {
        let operation = op.get_operation();
        self.insert_operation(ctx, operation);
    }

    fn insert_block(
        &mut self,
        ctx: &Context,
        insertion_point: BlockInsertionPoint,
        block: Ptr<BasicBlock>,
    ) {
        match insertion_point {
            BlockInsertionPoint::AtRegionStart(region) => {
                block.insert_at_front(region, ctx);
            }
            BlockInsertionPoint::AtRegionEnd(region) => {
                block.insert_at_back(region, ctx);
            }
            BlockInsertionPoint::AfterBlock(prev_block) => {
                block.insert_after(ctx, prev_block);
            }
            BlockInsertionPoint::BeforeBlock(next_block) => {
                block.insert_before(ctx, next_block);
            }
            BlockInsertionPoint::Unset => {
                panic!("Block insertion point is not set");
            }
        }
        // Notify the listener if present
        self.listener.notify_block_inserted(ctx, block);
        self.mark_modified();
    }

    fn create_block(
        &mut self,
        ctx: &mut Context,
        insertion_point: BlockInsertionPoint,
        label: Option<Identifier>,
        arg_types: Vec<Ptr<TypeObj>>,
    ) -> Ptr<BasicBlock> {
        let block = BasicBlock::new(ctx, label, arg_types);
        self.insert_block(ctx, insertion_point, block);
        self.op_insertion_point = OpInsertionPoint::AtBlockEnd(block);
        // We don't notify the listener or mark the IR as modified
        // since it's not yet linked into the IR.
        block
    }

    fn get_insertion_point(&self) -> OpInsertionPoint {
        self.op_insertion_point
    }

    fn set_insertion_point(&mut self, point: OpInsertionPoint) {
        self.op_insertion_point = point;
    }
}

/// A scoped inserter that sets the insertion point for the duration of its lifetime.
/// On drop, it restores the previous insertion point.
/// Implements [Inserter] by forwarding calls to the wrapped inserter.
/// ```rust
/// # use pliron::{context::Context,
/// #   builtin::{ops::ModuleOp, op_interfaces::SingleBlockRegionInterface}};
/// # use pliron::irbuild::{listener::DummyListener,
/// #   inserter::{Inserter, IRInserter, ScopedInserter, OpInsertionPoint}};
/// let ctx = &mut Context::new();
/// let module = ModuleOp::new(ctx, "test_module".try_into().unwrap());
/// let mut inserter = IRInserter::<DummyListener>::default();
/// inserter.set_insertion_point(OpInsertionPoint::AtBlockEnd(module.get_body(ctx, 0)));
/// {
///     // We can create a scoped inserter with a different insertion point,
///     // and it will restore the original insertion point after this block.
///     let mut scoped_inserter = ScopedInserter::new(&mut inserter, OpInsertionPoint::Unset);
///     assert!(!scoped_inserter.get_insertion_point().is_set());
/// }
/// assert!(inserter.get_insertion_point().is_set());
/// ```
pub struct ScopedInserter<'a> {
    inserter: &'a mut dyn Inserter,
    prev_insertion_point: OpInsertionPoint,
}

impl<'a> ScopedInserter<'a> {
    pub fn new(inserter: &'a mut dyn Inserter, insertion_point: OpInsertionPoint) -> Self {
        let prev_insertion_point = inserter.get_insertion_point();
        inserter.set_insertion_point(insertion_point);
        Self {
            inserter,
            prev_insertion_point,
        }
    }
}

impl<'a> Drop for ScopedInserter<'a> {
    fn drop(&mut self) {
        self.inserter.set_insertion_point(self.prev_insertion_point);
    }
}

impl<'a> Inserter for ScopedInserter<'a> {
    fn append_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        self.inserter.append_operation(ctx, operation);
    }

    fn append_op(&mut self, ctx: &Context, op: &dyn Op) {
        self.inserter.append_op(ctx, op);
    }

    fn insert_operation(&mut self, ctx: &Context, operation: Ptr<Operation>) {
        self.inserter.insert_operation(ctx, operation);
    }

    fn insert_op(&mut self, ctx: &Context, op: &dyn Op) {
        self.inserter.insert_op(ctx, op);
    }

    fn insert_block(
        &mut self,
        ctx: &Context,
        insertion_point: BlockInsertionPoint,
        block: Ptr<BasicBlock>,
    ) {
        self.inserter.insert_block(ctx, insertion_point, block);
    }

    fn create_block(
        &mut self,
        ctx: &mut Context,
        insertion_point: BlockInsertionPoint,
        label: Option<Identifier>,
        arg_types: Vec<Ptr<TypeObj>>,
    ) -> Ptr<BasicBlock> {
        self.inserter
            .create_block(ctx, insertion_point, label, arg_types)
    }

    fn get_insertion_point(&self) -> OpInsertionPoint {
        self.inserter.get_insertion_point()
    }

    fn set_insertion_point(&mut self, point: OpInsertionPoint) {
        self.inserter.set_insertion_point(point);
    }
}
