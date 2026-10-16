// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Clone IR entities with a value / block / op remapping.
//!
//! An [IrMapping] records, for every original IR entity, the clone that stands
//! in for it. Cloning an [Operation] rewrites each operand and successor through
//! the mapping.
//!
//! Pliron provides two forms of IR cloning:
//! 1. Within a [Context]: ([clone_operation], [clone_region_into], [clone_blocks_into]).
//!    Operands and successors that are *not* in the mapping are left unchanged. Uses
//!    of values defined outside the cloned scope continue to refer to the originals.
//! 2. Cross [Context]: ([clone_operation_into_context], [clone_region_into_context],
//!    [clone_blocks_into_context]). Types, attributes and locations are cloned with
//!    [CloneIntoContext]. Every operand and successor must be defined inside the cloned
//!    scope or be in the mapping.
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
use thiserror::Error;

use crate::{
    arg_err,
    attribute::AttributeDict,
    basic_block::BasicBlock,
    context::{Context, Ptr},
    irbuild::{
        decontext::CloneIntoContext,
        inserter::{BlockInsertionPoint, Inserter, OpInsertionPoint},
        rewriter::{Rewriter, ScopedRewriter},
    },
    linked_list::ContainsLinkedList,
    location::Location,
    operation::Operation,
    region::Region,
    result::Result,
    r#type::{TypeHandle, Typed},
    utils::table::{HMap, htable::Entry},
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

/// Errors from cloning IR into another [Context].
#[derive(Debug, Error)]
pub enum CloneErr {
    #[error("operand defined outside the cloned IR has no mapping")]
    UnmappedValue,
    #[error("successor outside the cloned IR has no mapping")]
    UnmappedBlock,
}

/// Common functionality shared by same-context and cross-context cloning.
trait CloneEnv {
    /// The [Context] that holds the IR to clone.
    fn src(&self) -> &Context;
    /// The [Context] that receives the clone.
    fn dst(&mut self) -> &mut Context;
    /// Get the type in [Self::dst] for `ty` from [Self::src].
    fn map_type(&mut self, ty: TypeHandle) -> Result<TypeHandle>;
    /// Get the attributes and location in [Self::dst] for `op` from [Self::src].
    fn clone_op_attrs_and_loc(&mut self, op: Ptr<Operation>) -> Result<(AttributeDict, Location)>;
    /// Get the attributes and location in [Self::dst] for `block` from [Self::src].
    fn clone_block_attrs_and_loc(
        &mut self,
        block: Ptr<BasicBlock>,
    ) -> Result<(AttributeDict, Location)>;
    /// Get the value to use for an operand `value` of `user` that has no mapping.
    fn unmapped_value(&self, value: Value, user: Ptr<Operation>) -> Result<Value>;
    /// Get the block to use for a successor `block` of `user` that has no mapping.
    fn unmapped_block(
        &self,
        block: Ptr<BasicBlock>,
        user: Ptr<Operation>,
    ) -> Result<Ptr<BasicBlock>>;
}

/// Cloning within a [Context]
struct SameContext<'a>(&'a mut Context);

impl CloneEnv for SameContext<'_> {
    fn src(&self) -> &Context {
        self.0
    }

    fn dst(&mut self) -> &mut Context {
        self.0
    }

    fn map_type(&mut self, ty: TypeHandle) -> Result<TypeHandle> {
        Ok(ty)
    }

    fn clone_op_attrs_and_loc(&mut self, op: Ptr<Operation>) -> Result<(AttributeDict, Location)> {
        let op_ref = op.deref(self.0);
        Ok((op_ref.attributes.clone(), op_ref.loc().clone()))
    }

    fn clone_block_attrs_and_loc(
        &mut self,
        block: Ptr<BasicBlock>,
    ) -> Result<(AttributeDict, Location)> {
        let block_ref = block.deref(self.0);
        Ok((block_ref.attributes.clone(), block_ref.loc().clone()))
    }

    fn unmapped_value(&self, value: Value, _user: Ptr<Operation>) -> Result<Value> {
        Ok(value)
    }

    fn unmapped_block(
        &self,
        block: Ptr<BasicBlock>,
        _user: Ptr<Operation>,
    ) -> Result<Ptr<BasicBlock>> {
        Ok(block)
    }
}

/// Cross [Context] cloning.
struct CrossContext<'a> {
    src: &'a Context,
    dst: &'a mut Context,
    /// Clones of the types seen so far. Type uniquing ensures that
    /// a source type always maps to the same destination type.
    types: HMap<TypeHandle, TypeHandle>,
}

impl<'a> CrossContext<'a> {
    fn new(src: &'a Context, dst: &'a mut Context) -> Self {
        Self {
            src,
            dst,
            types: HMap::default(),
        }
    }
}

impl CloneEnv for CrossContext<'_> {
    fn src(&self) -> &Context {
        self.src
    }

    fn dst(&mut self) -> &mut Context {
        self.dst
    }

    fn map_type(&mut self, ty: TypeHandle) -> Result<TypeHandle> {
        match self.types.entry(ty) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                let cloned = ty.clone_into_context(self.src, self.dst)?;
                Ok(*entry.insert(cloned))
            }
        }
    }

    fn clone_op_attrs_and_loc(&mut self, op: Ptr<Operation>) -> Result<(AttributeDict, Location)> {
        let op_ref = op.deref(self.src);
        Ok((
            op_ref.attributes.clone_into_context(self.src, self.dst)?,
            op_ref.loc().clone_into_context(self.src, self.dst)?,
        ))
    }

    fn clone_block_attrs_and_loc(
        &mut self,
        block: Ptr<BasicBlock>,
    ) -> Result<(AttributeDict, Location)> {
        let block_ref = block.deref(self.src);
        Ok((
            block_ref
                .attributes
                .clone_into_context(self.src, self.dst)?,
            block_ref.loc().clone_into_context(self.src, self.dst)?,
        ))
    }

    fn unmapped_value(&self, _value: Value, user: Ptr<Operation>) -> Result<Value> {
        arg_err!(user.deref(self.src).loc().clone(), CloneErr::UnmappedValue)
    }

    fn unmapped_block(
        &self,
        _block: Ptr<BasicBlock>,
        user: Ptr<Operation>,
    ) -> Result<Ptr<BasicBlock>> {
        arg_err!(user.deref(self.src).loc().clone(), CloneErr::UnmappedBlock)
    }
}

/// Clone `op` (and the contents of its regions).
///
/// * Uses `mapper` to remap operands and successors.
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
    clone_operation_in(op, &mut SameContext(ctx), rewriter, mapper)
        .expect("cloning within one context cannot fail")
}

/// Clone `op` (and the contents of its regions) from `src_ctx` into `dst_ctx`.
///
/// * Uses `mapper` to remap operands and successors from `src_ctx` to `dst_ctx`.
/// * Updates `mapper` with new clones.
///
/// The returned [Operation] is **unlinked** (i.e., not inserted in any block).
///
/// See module docs for algorithm details.
pub fn clone_operation_into_context(
    op: Ptr<Operation>,
    src_ctx: &Context,
    dst_ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<Ptr<Operation>> {
    clone_operation_in(
        op,
        &mut CrossContext::new(src_ctx, dst_ctx),
        rewriter,
        mapper,
    )
}

fn clone_operation_in<E: CloneEnv>(
    op: Ptr<Operation>,
    env: &mut E,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<Ptr<Operation>> {
    let new_op = clone_op_shell(op, env, mapper)?;
    fill_operation(op, env, rewriter, mapper)?;
    Ok(new_op)
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
fn clone_op_shell<E: CloneEnv>(
    op: Ptr<Operation>,
    env: &mut E,
    mapper: &mut IrMapping,
) -> Result<Ptr<Operation>> {
    let (concrete_op, result_types, successors, num_operands, num_regions) = {
        let op_ref = op.deref(env.src());
        (
            op_ref.concrete_op_info(),
            op_ref.result_types().collect::<Vec<_>>(),
            op_ref.successors().collect::<Vec<_>>(),
            op_ref.get_num_operands(),
            op_ref.num_regions(),
        )
    };
    let successors = successors
        .into_iter()
        .map(|b| match mapper.lookup_block(b) {
            Some(mapped) => Ok(mapped),
            None => env.unmapped_block(b, op),
        })
        .collect::<Result<Vec<_>>>()?;
    let result_types = result_types
        .into_iter()
        .map(|ty| env.map_type(ty))
        .collect::<Result<Vec<_>>>()?;
    let (attributes, loc) = env.clone_op_attrs_and_loc(op)?;

    // No operands yet: they are pushed, remapped, in `fill_operation`.
    let new_op = Operation::new(
        env.dst(),
        concrete_op,
        result_types,
        Vec::with_capacity(num_operands),
        successors,
        num_regions,
    );
    {
        let mut new_ref = new_op.deref_mut(env.dst());
        new_ref.attributes = attributes;
        new_ref.set_loc(loc);
    }

    // Record the op and its results for later use.
    mapper.map_op(op, new_op);
    let old_results: Vec<Value> = op.deref(env.src()).results().collect();
    let new_results: Vec<Value> = new_op.deref(env.dst()).results().collect();
    for (old, new) in old_results.into_iter().zip(new_results) {
        mapper.map_value(old, new);
    }

    Ok(new_op)
}

/// Phase two of cloning an op: add the clone's operands and clone the contents
/// of its nested regions.
fn fill_operation<E: CloneEnv>(
    op: Ptr<Operation>,
    env: &mut E,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<()> {
    let new_op = mapper
        .lookup_op(op)
        .expect("op shell must be created before it is filled");

    // Operands, remapped through the now-complete mapping and pushed in order.
    let operands: Vec<Value> = op.deref(env.src()).operands().collect();
    for operand in operands {
        let operand = match mapper.lookup_value(operand) {
            Some(mapped) => mapped,
            None => env.unmapped_value(operand, op)?,
        };
        Operation::push_operand(new_op, env.dst(), operand);
    }

    // Clone the blocks of each region into the corresponding (empty) region of
    // the clone.
    let num_regions = op.deref(env.src()).num_regions();
    for region_idx in 0..num_regions {
        let src_region = op.deref(env.src()).get_region(region_idx);
        let dest_region = new_op.deref(env.dst()).get_region(region_idx);
        clone_region_in(src_region, dest_region, env, rewriter, mapper)?;
    }
    Ok(())
}

/// Clone every block of `src_region` into `dest_region`, appending at its end.
///
/// * Uses `mapper` to remap operands and successors.
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
    clone_region_in(
        src_region,
        dest_region,
        &mut SameContext(ctx),
        rewriter,
        mapper,
    )
    .expect("cloning within one context cannot fail")
}

/// Clone every block of `src_region` (in `src_ctx`) into `dest_region` (in `dst_ctx`),
/// appending at its end.
///
/// See [clone_operation_into_context] for the use of `mapper`.
pub fn clone_region_into_context(
    src_region: Ptr<Region>,
    dest_region: Ptr<Region>,
    src_ctx: &Context,
    dst_ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<()> {
    clone_region_in(
        src_region,
        dest_region,
        &mut CrossContext::new(src_ctx, dst_ctx),
        rewriter,
        mapper,
    )
}

fn clone_region_in<E: CloneEnv>(
    src_region: Ptr<Region>,
    dest_region: Ptr<Region>,
    env: &mut E,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<()> {
    let blocks: Vec<Ptr<BasicBlock>> = src_region.deref(env.src()).iter(env.src()).collect();
    clone_blocks_in(&blocks, dest_region, env, rewriter, mapper)
}

/// Clone `blocks` (and their operations) into `dest_region`, appending at its end
/// in the given order.
///
/// * Uses `mapper` to remap operands and successors.
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
    clone_blocks_in(blocks, dest_region, &mut SameContext(ctx), rewriter, mapper)
        .expect("cloning within one context cannot fail")
}

/// Clone `blocks` (in `src_ctx`, with their operations) into `dest_region` (in `dst_ctx`),
/// appending at its end in the given order.
///
/// See [clone_operation_into_context] for the use of `mapper`.
pub fn clone_blocks_into_context(
    blocks: &[Ptr<BasicBlock>],
    dest_region: Ptr<Region>,
    src_ctx: &Context,
    dst_ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<()> {
    clone_blocks_in(
        blocks,
        dest_region,
        &mut CrossContext::new(src_ctx, dst_ctx),
        rewriter,
        mapper,
    )
}

fn clone_blocks_in<E: CloneEnv>(
    blocks: &[Ptr<BasicBlock>],
    dest_region: Ptr<Region>,
    env: &mut E,
    rewriter: &mut dyn Rewriter,
    mapper: &mut IrMapping,
) -> Result<()> {
    let mut rewriter = ScopedRewriter::new(rewriter, OpInsertionPoint::Unset);

    // Phase 1: create the clone blocks and their arguments, and record them.
    for &src_block in blocks {
        let (arg_types, label) = {
            let src = env.src();
            let block_ref = src_block.deref(src);
            let arg_types: Vec<_> = block_ref.arguments().map(|arg| arg.get_type(src)).collect();
            (arg_types, block_ref.label.clone())
        };
        let arg_types = arg_types
            .into_iter()
            .map(|ty| env.map_type(ty))
            .collect::<Result<Vec<_>>>()?;
        let (attrs, loc) = env.clone_block_attrs_and_loc(src_block)?;

        let new_block = rewriter.create_block(
            env.dst(),
            BlockInsertionPoint::AtRegionEnd(dest_region),
            label,
            arg_types,
        );

        {
            let mut new_ref = new_block.deref_mut(env.dst());
            new_ref.attributes = attrs;
            new_ref.set_loc(loc);
        }

        let old_args: Vec<Value> = src_block.deref(env.src()).arguments().collect();
        let new_args: Vec<Value> = new_block.deref(env.dst()).arguments().collect();
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
        let ops: Vec<Ptr<Operation>> = src_block.deref(env.src()).iter(env.src()).collect();
        for src_op in ops {
            let shell = clone_op_shell(src_op, env, mapper)?;
            rewriter.append_operation(env.dst(), shell);
        }
    }

    // Phase 3: add operands and clone nested regions, now that every op result
    // in `blocks` is recorded.
    for &src_block in blocks {
        let ops: Vec<Ptr<Operation>> = src_block.deref(env.src()).iter(env.src()).collect();
        for src_op in ops {
            fill_operation(src_op, env, &mut rewriter, mapper)?;
        }
    }
    Ok(())
}
