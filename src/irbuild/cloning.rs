// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Cloning IR entities with a value / block / op remapping.
//!
//! An [IrMapping] records, for every original IR entity, the clone that stands
//! in for it. Cloning an [Operation] rewrites each operand and successor through
//! the mapping; anything *not* in the mapping is left unchanged. This means that
//! uses of values defined outside the cloned scope will continue referring to the
//! originals.
//!
//! Cloning is **order-independent**, achieved conceptually in two passes.
//! 1. Clone and map (original -> cloned) definitions first. Definitions are
//!    op results, block arguments and basic blocks. The clones are just a "shell":
//!    they do not have any uses (operands / successors).
//! 2. Add mapped uses (operands / successors) to the cloned shell operations and
//!    clone nested regions.
//!
//! So a reference that points "forward" (a branch to a later block, or an operand
//! whose def isn't cloned yet) still resolves to its clone.
//!
//! New blocks and ops are inserted through a [Rewriter], so any listener it
//! carries is notified.

use alloc::vec::Vec;

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
    utils::table::HMap,
    value::Value,
};

/// Mapping between IR entities
#[derive(Debug, Default)]
pub struct IrMapping {
    values: HMap<Value, Value>,
    blocks: HMap<Ptr<BasicBlock>, Ptr<BasicBlock>>,
    ops: HMap<Ptr<Operation>, Ptr<Operation>>,
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

    /// Look up mapping for value `from`.
    pub fn lookup_value(&self, from: Value) -> Option<Value> {
        self.values.get(&from).copied()
    }

    /// Look up mapping for block `from`.
    pub fn lookup_block(&self, from: Ptr<BasicBlock>) -> Option<Ptr<BasicBlock>> {
        self.blocks.get(&from).copied()
    }

    /// Look up mapping for op `from`.
    pub fn lookup_op(&self, from: Ptr<Operation>) -> Option<Ptr<Operation>> {
        self.ops.get(&from).copied()
    }

    /// Look up mapping for value `from`. If none exists, returns `from`.
    pub fn lookup_value_or_default(&self, from: Value) -> Value {
        self.lookup_value(from).unwrap_or(from)
    }

    /// Look up mapping for block `from`. If none exists, returns `from`.
    pub fn lookup_block_or_default(&self, from: Ptr<BasicBlock>) -> Ptr<BasicBlock> {
        self.lookup_block(from).unwrap_or(from)
    }
}

/// Clone `op` (and the contents of its regions).
///
/// * Uses `mapper` for remapping operands and successors.
/// * Updates `mapper` with new clones.
///
/// The returned [Operation] is **unlinked** (i.e., not inserted in any block).
///
/// See module docs for algorithm details.
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

/// Phase one of cloning an op:
/// 1. Build its clone with the right result types, successors and (empty) regions
///    but **no operands**
/// 2. Record the op and its results in `mapper`.
///
/// Note: Successors are already safe to remap: when cloning a block list, every clone
/// block is created and recorded before any op shell is built. This implementation detail
/// deviates from the conceptual algorithm described in the module doc.
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

    // Record the op and its results for later use.
    mapper.map_op(op, new_op);
    let old_ref = op.deref(ctx);
    let new_ref = new_op.deref(ctx);
    for (old, new) in old_ref.results().zip(new_ref.results()) {
        mapper.map_value(old, new);
    }

    new_op
}

/// Phase two of cloning an op: add the clone's operands and clone the contents
/// of its nested regions.
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

/// Clone every block of `src_region` into `dest_region`, appending at its end.
///
/// * Uses `mapper` for remapping operands and successors.
/// * Updates `mapper` with new clones.
///
/// See module docs for algorithm details.
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

/// Clone `blocks` (and their operations) into `dest_region`, appending at its end
/// in the given order.
///
/// * Uses `mapper` for remapping operands and successors.
/// * Updates `mapper` with new clones.
///
/// See module docs for algorithm details.
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
        let (arg_types, label, attrs, loc) = {
            let block_ref = src_block.deref(ctx);
            let arg_types: Vec<_> = block_ref.arguments().map(|arg| arg.get_type(ctx)).collect();
            (
                arg_types,
                block_ref.label.clone(),
                block_ref.attributes.clone(),
                block_ref.loc(),
            )
        };
        let new_block = rewriter.create_block(
            ctx,
            BlockInsertionPoint::AtRegionEnd(dest_region),
            label,
            arg_types,
        );

        {
            let mut new_ref = new_block.deref_mut(ctx);
            new_ref.attributes = attrs;
            new_ref.set_loc(loc);
        }

        let src_ref = src_block.deref(ctx);
        let new_ref = new_block.deref(ctx);
        for (old, new) in src_ref.arguments().zip(new_ref.arguments()) {
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

    // Phase 3: add operands and clone nested regions, now that every op result
    // in `blocks` is recorded.
    for &src_block in blocks {
        let ops: Vec<Ptr<Operation>> = src_block.deref(ctx).iter(ctx).collect();
        for src_op in ops {
            fill_operation(src_op, ctx, &mut rewriter, mapper);
        }
    }
}
