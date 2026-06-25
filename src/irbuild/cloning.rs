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
//! Cloning a set of blocks is **order-independent**: every clone block, block
//! argument and op result is created and recorded before any operand or
//! successor is wired. So a reference that points "forward" in the block list
//! (a branch to a later block, a back-edge, or an operand whose def is cloned
//! later) still resolves to its clone, whatever order the blocks are given in.
//! New blocks and ops are inserted through a [Rewriter], so any listener it
//! carries is notified.
//!
//! [`IRMapping`]: https://mlir.llvm.org/doxygen/classmlir_1_1IRMapping.html

use alloc::vec::Vec;

use rustc_hash::FxHashMap;

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    irbuild::{
        inserter::{BlockInsertionPoint, Inserter, OpInsertionPoint},
        rewriter::{Rewriter, ScopedRewriter},
    },
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
/// inserts it. New blocks created while cloning nested regions are inserted
/// through `rewriter`, so any listener it carries is notified. Operands and
/// successors absent from `mapper` are kept as-is, so values defined outside
/// the cloned scope are shared with the original. The op and its results are
/// recorded into `mapper` so later clones can refer to them.
pub fn clone_operation(
    op: Ptr<Operation>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Ptr<Operation> {
    let new_op = clone_op_shell(op, ctx, mapper);
    fill_operation(op, ctx, rewriter, mapper);
    new_op
}

/// Phase one of cloning an op: build its clone with the right result types,
/// successors and (empty) regions, but **no operands**, then record the op and
/// its results in `mapper`.
///
/// Splitting the operands off into a later pass ([fill_operation]) is what makes
/// cloning a block list order-independent: a use can be cloned before its def,
/// because the def's result is already recorded by the time operands are wired.
/// It also keeps the source IR untouched while shells are built (an empty
/// operand list adds no uses to any of the source's values).
///
/// Successors are safe to remap now: when cloning a block list, every clone
/// block is created and recorded before any op shell is built.
///
/// The returned op is **unlinked**.
fn clone_op_shell(op: Ptr<Operation>, ctx: &mut Context, mapper: &mut IrMapping) -> Ptr<Operation> {
    let (concrete_op, result_types, successors, num_operands, num_regions, attributes, loc) = {
        let op_ref = op.deref(ctx);
        let successors: Vec<Ptr<BasicBlock>> = op_ref
            .successors()
            .map(|b| mapper.lookup_block_or_default(b))
            .collect();
        (
            op_ref.concrete_op_info(),
            op_ref.result_types().collect::<Vec<_>>(),
            successors,
            op_ref.get_num_operands(),
            op_ref.num_regions(),
            op_ref.attributes.clone(),
            op_ref.loc(),
        )
    };

    // No operands yet: they are pushed, remapped, in `fill_operation`.
    let new_op = Operation::new(
        ctx,
        concrete_op,
        result_types,
        Vec::with_capacity(num_operands),
        successors,
        num_regions,
    );
    {
        let mut new_ref = new_op.deref_mut(ctx);
        new_ref.attributes = attributes;
        new_ref.set_loc(loc);
    }

    // Record the op and its results so later shells (and the operand-wiring
    // pass) can refer to them.
    mapper.map_op(op, new_op);
    let old_results: Vec<Value> = op.deref(ctx).results().collect();
    let new_results: Vec<Value> = new_op.deref(ctx).results().collect();
    for (old, new) in old_results.into_iter().zip(new_results) {
        mapper.map_value(old, new);
    }

    new_op
}

/// Phase two of cloning an op: wire the clone's operands and clone the contents
/// of its nested regions.
///
/// By now [clone_op_shell] has recorded `op` and its results in `mapper`, as
/// have the shells of any sibling ops, so every operand resolves to its clone
/// (or, if defined outside the cloned scope, to the original via
/// [IrMapping::lookup_value_or_default]). Cloning the nested regions is deferred
/// to here too, matching MLIR's `Region::cloneInto`.
fn fill_operation(
    op: Ptr<Operation>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) {
    let new_op = mapper
        .lookup_op(op)
        .expect("op shell must be created before it is filled");

    // Operands, remapped through the now-complete mapping and pushed in order.
    let operands: Vec<Value> = op
        .deref(ctx)
        .operands()
        .map(|v| mapper.lookup_value_or_default(v))
        .collect();
    for operand in operands {
        Operation::push_operand(new_op, ctx, operand);
    }

    // Clone the blocks of each region into the corresponding (empty) region of
    // the clone.
    let num_regions = op.deref(ctx).num_regions();
    for region_idx in 0..num_regions {
        let src_region = op.deref(ctx).get_region(region_idx);
        let dest_region = new_op.deref(ctx).get_region(region_idx);
        clone_region_into(src_region, dest_region, ctx, rewriter, mapper);
    }
}

/// Clone every block of `src_region` into `dest_region`, appended at its end,
/// remapping through `mapper`.
///
/// New blocks and ops are inserted through `rewriter`, so any listener it carries is notified.
/// `rewriter`'s insertion point is saved on entry and restored on return.
pub fn clone_region_into(
    src_region: Ptr<Region>,
    dest_region: Ptr<Region>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) {
    let blocks: Vec<Ptr<BasicBlock>> = src_region.deref(ctx).iter(ctx).collect();
    clone_blocks_into(&blocks, dest_region, ctx, rewriter, mapper);
}

/// Clone `blocks` (and their operations) into `dest_region`, appended at its end
/// in the given order, remapping through `mapper`.
///
/// New blocks and ops are inserted through `rewriter`, so any listener it carries is notified.
/// `rewriter`'s insertion point is saved on entry and restored on return.
//
// Cloning is **three-phase**, so the result does not depend on the order of
// `blocks`:
//
// 1. Create every clone block and its block arguments, and record them.
// 2. Create every clone op as a *shell* (correct results and successors, but no
//    operands and empty regions), and record each op and its results.
// 3. Wire each clone op's operands and clone its nested regions.
//
// Because every block, block argument and op result is recorded (phases 1-2)
// before any operand or successor is wired (phase 3), a reference that points
// "forward" in `blocks` -- a branch to a later block, a back-edge, or an
// operand whose def is cloned later -- still resolves to its clone. Values and
// blocks absent from `mapper` are left unchanged
// ([IrMapping::lookup_value_or_default]), so uses of values defined outside
// `blocks` keep pointing at the originals.
pub fn clone_blocks_into(
    blocks: &[Ptr<BasicBlock>],
    dest_region: Ptr<Region>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) {
    let mut rewriter = ScopedRewriter::new(rewriter, OpInsertionPoint::Unset);

    // Phase 1: create the clone blocks and their arguments, and record them.
    for &src_block in blocks {
        let (arg_types, label, attrs) = {
            let block_ref = src_block.deref(ctx);
            let arg_types: Vec<_> = block_ref.arguments().map(|arg| arg.get_type(ctx)).collect();
            (
                arg_types,
                block_ref.label.clone(),
                block_ref.attributes.clone(),
            )
        };
        let new_block = rewriter.create_block(
            ctx,
            BlockInsertionPoint::AtRegionEnd(dest_region),
            label,
            arg_types,
        );
        // `create_block` takes only the label and argument types, so the rest of
        // the block's attributes (debug info, argument names, ...) are copied
        // here, mirroring how op attributes are cloned.
        new_block.deref_mut(ctx).attributes = attrs;

        let old_args: Vec<Value> = src_block.deref(ctx).arguments().collect();
        let new_args: Vec<Value> = new_block.deref(ctx).arguments().collect();
        for (old, new) in old_args.into_iter().zip(new_args) {
            mapper.map_value(old, new);
        }

        mapper.map_block(src_block, new_block);
    }

    // Phase 2: create each block's op shells (no operands, empty regions) and
    // record their results, so later phases can refer to them in any order.
    for &src_block in blocks {
        let new_block = mapper
            .lookup_block(src_block)
            .expect("block was mapped in phase one");
        rewriter.set_insertion_point(OpInsertionPoint::AtBlockEnd(new_block));
        let ops: Vec<Ptr<Operation>> = src_block.deref(ctx).iter(ctx).collect();
        for src_op in ops {
            let shell = clone_op_shell(src_op, ctx, mapper);
            rewriter.append_operation(ctx, shell);
        }
    }

    // Phase 3: wire operands and clone nested regions, now that every op result
    // in `blocks` is recorded.
    for &src_block in blocks {
        let ops: Vec<Ptr<Operation>> = src_block.deref(ctx).iter(ctx).collect();
        for src_op in ops {
            fill_operation(src_op, ctx, &mut rewriter, mapper);
        }
    }
}
