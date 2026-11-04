// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Memory to register promotion optimization pass.

use core::{ops::ControlFlow, panic};

use alloc::vec::Vec;
use pliron_derive::op_interface;

use crate::{
    basic_block::BasicBlock,
    builtin::{
        given_names::set_block_arg_name,
        op_interfaces::{
            BranchOpInterface, RegionBranchOpInterface, RegionBranchPoint, RegionSuccessor,
        },
    },
    common_traits::Named,
    context::{Context, Ptr},
    graph::{
        dominance::{DomFrontierMap, DomInfo},
        find_ancestor_block_of_op_in_region,
        walkers::{
            Direction, IRNode, WALKCONFIG_PREORDER_FORWARD,
            interruptible::{walk_advance, walk_break},
            uninterruptible::immutable::walk_op,
        },
    },
    irbuild::{
        IRStatus,
        inserter::{IRInserter, Inserter},
        listener::{Recorder, RecorderEvent},
        rewriter::{IRRewriter, Rewriter},
    },
    linked_list::ContainsLinkedList,
    op::{Op, op_cast, op_impls},
    operation::{OpDbg, Operation},
    pass::{AnalysisManager, Pass, PassResult},
    region::Region,
    result::Result,
    r#type::TypeHandle,
    utils::{
        table::{HMap, HSet, IMap, ISet, htable},
        vec_exns::IterExtns,
    },
    value::Value,
};

/// Information about an allocation of memory that can potentially be promoted to a register.
#[derive(Clone)]
pub struct AllocInfo {
    /// Pointer to the allocated memory.
    pub ptr: Value,
    /// Type of the allocated memory.
    pub ty: TypeHandle,
}

/// A promotable memory allocation implements this interface
/// to provide the necessary info for register promotion.
#[op_interface]
pub trait PromotableAllocationInterface {
    /// Get the allocation info(s) for this operation.
    fn alloc_info(&self, ctx: &Context) -> Vec<AllocInfo>;

    /// Get the default value for an allocation. This is used
    /// when there's no reaching definition for a use. The `alloc_info`
    /// passed is guaranteed to be one of the entries returned by `alloc_info`.
    /// The inserter is positioned in the entry block. If the allocation
    /// is in the entry block, the inserter position will be before it.
    fn default_value(
        &self,
        ctx: &mut Context,
        inserter: &mut dyn Inserter,
        alloc_info: &AllocInfo,
    ) -> Result<Value>;

    /// Promote allocations. This is called after all uses have been promoted,
    /// and should remove the allocation (not necessarily the operation itself) from the IR.
    /// The `alloc_info`s passed are guaranteed to be from the entries returned by `alloc_info`.
    /// The rewriter is set to insert before this alloc op.
    fn promote(
        &self,
        ctx: &mut Context,
        rewriter: &mut dyn Rewriter,
        alloc_infos: &[AllocInfo],
    ) -> Result<()>;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// An operation implementing [PromotableOpInterface] provides this info.
pub enum PromotableOpKind {
    /// This operation is a load from the given allocation.
    Load,
    /// This operation is a store of the value to the given allocation.
    Store(Value),
    /// This operation uses the allocation, but that use can be removed.
    EliminatableUse,
    /// This operation either does not use the queried allocation at all,
    /// or uses it in a way that cannot be promoted to a register.
    NonPromotableUse,
}

/// Interface for operations that can be promoted to registers.
#[op_interface]
pub trait PromotableOpInterface {
    /// Get the kind of register promotion for this operation for `alloc_info`.
    ///
    /// **Note**: If this operation does **not** use `alloc_info.ptr`, this method MUST
    /// return [PromotableOpKind::NonPromotableUse].
    fn promotion_kind(&self, ctx: &Context, alloc_info: &AllocInfo) -> PromotableOpKind;

    /// Promote this operation for the provided `alloc_info`. This **typically** is:
    /// - For a load: Replace the load(ed) value with the reaching SSA definition provided.
    /// - For a store: Remove the store operation.
    /// - For an eliminatable use: The operation must no longer use the allocation.
    ///   This includes the case where the operation is removed entirely, but it may also
    ///   be that the operation is retained, but modified to no longer use the allocation pointer.
    /// - For a non-promotable use: No callback to this method will be made as such an allocation
    ///   will not even be considered for promotion.
    ///
    /// Only those `alloc_info`s for which [Self::promotion_kind] is not
    /// [PromotableOpKind::NonPromotableUse] will be passed to this method.
    ///
    /// The rewriter is set to insert before this promotable op.
    fn promote(
        &self,
        ctx: &mut Context,
        alloc_info_reaching_defs: &[(AllocInfo, Value)],
        rewriter: &mut dyn Rewriter,
    ) -> Result<()>;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Interface to enable promotion of allocations that are
/// - defined outside the op, and
/// - used in the regions of the op.
#[op_interface]
pub trait PromotableRegionBranchOpInterface: RegionBranchOpInterface {
    /// Can stores in the regions of this op be promoted?
    ///
    /// Some reasons to return false:
    /// - The regions can run concurrently, so a stored value may not reach a successor.
    /// - The op has no results or region arguments to receive a stored value.
    fn permits_store_promotion(&self, ctx: &Context) -> bool;

    /// Add a receiver of type `ty` in each of the `successors`.
    /// Receivers may also be added in other successors, when they share an operand list.
    /// Return all receivers added.
    fn add_receivers(
        &self,
        ctx: &mut Context,
        successors: &[RegionSuccessor],
        ty: TypeHandle,
    ) -> Vec<(RegionSuccessor, Value)>;

    /// For the `receivers` returned by [Self::add_receivers], add `operand` to the edges
    /// from `source` into their successors, so that it flows into those receivers.
    fn add_carried_operand(
        &self,
        ctx: &mut Context,
        source: RegionBranchPoint,
        receivers: &[(RegionSuccessor, Value)],
        operand: Value,
    );

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// A single promotable allocation: one [AllocInfo] from a [PromotableAllocationInterface] op.
#[derive(Clone)]
struct AllocCandidate {
    alloc_op: Ptr<Operation>,
    alloc_info: AllocInfo,
}

/// Collect all individual [AllocInfo]s from operations implementing
/// [PromotableAllocationInterface] rooted at `root`.
fn collect_alloc_candidates(root: Ptr<Operation>, ctx: &Context) -> Vec<AllocCandidate> {
    let mut candidates: Vec<AllocCandidate> = Vec::new();
    walk_op(
        ctx,
        &mut candidates,
        &WALKCONFIG_PREORDER_FORWARD,
        root,
        |ctx, candidates, node| {
            if let IRNode::Operation(op) = node {
                let op_obj = Operation::get_op_dyn(op, ctx);
                if let Some(iface) = op_cast::<dyn PromotableAllocationInterface>(op_obj.as_ref()) {
                    for alloc_info in iface.alloc_info(ctx) {
                        candidates.push(AllocCandidate {
                            alloc_op: op,
                            alloc_info,
                        });
                    }
                }
            }
        },
    );
    candidates
}

/// Prune allocation candidates that cannot be promoted. A candidate is removed if:
/// - any use of its pointer doesn't implement [PromotableOpInterface], or
/// - any use reports [PromotableOpKind::NonPromotableUse], or
/// - the regions enclosing any use do not [permit promotion](enclosing_regions_permit_promotion).
fn prune_candidates(candidates: &mut Vec<AllocCandidate>, ctx: &Context) {
    candidates.retain(|cand| {
        let alloc_region = cand
            .alloc_op
            .deref(ctx)
            .get_parent_region(ctx)
            .expect("Alloc op must be in a region");
        cand.alloc_info.ptr.uses(ctx).iter().all(|r#use| {
            let user_op = r#use.user_op();
            let user_op_obj = Operation::get_op_dyn(user_op, ctx);
            op_cast::<dyn PromotableOpInterface>(user_op_obj.as_ref()).is_some_and(|piface| {
                let promotion_kind = piface.promotion_kind(ctx, &cand.alloc_info);
                !matches!(promotion_kind, PromotableOpKind::NonPromotableUse)
                    && enclosing_regions_permit_promotion(
                        ctx,
                        user_op,
                        &promotion_kind,
                        alloc_region,
                    )
            })
        })
    });
}

/// Do the regions from `user_op` up to `alloc_region` permit promotion of its use?
///
/// Each such region must have SSA dominance, and its parent op must:
/// - implement [PromotableRegionBranchOpInterface], and
/// - [permit store promotion](PromotableRegionBranchOpInterface::permits_store_promotion)
///   if the use is a [PromotableOpKind::Store].
fn enclosing_regions_permit_promotion(
    ctx: &Context,
    user_op: Ptr<Operation>,
    promotion_kind: &PromotableOpKind,
    alloc_region: Ptr<Region>,
) -> bool {
    let is_store = matches!(promotion_kind, PromotableOpKind::Store(_));
    regions_up_to(ctx, user_op, alloc_region).all(|region| {
        let parent_op_obj = Operation::get_op_dyn(region.deref(ctx).get_parent_op(), ctx);
        region.deref(ctx).has_ssa_dominance(ctx)
            && op_cast::<dyn PromotableRegionBranchOpInterface>(parent_op_obj.as_ref())
                .is_some_and(|iface| !is_store || iface.permits_store_promotion(ctx))
    })
}

/// Get the regions from `op`'s [parent region](Operation::get_parent_region)
/// up to, but not including, `region`.
fn regions_up_to(
    ctx: &Context,
    op: Ptr<Operation>,
    region: Ptr<Region>,
) -> impl Iterator<Item = Ptr<Region>> + '_ {
    let start = op.deref(ctx).get_parent_region(ctx);
    core::iter::successors(start, move |region| {
        region
            .deref(ctx)
            .get_parent_op()
            .deref(ctx)
            .get_parent_region(ctx)
    })
    .take_while(move |&r| r != region)
}

/// Uses of candidates in nested regions, recorded for each region
/// from the use up to, but not including, the candidate's own region.
#[derive(Default)]
struct NestedUses {
    /// Candidate `ptr` -> (nested region that contains a use -> does it contain a store?).
    user_regions: HMap<Value, IMap<Ptr<Region>, bool>>,
    /// Op with a user region -> (candidate `ptr` -> do its regions contain a store?).
    user_region_ops: HMap<Ptr<Operation>, IMap<Value, bool>>,
}

impl NestedUses {
    /// Get an iterator over the candidates used in the regions of `op`.
    /// Each candidate `ptr` is paired with whether any region of `op` stores to it.
    fn candidates_used_in_regions_of(
        &self,
        op: Ptr<Operation>,
    ) -> impl Iterator<Item = (Value, bool)> + '_ {
        self.user_region_ops
            .get(&op)
            .into_iter()
            .flatten()
            .map(|(&ptr, &has_stores)| (ptr, has_stores))
    }

    /// Does `op` have a user region?
    fn has_user_regions(&self, op: Ptr<Operation>) -> bool {
        self.user_region_ops.contains_key(&op)
    }

    /// Does `region` contain a use of the candidate `ptr`?
    fn has_uses_in(&self, ptr: Value, region: Ptr<Region>) -> bool {
        self.user_regions
            .get(&ptr)
            .is_some_and(|regions| regions.contains_key(&region))
    }

    /// Get an iterator over the nested regions that contain uses of `cand`.
    /// `only_with_stores = true` filters out regions without a store.
    fn regions_of_cand(
        &self,
        cand: &AllocCandidate,
        only_with_stores: bool,
    ) -> impl Iterator<Item = Ptr<Region>> + '_ {
        self.user_regions
            .get(&cand.alloc_info.ptr)
            .into_iter()
            .flatten()
            .filter_map(move |(&region, &has_stores)| {
                (has_stores || !only_with_stores).then_some(region)
            })
    }

    /// Get the nested regions that contain uses of `candidates`.
    /// `only_with_stores = true` filters out regions without a store.
    fn regions_of_cands(
        &self,
        candidates: &[AllocCandidate],
        only_with_stores: bool,
    ) -> ISet<Ptr<Region>> {
        candidates
            .iter()
            .flat_map(|cand| self.regions_of_cand(cand, only_with_stores))
            .collect()
    }

    /// Get the ops that store to any of `candidates` in their regions.
    fn ops_with_cand_stores(
        &self,
        ctx: &Context,
        candidates: &[AllocCandidate],
    ) -> ISet<Ptr<Operation>> {
        candidates
            .iter()
            .flat_map(|cand| self.regions_of_cand(cand, true))
            .map(|region| region.deref(ctx).get_parent_op())
            .collect()
    }
}

/// Collect the [NestedUses] of `candidates`.
fn collect_nested_uses(candidates: &[AllocCandidate], ctx: &Context) -> NestedUses {
    let mut nested_uses = NestedUses::default();
    for cand in candidates {
        let ptr = cand.alloc_info.ptr;
        let alloc_region = cand
            .alloc_op
            .deref(ctx)
            .get_parent_region(ctx)
            .expect("Alloc op must be in a region");
        let mut regions: IMap<Ptr<Region>, bool> = IMap::default();

        // Record each region that encloses a use of `cand`,
        // and whether the region contains a store to `cand`.
        for r#use in ptr.uses(ctx) {
            let user_op = r#use.user_op();
            let user_op_obj = Operation::get_op_dyn(user_op, ctx);
            let is_store =
                op_cast::<dyn PromotableOpInterface>(user_op_obj.as_ref()).is_some_and(|piface| {
                    matches!(
                        piface.promotion_kind(ctx, &cand.alloc_info),
                        PromotableOpKind::Store(_)
                    )
                });
            for region in regions_up_to(ctx, user_op, alloc_region) {
                match regions.get(&region) {
                    // Another use already marked this region and the regions above it.
                    Some(&has_stores) if has_stores || !is_store => break,
                    _ => regions.insert(region, is_store),
                };
            }
        }

        // Merge the recorded regions into their parent ops.
        for (&region, &has_stores) in &regions {
            *nested_uses
                .user_region_ops
                .entry(region.deref(ctx).get_parent_op())
                .or_default()
                .entry(ptr)
                .or_default() |= has_stores;
        }
        if !regions.is_empty() {
            nested_uses.user_regions.insert(ptr, regions);
        }
    }
    nested_uses
}

/// Maps each `alloc_info.ptr` in a given `&[AllocCandidate]` to its index in that slice.
struct CandidateIndexMap(HMap<Value, usize>);

impl CandidateIndexMap {
    /// Compute a map of every `alloc_info.ptr` to its index in `candidates`.
    fn new(candidates: &[AllocCandidate]) -> Self {
        Self(
            candidates
                .iter()
                .enumerate()
                .map(|(cand_idx, cand)| (cand.alloc_info.ptr, cand_idx))
                .collect(),
        )
    }

    /// Get the indices of the candidates that `op` uses, in ascending order.
    fn candidates_accessed_by_op(&self, op: Ptr<Operation>, ctx: &Context) -> Vec<usize> {
        let mut accessed_cands: Vec<usize> = op
            .deref(ctx)
            .operands()
            .filter_map(|opd| self.0.get(&opd).copied())
            .collect();
        accessed_cands.sort_unstable();
        accessed_cands.dedup();
        accessed_cands
    }
}

/// Compute the live-in blocks and the defining blocks of a candidate.
///
/// Returns:
/// - `live_in`: blocks where the candidate's value is live-in.
/// - `defining_blocks`: blocks that contain a store defining the candidate.
fn compute_candidate_live_in_and_defining_blocks(
    ctx: &Context,
    cand_user_blocks_info: &IMap<Ptr<BasicBlock>, CandidateUserBlockInfo>,
) -> (HSet<Ptr<BasicBlock>>, ISet<Ptr<BasicBlock>>) {
    let mut defining_blocks: ISet<Ptr<BasicBlock>> = ISet::default();
    let mut live_in: HSet<Ptr<BasicBlock>> = HSet::default();
    let mut live_in_worklist: Vec<Ptr<BasicBlock>> = Vec::new();

    // A block is a defining block if it has any store to this candidate.
    // A block seeds liveness if it has a load/eliminatable use before the first store.
    for (&block, info) in cand_user_blocks_info {
        if info.has_store {
            defining_blocks.insert(block);
        }
        if info.load_before_store {
            live_in_worklist.push(block);
        }
    }

    // Propagate liveness backward through predecessors, stopping at defining blocks.
    while let Some(live_in_block) = live_in_worklist.pop() {
        if !live_in.insert(live_in_block) {
            continue;
        }
        for pred in live_in_block.preds(ctx) {
            if !defining_blocks.contains(&pred) {
                live_in_worklist.push(pred);
            }
        }
    }

    (live_in, defining_blocks)
}

/// Summary of the accesses to a candidate within one of its user blocks.
#[derive(Default)]
struct CandidateUserBlockInfo {
    /// The block stores to the candidate.
    has_store: bool,
    /// The block uses the candidate's value before storing to it.
    load_before_store: bool,
}

/// Map each candidate in `candidates` to its user blocks info: a [CandidateUserBlockInfo]
/// for each of its user [blocks in `region`](find_ancestor_block_of_op_in_region).
fn compute_user_blocks_info(
    ctx: &Context,
    region: Ptr<Region>,
    candidates: &[AllocCandidate],
    nested_uses: &NestedUses,
) -> Vec<IMap<Ptr<BasicBlock>, CandidateUserBlockInfo>> {
    let cand_index_map = CandidateIndexMap::new(candidates);
    let mut user_blocks: ISet<Ptr<BasicBlock>> = ISet::default();
    for cand in candidates {
        for u in cand.alloc_info.ptr.uses(ctx) {
            if let Some(block) = find_ancestor_block_of_op_in_region(ctx, u.user_op(), region) {
                user_blocks.insert(block);
            }
        }
    }

    let mut user_blocks_info: Vec<IMap<Ptr<BasicBlock>, CandidateUserBlockInfo>> =
        (0..candidates.len()).map(|_| IMap::default()).collect();
    for block in user_blocks {
        for op in block.iter(ctx) {
            // Handle uses in the regions of `op`, which are effectively `op`'s uses.
            for (ptr, has_stores) in nested_uses.candidates_used_in_regions_of(op) {
                let Some(&cand_idx) = cand_index_map.0.get(&ptr) else {
                    continue;
                };
                let info = user_blocks_info[cand_idx].entry(block).or_default();
                // Any use of a candidate in the regions of `op` is conservatively treated as a load.
                if !info.has_store {
                    info.load_before_store = true;
                }
                info.has_store |= has_stores;
            }

            let accessed_cands = cand_index_map.candidates_accessed_by_op(op, ctx);
            if accessed_cands.is_empty() {
                continue;
            }
            let op_obj = Operation::get_op_dyn(op, ctx);
            let op_promotable = op_cast::<dyn PromotableOpInterface>(op_obj.as_ref())
                .expect("User op of a candidate must implement PromotableOpInterface");
            for cand_idx in accessed_cands {
                let info = user_blocks_info[cand_idx].entry(block).or_default();
                match op_promotable.promotion_kind(ctx, &candidates[cand_idx].alloc_info) {
                    PromotableOpKind::Load | PromotableOpKind::EliminatableUse => {
                        if !info.has_store {
                            info.load_before_store = true;
                        }
                    }
                    PromotableOpKind::Store(_) => {
                        info.has_store = true;
                    }
                    PromotableOpKind::NonPromotableUse => {
                        unreachable!(
                            "prune_candidates removes candidates with a non-promotable use"
                        )
                    }
                }
            }
        }
    }

    /// Record uses of `candidates`, as region-edge carried operands, in terminators of `region`.
    fn record_terminator_uses(
        ctx: &Context,
        region: Ptr<Region>,
        nested_uses: &NestedUses,
        cand_index_map: &CandidateIndexMap,
        user_blocks_info: &mut [IMap<Ptr<BasicBlock>, CandidateUserBlockInfo>],
    ) {
        let op = region.deref(ctx).get_parent_op();
        let stored_cands: Vec<usize> = nested_uses
            .candidates_used_in_regions_of(op)
            .filter(|&(_, has_stores)| has_stores)
            .filter_map(|(ptr, _)| cand_index_map.0.get(&ptr).copied())
            .collect();
        if stored_cands.is_empty() {
            return;
        }

        let op_obj = Operation::get_op_dyn(op, ctx);
        let iface = op_cast::<dyn PromotableRegionBranchOpInterface>(op_obj.as_ref()).expect(
            "Op with stores in its regions must implement PromotableRegionBranchOpInterface",
        );
        let carrying_blocks: ISet<Ptr<BasicBlock>> = iface
            .region_edges(ctx)
            .into_iter()
            .filter_map(|edge| match edge.from {
                RegionBranchPoint::Terminator(term) => term
                    .deref(ctx)
                    .get_parent_block()
                    .filter(|block| block.deref(ctx).get_parent_region() == Some(region)),
                RegionBranchPoint::Parent => None,
            })
            .collect();
        for block in carrying_blocks {
            for &cand_idx in &stored_cands {
                let info = user_blocks_info[cand_idx].entry(block).or_default();
                if !info.has_store {
                    info.load_before_store = true;
                }
            }
        }
    }

    record_terminator_uses(
        ctx,
        region,
        nested_uses,
        &cand_index_map,
        &mut user_blocks_info,
    );

    user_blocks_info
}

/// Compute phi placement blocks for one candidate using liveness-pruned IDF.
///
/// Starting from the candidate's defining blocks, this computes the iterated
/// dominance frontier and keeps only blocks where the candidate is live-in.
fn compute_candidate_phi_blocks(
    df_map: &DomFrontierMap<Ptr<Region>, Context>,
    live_in: &HSet<Ptr<BasicBlock>>,
    defining_blocks: &ISet<Ptr<BasicBlock>>,
) -> ISet<Ptr<BasicBlock>> {
    // Compute liveness-pruned IDF for this candidate.
    let mut phi_blocks: ISet<Ptr<BasicBlock>> = ISet::default();
    let mut worklist: Vec<Ptr<BasicBlock>> = defining_blocks.iter().cloned().collect();
    while let Some(block) = worklist.pop() {
        for &df_block in df_map.frontier(&block) {
            // Prune early: only live-in blocks can host a useful phi.
            if !live_in.contains(&df_block) {
                continue;
            }
            if !phi_blocks.insert(df_block) {
                continue;
            }
            // Continue the IDF growth from newly inserted phi blocks.
            if !defining_blocks.contains(&df_block) {
                worklist.push(df_block);
            }
        }
    }

    phi_blocks
}

/// Add the phi placement blocks of `candidates` in `region` to `phi_blocks`.
fn compute_phi_blocks_in_region(
    ctx: &Context,
    dom_info: &mut DomInfo,
    region: Ptr<Region>,
    candidates: &[AllocCandidate],
    nested_uses: &NestedUses,
    phi_blocks: &mut HMap<Value, ISet<Ptr<BasicBlock>>>,
) {
    // Dominator tree and dominance frontier, once per region.
    let dom_tree = dom_info.get_dom_tree(ctx, region);
    let df_map = DomFrontierMap::new(ctx, &region, dom_tree);

    // Compute liveness and phi-placement per candidate.
    let user_blocks_info = compute_user_blocks_info(ctx, region, candidates, nested_uses);
    for (cand, cand_user_blocks_info) in candidates.iter().zip(&user_blocks_info) {
        let (live_in, defining_blocks) =
            compute_candidate_live_in_and_defining_blocks(ctx, cand_user_blocks_info);
        let candidate_phi_blocks =
            compute_candidate_phi_blocks(&df_map, &live_in, &defining_blocks);
        phi_blocks
            .entry(cand.alloc_info.ptr)
            .or_default()
            .extend(candidate_phi_blocks);
    }
}

/// Prune candidates that the rename walk cannot fully promote. A candidate is removed if:
/// - a predecessor of one of its phi blocks
///   - is unreachable, or
///   - does not have a terminator that implements [BranchOpInterface],
/// - a block that encloses one of its uses is unreachable, or
/// - a region edge into one of its receivers starts at an unreachable terminator.
///
/// This removes both:
/// - candidates from `alloc_candidates`, and
/// - corresponding entries from `phi_blocks`.
fn prune_candidates_with_unfillable_operands(
    ctx: &Context,
    dom_info: &DomInfo,
    nested_uses: &NestedUses,
    ops_with_cand_stores: &ISet<Ptr<Operation>>,
    alloc_candidates: &mut Vec<AllocCandidate>,
    phi_blocks: &mut HMap<Value, ISet<Ptr<BasicBlock>>>,
) {
    /// Is `block` reachable from the entry of its region?
    fn is_reachable(ctx: &Context, dom_info: &DomInfo, block: Ptr<BasicBlock>) -> bool {
        let region = block
            .deref(ctx)
            .get_parent_region()
            .expect("Block must be in a region");
        dom_info
            .get_cached_dom_tree(region)
            .expect("Dominator tree of the region must be cached")
            .contains(&block)
    }

    /// Is any block from `op`'s parent block up to its ancestor block in `region` unreachable?
    fn has_unreachable_ancestor_block(
        ctx: &Context,
        dom_info: &DomInfo,
        op: Ptr<Operation>,
        region: Ptr<Region>,
    ) -> bool {
        let mut op = op;
        loop {
            let block = op
                .deref(ctx)
                .get_parent_block()
                .expect("Op must be in a block");
            if !is_reachable(ctx, dom_info, block) {
                return true;
            }
            let block_region = block
                .deref(ctx)
                .get_parent_region()
                .expect("Block must be in a region");
            if block_region == region {
                return false;
            }
            op = block_region.deref(ctx).get_parent_op();
        }
    }

    // Ops in which a region edge into receivers starts at an unreachable terminator.
    let ops_with_unreachable_sources: HSet<Ptr<Operation>> = ops_with_cand_stores
        .iter()
        .copied()
        .filter(|&op| {
            let op_obj = Operation::get_op_dyn(op, ctx);
            let iface = op_cast::<dyn PromotableRegionBranchOpInterface>(op_obj.as_ref()).expect(
                "Op with stores in its regions must implement PromotableRegionBranchOpInterface",
            );
            iface.region_edges(ctx).iter().any(|edge| match edge.from {
                RegionBranchPoint::Terminator(term) => {
                    let block = term
                        .deref(ctx)
                        .get_parent_block()
                        .expect("Terminator must be in a block");
                    !is_reachable(ctx, dom_info, block)
                }
                RegionBranchPoint::Parent => false,
            })
        })
        .collect();

    // Track invalid individual candidates by their allocation pointer.
    let mut invalid_ptrs: HSet<Value> = HSet::default();
    for cand in alloc_candidates.iter() {
        let ptr = cand.alloc_info.ptr;
        let alloc_region = cand
            .alloc_op
            .deref(ctx)
            .get_parent_region(ctx)
            .expect("Alloc op must be in a region");
        let invalid = phi_blocks
            .get(&ptr)
            .into_iter()
            .flatten()
            .flat_map(|&phi_block| phi_block.preds(ctx).into_iter())
            // If any predecessor is unreachable, or its terminator does not implement
            // BranchOpInterface, we won't be able to fill phi operands for this candidate.
            .any(|pred| {
                !is_reachable(ctx, dom_info, pred)
                    || pred.deref(ctx).get_terminator(ctx).is_none_or(|term| {
                        !op_impls::<dyn BranchOpInterface>(
                            Operation::get_op_dyn(term, ctx).as_ref(),
                        )
                    })
            })
            // The rename walk does not promote uses inside unreachable blocks.
            || ptr.uses(ctx).iter().any(|r#use| {
                has_unreachable_ancestor_block(ctx, dom_info, r#use.user_op(), alloc_region)
            })
            // An unreachable terminator cannot carry values into the receivers.
            || nested_uses.regions_of_cand(cand, true).any(|region| {
                ops_with_unreachable_sources.contains(&region.deref(ctx).get_parent_op())
            });
        if invalid {
            invalid_ptrs.insert(ptr);
        }
    }

    alloc_candidates.retain(|c| !invalid_ptrs.contains(&c.alloc_info.ptr));
    phi_blocks.retain(|&ptr, _| !invalid_ptrs.contains(&ptr));
}

/// Add block arguments for `candidates` in their `phi_blocks`.
fn add_phis(
    ctx: &mut Context,
    candidates: &[AllocCandidate],
    phi_blocks: &HMap<Value, ISet<Ptr<BasicBlock>>>,
) -> HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>> {
    let mut new_phis_in_block: HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>> =
        HMap::default();
    for cand in candidates {
        let ptr = cand.alloc_info.ptr;
        for &phi_block in phi_blocks.get(&ptr).into_iter().flatten() {
            let arg_idx = BasicBlock::push_argument(phi_block, ctx, cand.alloc_info.ty);
            set_block_arg_name(ctx, phi_block, arg_idx, ptr.given_name(ctx));
            new_phis_in_block
                .entry(phi_block)
                .or_default()
                .push((cand.clone(), arg_idx));
        }
    }
    new_phis_in_block
}

fn get_or_create_default_def(
    alloc_cand: &AllocCandidate,
    ctx: &mut Context,
    default_defs: &mut HMap<Value, Value>,
) -> Result<Value> {
    match default_defs.entry(alloc_cand.alloc_info.ptr) {
        htable::Entry::Occupied(entry) => Ok(*entry.get()),
        htable::Entry::Vacant(entry) => {
            let alloc_op = alloc_cand.alloc_op;
            let alloc_obj = Operation::get_op_dyn(alloc_op, ctx);
            let alloc_iface = op_cast::<dyn PromotableAllocationInterface>(alloc_obj.as_ref())
                .expect("Alloc op must implement PromotableAllocationInterface");

            // The default value must be at a place that dominates the alloc
            // and all places that the promoted value may be live at. The safest
            // such point is in the entry block, before the alloc itself.
            let alloc_block = alloc_op
                .deref(ctx)
                .get_parent_block()
                .expect("Alloc op must be in a block");
            let alloc_region = alloc_block
                .deref(ctx)
                .get_parent_region()
                .expect("Alloc op must be in a region");

            let alloc_region_entry = alloc_region
                .deref(ctx)
                .get_entry_block()
                .expect("Region must have entry block");
            let mut inserter = if alloc_region_entry == alloc_block {
                IRInserter::<Recorder>::new_before_operation(alloc_op)
            } else {
                IRInserter::<Recorder>::new_before_block_terminator(alloc_region_entry, ctx)
            };

            let default_val =
                alloc_iface.default_value(ctx, &mut inserter, &alloc_cand.alloc_info)?;
            entry.insert(default_val);
            Ok(default_val)
        }
    }
}

/// Process the events in the recorder to note down erased operations.
fn note_erased_ops(recorder: &mut Recorder, erased: &mut HSet<Ptr<Operation>>) {
    for event in recorder.events.drain(..) {
        match event {
            RecorderEvent::ErasedOperation(op) => {
                erased.insert(op);
            }
            RecorderEvent::ErasedBlock(_)
            | RecorderEvent::ErasedRegion(_)
            | RecorderEvent::UnlinkedBlock(_, _) => {
                panic!("mem2reg rewrite (promotion) call backs must not alter control flow");
            }
            RecorderEvent::InsertedOperation(_)
            | RecorderEvent::InsertedBlock(_)
            | RecorderEvent::ReplacedValueUses { .. }
            | RecorderEvent::ValueTypeChanged { .. }
            | RecorderEvent::UnlinkedOperation(_, _) => {
                // No action needed for these events in this context.
            }
        }
    }
}

/// State of the SSA rename walk over the dominator tree.
struct RenameState<'a> {
    ctx: &'a mut Context,
    /// The stack of reaching definitions for each promotable allocation pointer.
    reaching_defs: HMap<Value, Vec<Value>>,
    /// Record (`block`, `ptr`) of each new reaching definition of `ptr` in `block`.
    renames_in_block: Vec<(Ptr<BasicBlock>, Value)>,
    /// The default value (if already created) for each promotable allocation pointer.
    default_def_map: HMap<Value, Value>,
    /// The candidates being promoted.
    alloc_candidates: &'a [AllocCandidate],
    /// Finds the candidates (as indices in [Self::alloc_candidates]) that an op uses.
    cand_index_map: CandidateIndexMap,
    /// Dominator trees of all concerned regions.
    dom_info: &'a DomInfo,
    /// Uses of the candidates in nested regions.
    nested_uses: &'a NestedUses,
    /// PHIs (candidate, block arg index) added to a block.
    new_phis_in_block: &'a HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>>,
    /// Receivers added for the candidates stored to in the regions of an op.
    new_receivers_in_op: IMap<Ptr<Operation>, Vec<CandidateReceivers>>,
}

impl RenameState<'_> {
    /// Push `def` as the current reaching definition of `ptr`.
    fn push_def(&mut self, block: Ptr<BasicBlock>, ptr: Value, def: Value) {
        self.reaching_defs.get_mut(&ptr).unwrap().push(def);
        self.renames_in_block.push((block, ptr));
    }

    /// Pop all reaching definitions that were pushed in `block`.
    ///
    /// Only the top entries of [Self::renames_in_block]
    /// are compared with `block` and popped.
    fn pop_defs(&mut self, block: Ptr<BasicBlock>) {
        while let Some((_, ptr)) = self
            .renames_in_block
            .pop_if(|(rename_block, _)| *rename_block == block)
        {
            self.reaching_defs.get_mut(&ptr).unwrap().pop();
        }
    }

    /// Get the current reaching definition of `cand`.
    /// Gets or creates the default value if there is no reaching definition.
    fn current_def(&mut self, cand: &AllocCandidate) -> Result<Value> {
        match self.reaching_defs[&cand.alloc_info.ptr].last() {
            Some(&def) => Ok(def),
            None => get_or_create_default_def(cand, self.ctx, &mut self.default_def_map),
        }
    }
}

/// Process one block during SSA rename for mem2reg.
///
/// Goes through the block from start to end and:
/// - Records the current reaching definitions.
/// - Promotes the uses of the allocations.
/// - Carries the definitions at its end along region edges from its terminator.
/// - Fills the phi operands of the successors.
fn process_rename_block(state: &mut RenameState, block: Ptr<BasicBlock>) -> Result<()> {
    let alloc_candidates = state.alloc_candidates;
    let new_phis_in_block = state.new_phis_in_block;
    // Push phi args for this block.
    for &(ref cand, arg_idx) in new_phis_in_block.get(&block).into_iter().flatten() {
        let new_val = block.deref(state.ctx).get_argument(arg_idx);
        state.push_def(block, cand.alloc_info.ptr, new_val);
    }

    let ops: Vec<Ptr<Operation>> = block.deref(state.ctx).iter(state.ctx).collect();
    let mut erased_ops = HSet::default();
    for &op in &ops {
        if erased_ops.contains(&op) {
            continue;
        }
        // Rename uses in nested regions of `op`.
        if state.nested_uses.has_user_regions(op) {
            rename_in_regions_of(state, op)?;
        }
        let accessed_cands = state
            .cand_index_map
            .candidates_accessed_by_op(op, state.ctx);
        if accessed_cands.is_empty() {
            continue;
        }
        let op_obj = Operation::get_op_dyn(op, state.ctx);
        let piface = op_cast::<dyn PromotableOpInterface>(op_obj.as_ref())
            .expect("User op of a candidate must implement PromotableOpInterface");

        let mut promote_queue = Vec::new();
        for cand_idx in accessed_cands {
            let cand = &alloc_candidates[cand_idx];
            match piface.promotion_kind(state.ctx, &cand.alloc_info) {
                PromotableOpKind::Load | PromotableOpKind::EliminatableUse => {
                    let current_def = state.current_def(cand)?;
                    promote_queue.push((cand.alloc_info.clone(), current_def));
                }
                PromotableOpKind::Store(stored_val) => {
                    state.push_def(block, cand.alloc_info.ptr, stored_val);
                    promote_queue.push((cand.alloc_info.clone(), stored_val));
                }
                PromotableOpKind::NonPromotableUse => {
                    unreachable!("prune_candidates removes candidates with a non-promotable use")
                }
            }
        }
        if !promote_queue.is_empty() {
            let rewriter = &mut IRRewriter::default();
            rewriter.set_insertion_point_before_operation(op);
            log::trace!("Promoting op {}", OpDbg { op, ctx: state.ctx });
            piface.promote(state.ctx, &promote_queue, rewriter)?;
            note_erased_ops(rewriter.get_listener_mut(), &mut erased_ops);
        }
    }

    // Carry the definitions at the end of `block` along region edges from its terminator.
    if !state.new_receivers_in_op.is_empty() {
        let terminator = block.deref(state.ctx).get_terminator(state.ctx);
        if let Some(terminator) = terminator {
            let op = block
                .deref(state.ctx)
                .get_parent_region()
                .expect("Block must be in a region")
                .deref(state.ctx)
                .get_parent_op();
            add_carried_operands(state, op, RegionBranchPoint::Terminator(terminator))?;
        }
    }

    // Fill phi operands in successor branch ops.
    let succs = block.deref(state.ctx).succs(state.ctx);
    for (succ_idx, new_phis_in_succ) in succs.iter().enumerate().filter_map(|(succ_idx, succ)| {
        new_phis_in_block
            .get(succ)
            .map(|new_phis| (succ_idx, new_phis))
    }) {
        let term = block
            .deref(state.ctx)
            .get_terminator(state.ctx)
            .expect("Block has successors but no terminator");
        let term_obj = Operation::get_op_dyn(term, state.ctx);
        let branch_iface = op_cast::<dyn BranchOpInterface>(term_obj.as_ref())
            .expect("Terminator must implement BranchOpInterface for phi blocks");
        for &(ref cand, arg_idx) in new_phis_in_succ {
            let current_def = state.current_def(cand)?;
            let succ_opd_idx = branch_iface.add_successor_operand(state.ctx, succ_idx, current_def);
            assert!(succ_opd_idx == arg_idx, "Mismatched phi argument index");
        }
    }

    Ok(())
}

/// Promote all uses of allocation pointers in `region` to SSA values:
/// - Replace loads with reaching definitions, and remove stores.
/// - Fill successor phi operands for each processed block.
/// - Fill the carried operands of region edges into receivers.
fn rename_blocks(
    ctx: &mut Context,
    dom_info: &DomInfo,
    nested_uses: &NestedUses,
    region: Ptr<Region>,
    new_phis_in_block: &HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>>,
    new_receivers_in_op: IMap<Ptr<Operation>, Vec<CandidateReceivers>>,
    alloc_candidates: &[AllocCandidate],
) -> Result<()> {
    // All candidates have an empty reaching defs stack initially.
    let reaching_defs = alloc_candidates
        .iter()
        .map(|cand| (cand.alloc_info.ptr, Vec::new()))
        .collect();

    let mut state = RenameState {
        ctx,
        reaching_defs,
        renames_in_block: Vec::new(),
        default_def_map: HMap::default(),
        alloc_candidates,
        cand_index_map: CandidateIndexMap::new(alloc_candidates),
        dom_info,
        nested_uses,
        new_phis_in_block,
        new_receivers_in_op,
    };

    rename_region_blocks(&mut state, region)
}

/// Process the blocks of `region` in dominator tree order.
fn rename_region_blocks(state: &mut RenameState, region: Ptr<Region>) -> Result<()> {
    let dom_tree = state
        .dom_info
        .get_cached_dom_tree(region)
        .expect("Dominator tree of a region to rename must be cached");
    let res = dom_tree.walk(
        state,
        Direction::Forward,
        |state, block| match process_rename_block(state, block) {
            Ok(()) => walk_advance(),
            Err(err) => walk_break(err),
        },
        |state, block| {
            // The children of `block` have popped their definitions already,
            // so the definitions of `block` (if any) are at the top.
            state.pop_defs(block);
            walk_advance()
        },
    );

    match res {
        ControlFlow::Break(err) => Err(err),
        ControlFlow::Continue(_) => Ok(()),
    }
}

/// Receivers added for a candidate stored to in the regions of an op.
struct CandidateReceivers {
    cand: AllocCandidate,
    /// (Successor, receiver) for each successor that
    /// [add_receivers](PromotableRegionBranchOpInterface::add_receivers) added to.
    receivers: Vec<(RegionSuccessor, Value)>,
    /// Distinct sources of all edges into `receivers`.
    sources: Vec<RegionBranchPoint>,
}

impl CandidateReceivers {
    /// Get the receiver of `successor`, if any.
    fn receiver(&self, successor: RegionSuccessor) -> Option<Value> {
        self.receivers
            .iter()
            .find(|(succ, _)| *succ == successor)
            .map(|&(_, receiver)| receiver)
    }

    /// Is there an edge from `source` into any of [Self::receivers]?
    fn has_source(&self, source: RegionBranchPoint) -> bool {
        self.sources.contains(&source)
    }
}

/// Carry the current definitions along the edges from `source` into the receivers in `op`.
fn add_carried_operands(
    state: &mut RenameState,
    op: Ptr<Operation>,
    source: RegionBranchPoint,
) -> Result<()> {
    // Take the entry out of the map, so that `current_def` can borrow `state`.
    let Some(op_receivers) = state.new_receivers_in_op.swap_remove(&op) else {
        return Ok(());
    };
    let op_obj = Operation::get_op_dyn(op, state.ctx);
    let iface = op_cast::<dyn PromotableRegionBranchOpInterface>(op_obj.as_ref())
        .expect("Region op must implement PromotableRegionBranchOpInterface");
    for cand_receivers in op_receivers.iter().filter(|c| c.has_source(source)) {
        let operand = state.current_def(&cand_receivers.cand)?;
        iface.add_carried_operand(state.ctx, source, &cand_receivers.receivers, operand);
    }
    state.new_receivers_in_op.insert(op, op_receivers);
    Ok(())
}

/// For `candidates` stored to in the regions of an op,
/// add a receiver in each successor that a terminator of the op enters.
///
/// This is the region control-flow analogue of [add_phis].
fn add_receivers(
    ctx: &mut Context,
    nested_uses: &NestedUses,
    candidates: &[AllocCandidate],
) -> IMap<Ptr<Operation>, Vec<CandidateReceivers>> {
    // Gather the candidates that are stored to in each region op.
    let mut stored_in_ops: IMap<Ptr<Operation>, Vec<&AllocCandidate>> = IMap::default();
    for cand in candidates {
        let ops = nested_uses.ops_with_cand_stores(ctx, core::slice::from_ref(cand));
        for op in ops {
            stored_in_ops.entry(op).or_default().push(cand);
        }
    }

    let mut new_receivers_in_op: IMap<Ptr<Operation>, Vec<CandidateReceivers>> = IMap::default();
    for (op, cands) in stored_in_ops {
        let op_obj = Operation::get_op_dyn(op, ctx);
        let iface = op_cast::<dyn PromotableRegionBranchOpInterface>(op_obj.as_ref()).expect(
            "Op with promotable uses in its regions must implement PromotableRegionBranchOpInterface",
        );
        // Get all the edges before any receiver is added. Later calls to `region_edges`
        // are not valid until the receivers get their carried operands.
        let edges: Vec<(RegionBranchPoint, RegionSuccessor)> = iface
            .region_edges(ctx)
            .into_iter()
            .map(|edge| (edge.from, edge.to))
            .collect();
        // Only edges from a terminator can carry a value stored inside a region. So, only
        // *their* successors need a receiver to merge the values from all their edges.
        let entered_from_terminator: Vec<RegionSuccessor> = edges
            .iter()
            .filter_map(|&(from, to)| {
                matches!(from, RegionBranchPoint::Terminator(_)).then_some(to)
            })
            .collect_unique();
        let mut op_receivers = Vec::with_capacity(cands.len());
        for cand in cands {
            let ptr = cand.alloc_info.ptr;
            let receivers = iface.add_receivers(ctx, &entered_from_terminator, cand.alloc_info.ty);
            for &(_, receiver) in &receivers {
                receiver.set_name(ctx, ptr.given_name(ctx));
            }
            // Collect the distinct sources of all edges that enter `receivers`.
            let sources = edges
                .iter()
                .filter(|(_, to)| receivers.iter().any(|(succ, _)| succ == to))
                .map(|&(from, _)| from)
                .collect_unique();
            op_receivers.push(CandidateReceivers {
                cand: cand.clone(),
                receivers,
                sources,
            });
        }
        new_receivers_in_op.insert(op, op_receivers);
    }
    new_receivers_in_op
}

/// Promote the uses of candidates in `op`'s regions.
///
/// - If no region of `op` stores to a candidate, its uses get the definition that reaches `op`.
/// - Otherwise:
///   - Each successor that a terminator enters has a receiver for the candidate.
///   - Every edge into such a successor carries the definition that reaches its source.
fn rename_in_regions_of(state: &mut RenameState, op: Ptr<Operation>) -> Result<()> {
    let nested_uses = state.nested_uses;
    let used_ptrs: Vec<Value> = nested_uses
        .candidates_used_in_regions_of(op)
        .filter_map(|(ptr, _)| state.cand_index_map.0.contains_key(&ptr).then_some(ptr))
        .collect();

    // Carry the definitions that reach `op` along the edges from `op` itself.
    add_carried_operands(state, op, RegionBranchPoint::Parent)?;
    let has_receivers = state.new_receivers_in_op.contains_key(&op);

    let regions: Vec<Ptr<Region>> = op.deref(state.ctx).regions().collect();
    for region in regions {
        let Some(entry) = region.deref(state.ctx).get_entry_block() else {
            continue;
        };
        // We need to process the region if either of these two hold:
        // - `region` has receivers, which its terminators may carry.
        // - `region` uses a candidate.
        if !has_receivers
            && !used_ptrs
                .iter()
                .any(|&ptr| nested_uses.has_uses_in(ptr, region))
        {
            continue;
        }
        // Each candidate with a receiver in `region` starts with that receiver.
        // A candidate without one starts with the definition that reaches `op`,
        // which is already at the top of the stack.
        let region_receivers: Vec<(Value, Value)> = state
            .new_receivers_in_op
            .get(&op)
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let receiver = c.receiver(RegionSuccessor::Region(region))?;
                Some((c.cand.alloc_info.ptr, receiver))
            })
            .collect();
        for (ptr, receiver) in region_receivers {
            state.push_def(entry, ptr, receiver);
        }
        rename_region_blocks(state, region)?;
    }

    // Update the reaching definitions to the receivers added as results of `op`, if any.
    if let Some(op_receivers) = state.new_receivers_in_op.swap_remove(&op) {
        let block = op
            .deref(state.ctx)
            .get_parent_block()
            .expect("Op must be in a block");
        for cand_receivers in &op_receivers {
            if let Some(receiver) = cand_receivers.receiver(RegionSuccessor::After) {
                state.push_def(block, cand_receivers.cand.alloc_info.ptr, receiver);
            }
        }
    }

    Ok(())
}

/// Perform memory to register promotion on regions within root.
pub fn mem2reg(
    root: Ptr<Operation>,
    ctx: &mut Context,
    analyses: &mut AnalysisManager,
) -> Result<IRStatus> {
    // Collect allocations that implement the promotable allocation interface.
    let mut candidates = collect_alloc_candidates(root, ctx);
    // Prune candidates that we cannot promote based on their uses.
    prune_candidates(&mut candidates, ctx);
    let nested_uses = collect_nested_uses(&candidates, ctx);

    if candidates.is_empty() {
        return Ok(IRStatus::Unchanged);
    }

    // Categorize by region for efficiency.
    // We process all candidates in a region together to amortize
    // the cost of computing dominator trees, frontiers, etc.
    let mut by_region: IMap<Ptr<Region>, Vec<AllocCandidate>> = IMap::default();
    for cand in candidates {
        let region = cand
            .alloc_op
            .deref(ctx)
            .get_parent_region(ctx)
            .expect("Alloc op must be in a region");
        by_region.entry(region).or_default().push(cand);
    }

    let dom_info = &mut *analyses.get_analysis_mut::<DomInfo>(root, ctx)?;

    let mut opt_status = IRStatus::Unchanged;
    for (region, mut alloc_candidates) in by_region {
        let mut phi_blocks: HMap<Value, ISet<Ptr<BasicBlock>>> = HMap::default();
        // Compute PHIs in the candidates' own region.
        compute_phi_blocks_in_region(
            ctx,
            dom_info,
            region,
            &alloc_candidates,
            &nested_uses,
            &mut phi_blocks,
        );
        // Compute PHIs in nested regions with stores.
        for nested_region in nested_uses.regions_of_cands(&alloc_candidates, true) {
            compute_phi_blocks_in_region(
                ctx,
                dom_info,
                nested_region,
                &alloc_candidates,
                &nested_uses,
                &mut phi_blocks,
            );
        }

        // The pruning and the rename walk require dominator trees of all nested regions.
        // 1. Nested regions that contain a use of the candidates.
        for nested_region in nested_uses.regions_of_cands(&alloc_candidates, false) {
            dom_info.get_dom_tree(ctx, nested_region);
        }
        // 2. Nested regions of ops that will get receivers.
        let ops_with_cand_stores = nested_uses.ops_with_cand_stores(ctx, &alloc_candidates);
        for &op in &ops_with_cand_stores {
            let regions: Vec<Ptr<Region>> = op.deref(ctx).regions().collect();
            for region in regions {
                dom_info.get_dom_tree(ctx, region);
            }
        }

        // Remove candidates that cannot be fully promoted by the rename walk.
        prune_candidates_with_unfillable_operands(
            ctx,
            dom_info,
            &nested_uses,
            &ops_with_cand_stores,
            &mut alloc_candidates,
            &mut phi_blocks,
        );
        if alloc_candidates.is_empty() {
            continue;
        }
        opt_status |= IRStatus::Changed;

        // Add block arguments for phis.
        let new_phis_in_block = add_phis(ctx, &alloc_candidates, &phi_blocks);

        // Add receivers for the candidates stored to in nested regions.
        let new_receivers_in_op = add_receivers(ctx, &nested_uses, &alloc_candidates);

        // SSA rename via dominator tree walk, starting from the entry block.
        rename_blocks(
            ctx,
            dom_info,
            &nested_uses,
            region,
            &new_phis_in_block,
            new_receivers_in_op,
            &alloc_candidates,
        )?;

        // "Promote" (remove) the allocations themselves. Group them into
        // a single promote call per alloc op and then invoke the interface method.
        let mut alloc_op_to_infos: IMap<Ptr<Operation>, Vec<AllocInfo>> = IMap::default();
        let rewriter = &mut IRRewriter::default();

        for cand in alloc_candidates.iter() {
            alloc_op_to_infos
                .entry(cand.alloc_op)
                .or_default()
                .push(cand.alloc_info.clone());
        }

        let mut erased_ops = HSet::default();
        for (op, infos) in alloc_op_to_infos {
            if erased_ops.contains(&op) {
                panic!("Alloc op was already erased during promotion of another candidate");
            }
            rewriter.set_insertion_point_before_operation(op);
            let op = Operation::get_op_dyn(op, ctx);
            let piface = op_cast::<dyn PromotableAllocationInterface>(op.as_ref())
                .expect("Alloc op must implement PromotableAllocationInterface");
            log::debug!(
                "Promoting allocation {}",
                OpDbg {
                    op: op.get_operation(),
                    ctx
                }
            );
            piface.promote(ctx, rewriter, &infos)?;
            note_erased_ops(rewriter.get_listener_mut(), &mut erased_ops);
        }
    }

    Ok(opt_status)
}

#[derive(Default)]
/// The mem2reg pass, which promotes memory allocations to SSA registers where possible.
pub struct Mem2RegPass;

impl Pass for Mem2RegPass {
    fn name(&self) -> &str {
        "mem2reg"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_res = PassResult::default();
        // Run mem2reg on the entire operation tree rooted at `op`
        pass_res.ir_changed |= mem2reg(op, ctx, analyses)?;
        // mem2reg does not touch the CFG structure, so we can preserve dominator info if it exists.
        pass_res.set_preserved::<DomInfo>();
        Ok(pass_res)
    }
}
