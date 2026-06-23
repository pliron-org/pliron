//! Cloning IR entities with a value / block / op remapping.
//!
//! This mirrors MLIR's [`IRMapping`] + `Operation::clone(mapper)` /
//! `Region::cloneInto(mapper)`. An [IrMapping] records, for every original IR
//! entity, the clone that should stand in for it. Cloning an [Operation]
//! rewrites each operand and successor through the mapping; anything *not* in
//! the mapping is left unchanged (the MLIR `lookupOrDefault` rule), so uses of
//! values defined outside the cloned scope correctly keep pointing at the
//! originals.
//!
//! Cloning a set of blocks is **two-phase**: every clone block and its block
//! arguments are created and recorded first, and only then are the operations
//! cloned. This way branch successors and block-argument references resolve
//! even though they may point "forward" in the block list.
//!
//! [`IRMapping`]: https://mlir.llvm.org/doxygen/classmlir_1_1IRMapping.html

use alloc::vec::Vec;

use rustc_hash::FxHashMap;

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    linked_list::ContainsLinkedList,
    location::Located,
    operation::Operation,
    region::Region,
    r#type::Typed,
    value::Value,
};

/// A mapping from original IR entities to their clones, used while cloning.
///
/// Holds three independent maps: [Value]s, [BasicBlock]s, and [Operation]s.
/// While cloning, operand [Value]s are looked up in the value map and branch
/// successor [BasicBlock]s in the block map; entries absent from a map resolve
/// to themselves (see [IrMapping::lookup_value_or_default]).
#[derive(Debug, Default)]
pub struct IrMapping {
    values: FxHashMap<Value, Value>,
    blocks: FxHashMap<Ptr<BasicBlock>, Ptr<BasicBlock>>,
    ops: FxHashMap<Ptr<Operation>, Ptr<Operation>>,
}

impl IrMapping {
    /// Create an empty mapping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `from` maps to `to`. Overwrites any existing entry.
    pub fn map_value(&mut self, from: Value, to: Value) {
        self.values.insert(from, to);
    }

    /// Record that `from` maps to `to`. Overwrites any existing entry.
    pub fn map_block(&mut self, from: Ptr<BasicBlock>, to: Ptr<BasicBlock>) {
        self.blocks.insert(from, to);
    }

    /// Record that `from` maps to `to`. Overwrites any existing entry.
    pub fn map_op(&mut self, from: Ptr<Operation>, to: Ptr<Operation>) {
        self.ops.insert(from, to);
    }

    /// The clone recorded for `from`, if any.
    pub fn lookup_value(&self, from: Value) -> Option<Value> {
        self.values.get(&from).copied()
    }

    /// The clone recorded for `from`, if any.
    pub fn lookup_block(&self, from: Ptr<BasicBlock>) -> Option<Ptr<BasicBlock>> {
        self.blocks.get(&from).copied()
    }

    /// The clone recorded for `from`, if any.
    pub fn lookup_op(&self, from: Ptr<Operation>) -> Option<Ptr<Operation>> {
        self.ops.get(&from).copied()
    }

    /// The clone recorded for `from`, or `from` itself if none was recorded.
    /// Mirrors MLIR's `IRMapping::lookupOrDefault`.
    pub fn lookup_value_or_default(&self, from: Value) -> Value {
        self.lookup_value(from).unwrap_or(from)
    }

    /// The clone recorded for `from`, or `from` itself if none was recorded.
    /// Mirrors MLIR's `IRMapping::lookupOrDefault`.
    pub fn lookup_block_or_default(&self, from: Ptr<BasicBlock>) -> Ptr<BasicBlock> {
        self.lookup_block(from).unwrap_or(from)
    }
}

/// Clone `op` (and the contents of its regions), remapping its operands and
/// successors through `mapper`.
///
/// The returned [Operation] is **unlinked** (not in any block); the caller
/// inserts it. Operands and successors absent from `mapper` are kept as-is, so
/// values defined outside the cloned scope are shared with the original. The op
/// and its results are recorded into `mapper` so later clones can refer to them.
pub fn clone_operation(
    op: Ptr<Operation>,
    ctx: &mut Context,
    mapper: &mut IrMapping,
) -> Ptr<Operation> {
    // Gather everything needed to rebuild the op, remapping operands and
    // successors, then drop the borrow before `Operation::new` takes `&mut ctx`.
    let (concrete_op, result_types, operands, successors, num_regions, attributes, loc) = {
        let op_ref = op.deref(ctx);
        let operands: Vec<Value> = op_ref
            .operands()
            .map(|v| mapper.lookup_value_or_default(v))
            .collect();
        let successors: Vec<Ptr<BasicBlock>> = op_ref
            .successors()
            .map(|b| mapper.lookup_block_or_default(b))
            .collect();
        (
            op_ref.concrete_op_info(),
            op_ref.result_types().collect::<Vec<_>>(),
            operands,
            successors,
            op_ref.num_regions(),
            op_ref.attributes.clone(),
            op_ref.loc(),
        )
    };

    let new_op = Operation::new(
        ctx,
        concrete_op,
        result_types,
        operands,
        successors,
        num_regions,
    );
    {
        let mut new_ref = new_op.deref_mut(ctx);
        new_ref.attributes = attributes;
        new_ref.set_loc(loc);
    }

    // Record the op and its results before cloning nested regions.
    mapper.map_op(op, new_op);
    let old_results: Vec<Value> = op.deref(ctx).results().collect();
    let new_results: Vec<Value> = new_op.deref(ctx).results().collect();
    for (old, new) in old_results.into_iter().zip(new_results) {
        mapper.map_value(old, new);
    }

    // Clone the blocks of each region into the corresponding (empty) region of
    // the clone.
    for region_idx in 0..num_regions {
        let src_region = op.deref(ctx).get_region(region_idx);
        let dest_region = new_op.deref(ctx).get_region(region_idx);
        clone_region_into(src_region, dest_region, ctx, mapper);
    }

    new_op
}

/// Clone every block of `src_region` (in order) into `dest_region`, appended at
/// its end, remapping through `mapper`. See [clone_blocks_into].
pub fn clone_region_into(
    src_region: Ptr<Region>,
    dest_region: Ptr<Region>,
    ctx: &mut Context,
    mapper: &mut IrMapping,
) {
    let blocks: Vec<Ptr<BasicBlock>> = src_region.deref(ctx).iter(ctx).collect();
    clone_blocks_into(&blocks, dest_region, ctx, mapper);
}

/// Clone `blocks` (and their operations) into `dest_region`, appended at its end
/// in the given order, remapping through `mapper`.
///
/// Two-phase: all clone blocks and their block arguments are created and
/// recorded first, then operations are cloned. This lets branch successors and
/// block-argument references resolve even when they point forward in `blocks`.
/// Values and blocks absent from `mapper` are left unchanged
/// ([IrMapping::lookup_value_or_default]), so uses of values defined outside
/// `blocks` correctly keep pointing at the originals.
///
/// `blocks` should be given in a dominance-respecting order (for example,
/// reverse post-order) so that an operation result is cloned before its uses;
/// values that flow across a back-edge ride block arguments, which are mapped in
/// phase one and so resolve regardless of order.
///
/// Op attributes (including op result names) are copied, but block attributes
/// (block labels, block debug info) are not: the clone blocks are fresh.
pub fn clone_blocks_into(
    blocks: &[Ptr<BasicBlock>],
    dest_region: Ptr<Region>,
    ctx: &mut Context,
    mapper: &mut IrMapping,
) {
    // Phase 1: create the clone blocks and their arguments, and record them.
    for &src_block in blocks {
        let arg_types: Vec<_> = {
            let block_ref = src_block.deref(ctx);
            block_ref.arguments().map(|arg| arg.get_type(ctx)).collect()
        };
        let new_block = BasicBlock::new(ctx, None, arg_types);

        let old_args: Vec<Value> = src_block.deref(ctx).arguments().collect();
        let new_args: Vec<Value> = new_block.deref(ctx).arguments().collect();
        for (old, new) in old_args.into_iter().zip(new_args) {
            mapper.map_value(old, new);
        }

        new_block.insert_at_back(dest_region, ctx);
        mapper.map_block(src_block, new_block);
    }

    // Phase 2: clone each block's operations into its mapped clone.
    for &src_block in blocks {
        let new_block = mapper
            .lookup_block(src_block)
            .expect("block was mapped in phase one");
        let ops: Vec<Ptr<Operation>> = src_block.deref(ctx).iter(ctx).collect();
        for src_op in ops {
            let new_op = clone_operation(src_op, ctx, mapper);
            new_op.insert_at_back(new_block, ctx);
        }
    }
}
