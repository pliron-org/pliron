// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! IR equivalence, hashing etc
//!
//! Structural equivalence for [Operation]s, [Region]s and [BasicBlock]s.
//!
//! The equivalence algorithm is two-pass, conceptually.
//! 1. Map all definitions (op results, block args and blocks).
//! 2. Compare everything. Uses (operands / successors) are compared
//!    through their mappings if not directly equal.
//!
//! Structural hashing for [Operation]s, [Region]s and [BasicBlock]s.
//!
//! Hash computation is coherent with equivalence. i.e., whenever equivalence
//! returns [EqResult::Eq] for `lhs` and `rhs` (compared with an empty [IrMapping]
//! and the same [IgnoreConfig]), the corresponding hash function returns the same
//! hash for `lhs` and `rhs`. The converse isn't guaranteed: collisions are possible.
//!
//! If the IR being hashed is self contained (i.e., no free values or blocks),
//! the hash is stable across builds and common platforms, making it suitable for use
//! as a key for caching the IR. Hashes may vary across versions of pliron or rustc.

use core::hash::{Hash, Hasher};

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    attribute::{AttrObj, Attribute, AttributeDict},
    basic_block::BasicBlock,
    common_traits::Named,
    context::{Context, Ptr},
    identifier::Identifier,
    irbuild::cloning::IrMapping,
    linked_list::ContainsLinkedList,
    location::Located,
    operation::{OpDbg, Operation},
    printable::{Printable, State},
    region::Region,
    r#type::Typed,
    utils::table::{FxHasher, HMap, IMap},
    value::Value,
};

/// Configuration to decide what parts of the IR must be ignored.
#[derive(Clone, Copy)]
pub struct IgnoreConfig {
    // Should location information be ignored?
    pub ignore_loc: bool,
    // Should a specific attribute be ignored?
    pub ignore_attr: fn(ctx: &Context, attr: &dyn Attribute) -> bool,
}

/// The result of equivalence checking b/w two IR entities
pub enum EqResult {
    /// The two IR entities are equivalent.
    Eq,
    /// First pair of [Operation]s found to be unequal.
    FirstNEQOps((Ptr<Operation>, Ptr<Operation>)),
    /// First pair of [Region]s found to be unequal.
    FirstNEQRegions((Ptr<Region>, Ptr<Region>)),
    /// First pair of [BasicBlock]s found to be unequal.
    FirstNEQBlocks((Ptr<BasicBlock>, Ptr<BasicBlock>)),
}

impl Printable for EqResult {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            EqResult::Eq => write!(f, "Eq"),
            EqResult::FirstNEQOps((lhs, rhs)) => write!(
                f,
                "{} != {}",
                OpDbg { op: *lhs, ctx },
                OpDbg { op: *rhs, ctx }
            ),
            EqResult::FirstNEQRegions((lhs, rhs)) => {
                let (lhs, rhs) = (lhs.deref(ctx), rhs.deref(ctx));
                write!(
                    f,
                    "{}[{}] != {}[{}]",
                    OpDbg {
                        op: lhs.get_parent_op(),
                        ctx
                    },
                    lhs.find_index_in_parent(ctx),
                    OpDbg {
                        op: rhs.get_parent_op(),
                        ctx
                    },
                    rhs.find_index_in_parent(ctx)
                )
            }
            EqResult::FirstNEQBlocks((lhs, rhs)) => write!(
                f,
                "{} != {}",
                lhs.deref(ctx).unique_name(ctx),
                rhs.deref(ctx).unique_name(ctx)
            ),
        }
    }
}

/// Compare two [AttributeDict]s.
pub fn attributes_eq(
    ctx: &Context,
    ignore_config: &IgnoreConfig,
    lhs: &AttributeDict,
    rhs: &AttributeDict,
) -> bool {
    let is_relevant = |attr: &AttrObj| !(ignore_config.ignore_attr)(ctx, attr.as_ref());

    let lhs_relevant: IMap<&Identifier, &AttrObj> =
        lhs.0.iter().filter(|(_, attr)| is_relevant(attr)).collect();

    let rhs_relevant: IMap<&Identifier, &AttrObj> =
        rhs.0.iter().filter(|(_, attr)| is_relevant(attr)).collect();

    if lhs_relevant.len() != rhs_relevant.len() {
        return false;
    }

    lhs_relevant.iter().all(|(lhs_id, lhs_attr)| {
        rhs_relevant
            .get(lhs_id)
            .is_some_and(|rhs_attr| lhs_attr == rhs_attr)
    })
}

/// Compare two [Operation]s.
pub fn operation_eq(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs: Ptr<Operation>,
    rhs: Ptr<Operation>,
    ignore_config: &IgnoreConfig,
) -> EqResult {
    match op_eq_map(ctx, mapper, lhs, rhs, ignore_config) {
        EqResult::Eq => {}
        neq => return neq,
    }
    op_eq_mapped(ctx, mapper, lhs, rhs, ignore_config)
}

/// Check that `lhs` and `rhs` have the same shape and map `lhs -> rhs`.
fn op_eq_map(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs: Ptr<Operation>,
    rhs: Ptr<Operation>,
    ignore_config: &IgnoreConfig,
) -> EqResult {
    let lhs_ref = lhs.deref(ctx);
    let rhs_ref = rhs.deref(ctx);

    // Compare the operation's identity (dialect + op kind) and shape.
    if lhs_ref.concrete_op_info().1 != rhs_ref.concrete_op_info().1
        || lhs_ref.num_regions() != rhs_ref.num_regions()
        || lhs_ref.get_num_successors() != rhs_ref.get_num_successors()
        || lhs_ref.get_num_operands() != rhs_ref.get_num_operands()
        || lhs_ref.get_num_results() != rhs_ref.get_num_results()
    {
        return EqResult::FirstNEQOps((lhs, rhs));
    }

    if !ignore_config.ignore_loc && lhs_ref.loc() != rhs_ref.loc() {
        return EqResult::FirstNEQOps((lhs, rhs));
    }

    if !attributes_eq(ctx, ignore_config, &lhs_ref.attributes, &rhs_ref.attributes) {
        return EqResult::FirstNEQOps((lhs, rhs));
    }

    // Check result types.
    let results = lhs_ref.results().zip(rhs_ref.results());
    if results
        .clone()
        .any(|(l_res, r_res)| l_res.get_type(ctx) != r_res.get_type(ctx))
    {
        return EqResult::FirstNEQOps((lhs, rhs));
    }

    // Do all the mapping
    mapper.map_op(lhs, rhs);
    for (l_res, r_res) in results {
        mapper.map_value(l_res, r_res);
    }

    EqResult::Eq
}

/// Compare everything that [op_eq_map] left unchecked: operands, successors and
/// nested regions. Their mapping must already be updated.
fn op_eq_mapped(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs: Ptr<Operation>,
    rhs: Ptr<Operation>,
    ignore_config: &IgnoreConfig,
) -> EqResult {
    let num_regions = {
        // Hold runtime borrows for as little time as needed
        // (just a good practice).
        let lhs_ref = lhs.deref(ctx);
        let rhs_ref = rhs.deref(ctx);

        // If an operand is mapped, its image must match. Otherwise, it must
        // be identical (e.g. a free/external value shared by both sides).
        for (l_opd, r_opd) in lhs_ref.operands().zip(rhs_ref.operands()) {
            if mapper.lookup_value_or_default(l_opd) != r_opd
                || l_opd.get_type(ctx) != r_opd.get_type(ctx)
            {
                return EqResult::FirstNEQOps((lhs, rhs));
            }
        }

        // If a successor is mapped, its image must match. Otherwise, it must
        // be identical (e.g. a free/external block shared by both sides).
        for (l_succ, r_succ) in lhs_ref.successors().zip(rhs_ref.successors()) {
            if mapper.lookup_block_or_default(l_succ) != r_succ {
                return EqResult::FirstNEQOps((lhs, rhs));
            }
        }
        lhs_ref.num_regions()
    };

    // Recurse into nested regions.
    for region_idx in 0..num_regions {
        let l_region = lhs.deref(ctx).get_region(region_idx);
        let r_region = rhs.deref(ctx).get_region(region_idx);
        match region_eq(ctx, mapper, l_region, r_region, ignore_config) {
            EqResult::Eq => {}
            neq => return neq,
        }
    }

    EqResult::Eq
}

/// Compare two [Region]s.
pub fn region_eq(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs: Ptr<Region>,
    rhs: Ptr<Region>,
    ignore_config: &IgnoreConfig,
) -> EqResult {
    let lhs_blocks = lhs.deref(ctx).iter(ctx);
    let rhs_blocks = rhs.deref(ctx).iter(ctx);

    match blocks_eq(ctx, mapper, lhs_blocks, rhs_blocks, ignore_config) {
        Ok(eq) => eq,
        Err(()) => EqResult::FirstNEQRegions((lhs, rhs)),
    }
}

/// Compare two [BasicBlock]s
pub fn basic_block_eq(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs: Ptr<BasicBlock>,
    rhs: Ptr<BasicBlock>,
    ignore_config: &IgnoreConfig,
) -> EqResult {
    blocks_eq(
        ctx,
        mapper,
        core::iter::once(lhs),
        core::iter::once(rhs),
        ignore_config,
    )
    .expect("blocks_eq only fails when the blocks list length differs")
}

/// Compare two equal-length lists of blocks.
/// Returns `Err` only if the lists length differs.
fn blocks_eq(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs_blocks: impl Iterator<Item = Ptr<BasicBlock>> + Clone,
    rhs_blocks: impl Iterator<Item = Ptr<BasicBlock>> + Clone,
    ignore_config: &IgnoreConfig,
) -> Result<EqResult, ()> {
    // Pass 1: blocks and their arguments.
    let (mut lhs_blocks_clone, mut rhs_blocks_clone) = (lhs_blocks.clone(), rhs_blocks.clone());
    for (l_block, r_block) in lhs_blocks_clone.by_ref().zip(rhs_blocks_clone.by_ref()) {
        match block_eq_map(ctx, mapper, l_block, r_block, ignore_config) {
            EqResult::Eq => {}
            neq => return Ok(neq),
        }
    }

    if lhs_blocks_clone.next() != rhs_blocks_clone.next() {
        return Err(());
    }

    // Pass 2: every block's ops and their results.
    for (l_block, r_block) in lhs_blocks.clone().zip(rhs_blocks.clone()) {
        let mut l_ops = l_block.deref(ctx).iter(ctx);
        let mut r_ops = r_block.deref(ctx).iter(ctx);
        let l_r_ops = l_ops.by_ref().zip(r_ops.by_ref());
        for (l_op, r_op) in l_r_ops {
            match op_eq_map(ctx, mapper, l_op, r_op, ignore_config) {
                EqResult::Eq => {}
                neq => return Ok(neq),
            }
        }
        if l_ops.next() != r_ops.next() {
            // Unequal lengths
            return Ok(EqResult::FirstNEQBlocks((l_block, r_block)));
        }
    }

    // Pass 3: full comparison of every op (operands, successors, regions).
    for (l_block, r_block) in lhs_blocks.zip(rhs_blocks) {
        let l_ops = l_block.deref(ctx).iter(ctx);
        let r_ops = r_block.deref(ctx).iter(ctx);
        for (l_op, r_op) in l_ops.zip(r_ops) {
            match op_eq_mapped(ctx, mapper, l_op, r_op, ignore_config) {
                EqResult::Eq => {}
                neq => return Ok(neq),
            }
        }
    }

    Ok(EqResult::Eq)
}

/// Check that `lhs` and `rhs` have the same shape and map `lhs -> rhs`
fn block_eq_map(
    ctx: &Context,
    mapper: &mut IrMapping,
    lhs: Ptr<BasicBlock>,
    rhs: Ptr<BasicBlock>,
    ignore_config: &IgnoreConfig,
) -> EqResult {
    let lhs_ref = lhs.deref(ctx);
    let rhs_ref = rhs.deref(ctx);

    if lhs_ref.get_num_arguments() != rhs_ref.get_num_arguments() {
        return EqResult::FirstNEQBlocks((lhs, rhs));
    }

    if !ignore_config.ignore_loc && lhs_ref.loc() != rhs_ref.loc() {
        return EqResult::FirstNEQBlocks((lhs, rhs));
    }

    if !attributes_eq(ctx, ignore_config, &lhs_ref.attributes, &rhs_ref.attributes) {
        return EqResult::FirstNEQBlocks((lhs, rhs));
    }

    // Check arg types.
    let args = lhs_ref.arguments().zip(rhs_ref.arguments());
    if args
        .clone()
        .any(|(l_arg, r_arg)| l_arg.get_type(ctx) != r_arg.get_type(ctx))
    {
        return EqResult::FirstNEQBlocks((lhs, rhs));
    }

    mapper.map_block(lhs, rhs);
    for (l_arg, r_arg) in args {
        mapper.map_value(l_arg, r_arg);
    }

    EqResult::Eq
}

struct HashNumbering {
    values: HMap<Value, usize>,
    blocks: HMap<Ptr<BasicBlock>, usize>,
}

impl HashNumbering {
    fn new() -> Self {
        HashNumbering {
            values: HMap::default(),
            blocks: HMap::default(),
        }
    }

    /// Record that `v` is defined within the entity being hashed.
    fn define_value(&mut self, v: Value) {
        let id = self.values.len();
        self.values.insert(v, id);
    }

    /// Record that `b` is defined within the entity being hashed.
    fn define_block(&mut self, b: Ptr<BasicBlock>) {
        let id = self.blocks.len();
        self.blocks.insert(b, id);
    }

    /// Hash a use of `v`: its position if defined within the entity being
    /// hashed, or its absolute identity otherwise.
    fn hash_value(&self, v: Value, state: &mut FxHasher) {
        match self.values.get(&v) {
            Some(id) => (0u8, id).hash(state),
            None => (1u8, v).hash(state),
        }
    }

    /// Hash a use of `b` as a successor: its position if defined within the
    /// entity being hashed, or its absolute identity otherwise.
    fn hash_block(&self, b: Ptr<BasicBlock>, state: &mut FxHasher) {
        match self.blocks.get(&b) {
            Some(id) => (0u8, id).hash(state),
            None => (1u8, b).hash(state),
        }
    }
}

/// Hash an [AttributeDict], ignoring those specified by `ignore_config`.
fn attributes_hash(
    ctx: &Context,
    ignore_config: &IgnoreConfig,
    attributes: &AttributeDict,
    state: &mut FxHasher,
) {
    let is_relevant = |attr: &AttrObj| !(ignore_config.ignore_attr)(ctx, attr.as_ref());

    let mut relevant: Vec<_> = attributes
        .0
        .iter()
        .filter(|(_, attr)| is_relevant(attr))
        .collect();
    // Sort by key so that the hash doesn't depend on insertion order.
    relevant.sort_by_key(|(k, _)| *k);

    // Because attributes can contain Types, which are interned and cannot be hashed
    // based on just their handle, we print the attributes and hash that instead.
    let mut attr_string = String::new();
    for (k, attr) in relevant {
        attr_string.push_str(&format!("{}={},", k, attr.disp(ctx)));
    }
    attr_string.hash(state);
}

/// Hash components of an op that do not use or contain other IR entities,
/// and define `op`'s results in `numbering`.
fn hash_op_shell(
    ctx: &Context,
    numbering: &mut HashNumbering,
    op: Ptr<Operation>,
    ignore_config: &IgnoreConfig,
    state: &mut FxHasher,
) {
    Operation::get_opid(op, ctx).hash(state);
    {
        let op_ref = op.deref(ctx);

        // Although we hash each region, they may be empty, so this is necessary.
        op_ref.num_regions().hash(state);

        if !ignore_config.ignore_loc {
            // Can't hash locations based on just their handle (interned file paths).
            op_ref.loc().disp(ctx).to_string().hash(state);
        }

        attributes_hash(ctx, ignore_config, &op_ref.attributes, state);

        for res in op_ref.results() {
            // Types are interned and cannot be hashed based on just their handle,
            // so we print the type and hash that instead.
            res.get_type(ctx).disp(ctx).to_string().hash(state);
        }
    }

    for res in op.deref(ctx).results() {
        numbering.define_value(res);
    }
}

/// Complete the hash of an op by hashing its operands, successors and nested regions.
/// Does not hash components already hashed by [hash_op_shell].
fn hash_op_full(
    ctx: &Context,
    numbering: &mut HashNumbering,
    op: Ptr<Operation>,
    ignore_config: &IgnoreConfig,
    state: &mut FxHasher,
) {
    let num_regions = {
        let op_ref = op.deref(ctx);

        for opd in op_ref.operands() {
            numbering.hash_value(opd, state);
        }
        for succ in op_ref.successors() {
            numbering.hash_block(succ, state);
        }
        op_ref.num_regions()
    };

    for region_idx in 0..num_regions {
        let region = op.deref(ctx).get_region(region_idx);
        hash_blocks_full(
            ctx,
            numbering,
            region.deref(ctx).iter(ctx),
            ignore_config,
            state,
        );
    }
}

/// Hash components of a block that do not use or contain other IR entities,
/// and define `block` and its arguments in `numbering`.
fn hash_block_shell(
    ctx: &Context,
    numbering: &mut HashNumbering,
    block: Ptr<BasicBlock>,
    ignore_config: &IgnoreConfig,
    state: &mut FxHasher,
) {
    {
        let block_ref = block.deref(ctx);

        if !ignore_config.ignore_loc {
            // Can't hash locations based on just their handle (interned file paths).
            block_ref.loc().disp(ctx).to_string().hash(state);
        }

        attributes_hash(ctx, ignore_config, &block_ref.attributes, state);

        for arg in block_ref.arguments() {
            // Types are interned and cannot be hashed based on just their handle,
            // so we print the type and hash that instead.
            arg.get_type(ctx).disp(ctx).to_string().hash(state);
        }
    }

    numbering.define_block(block);
    for arg in block.deref(ctx).arguments() {
        numbering.define_value(arg);
    }
}

/// Hash a list of blocks, hashing the following components in order:
/// 1. Every block's shell (attributes and arguments).
/// 2. Components of every op (attributes, results etc) that don't depend on other IR entities.
/// 3. Every op's operands, successors and nested regions.
///
/// The multi-pass structure ensures that any reference to a block or value
/// defined anywhere in `blocks` (including forward references, since regions
/// aren't required to have SSA dominance) is already numbered by the time
/// it's hashed.
fn hash_blocks_full(
    ctx: &Context,
    numbering: &mut HashNumbering,
    blocks: impl Iterator<Item = Ptr<BasicBlock>> + Clone,
    ignore_config: &IgnoreConfig,
    state: &mut FxHasher,
) {
    for block in blocks.clone() {
        hash_block_shell(ctx, numbering, block, ignore_config, state);
    }

    for block in blocks.clone() {
        let ops: Vec<_> = block.deref(ctx).iter(ctx).collect();
        for op in &ops {
            hash_op_shell(ctx, numbering, *op, ignore_config, state);
        }
    }

    for block in blocks {
        for op in block.deref(ctx).iter(ctx) {
            hash_op_full(ctx, numbering, op, ignore_config, state);
        }
    }
}

/// Compute a structural hash for `op`.
pub fn operation_hash(ctx: &Context, op: Ptr<Operation>, ignore_config: &IgnoreConfig) -> u64 {
    let mut numbering = HashNumbering::new();
    let mut state = FxHasher::default();
    hash_op_shell(ctx, &mut numbering, op, ignore_config, &mut state);
    hash_op_full(ctx, &mut numbering, op, ignore_config, &mut state);
    state.finish()
}

/// Compute a structural hash for `region`.
pub fn region_hash(ctx: &Context, region: Ptr<Region>, ignore_config: &IgnoreConfig) -> u64 {
    let mut numbering = HashNumbering::new();
    let mut state = FxHasher::default();
    hash_blocks_full(
        ctx,
        &mut numbering,
        region.deref(ctx).iter(ctx),
        ignore_config,
        &mut state,
    );
    state.finish()
}

/// Compute a structural hash for `block`.
pub fn basic_block_hash(
    ctx: &Context,
    block: Ptr<BasicBlock>,
    ignore_config: &IgnoreConfig,
) -> u64 {
    let mut numbering = HashNumbering::new();
    let mut state = FxHasher::default();
    hash_blocks_full(
        ctx,
        &mut numbering,
        core::iter::once(block),
        ignore_config,
        &mut state,
    );
    state.finish()
}
