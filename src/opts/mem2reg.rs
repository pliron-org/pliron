// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Memory to register promotion optimization pass.

use core::{ops::ControlFlow, panic};

use alloc::vec::Vec;
use pliron_derive::op_interface;
use smallvec::SmallVec;

use crate::{
    basic_block::BasicBlock,
    builtin::{given_names::set_block_arg_name, op_interfaces::BranchOpInterface},
    common_traits::Named,
    context::{Context, Ptr},
    graph::{
        dominance::{DomFrontierMap, DomInfo, DomTree},
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
    utils::table::{HMap, HSet, IMap, ISet, htable},
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
/// - any use of its pointer doesn't implement [PromotableOpInterface],
/// - any use reports [PromotableOpKind::NonPromotableUse], or
/// - any use is in a different region from the allocation.
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
                let user_region = user_op
                    .deref(ctx)
                    .get_parent_region(ctx)
                    .expect("Use op must be in a region");
                let promotion_kind = piface.promotion_kind(ctx, &cand.alloc_info);
                user_region == alloc_region
                    && !matches!(promotion_kind, PromotableOpKind::NonPromotableUse)
            })
        })
    });
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
    fn candidates_accessed_by_op(&self, op: Ptr<Operation>, ctx: &Context) -> SmallVec<[usize; 2]> {
        let mut accessed_cands: SmallVec<[usize; 2]> = op
            .deref(ctx)
            .operands()
            .filter_map(|opd| self.0.get(&opd).copied())
            .collect();
        accessed_cands.sort_unstable();
        accessed_cands.dedup();
        accessed_cands
    }
}

/// Compute per-candidate liveness anchors for mem2reg analysis.
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

/// Map each candidate in `candidates` to its user blocks info:
/// a [CandidateUserBlockInfo] for each of its user blocks.
fn compute_user_blocks_info(
    ctx: &Context,
    candidates: &[AllocCandidate],
) -> Vec<IMap<Ptr<BasicBlock>, CandidateUserBlockInfo>> {
    let cand_index_map = CandidateIndexMap::new(candidates);
    let mut user_blocks: ISet<Ptr<BasicBlock>> = ISet::default();
    for cand in candidates {
        for u in cand.alloc_info.ptr.uses(ctx) {
            if let Some(block) = u.user_op().deref(ctx).get_parent_block() {
                user_blocks.insert(block);
            }
        }
    }

    let mut user_blocks_info: Vec<IMap<Ptr<BasicBlock>, CandidateUserBlockInfo>> =
        (0..candidates.len()).map(|_| IMap::default()).collect();
    for block in user_blocks {
        for op in block.iter(ctx) {
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

/// Prune candidates whose new block args cannot be populated because a predecessor
/// terminator does not implement [BranchOpInterface].
///
/// This removes both:
/// - candidates from `alloc_candidates`, and
/// - corresponding entries from `phi_blocks`.
fn prune_candidates_with_unknown_branch_from_pred(
    ctx: &Context,
    alloc_candidates: &mut Vec<AllocCandidate>,
    phi_blocks: &mut HMap<Value, ISet<Ptr<BasicBlock>>>,
) {
    // Track invalid individual candidates by their allocation pointer.
    let mut invalid_ptrs: HSet<Value> = HSet::default();
    for cand in alloc_candidates.iter() {
        let ptr = cand.alloc_info.ptr;
        let invalid = phi_blocks
            .get(&ptr)
            .into_iter()
            .flatten()
            .flat_map(|&phi_block| phi_block.preds(ctx).into_iter())
            // If any predecessor terminator does not implement BranchOpInterface,
            // we won't be able to fill phi operands for this candidate, so prune it.
            .any(|pred| {
                pred.deref(ctx).get_terminator(ctx).is_none_or(|term| {
                    !op_impls::<dyn BranchOpInterface>(Operation::get_op_dyn(term, ctx).as_ref())
                })
            });
        if invalid {
            invalid_ptrs.insert(ptr);
        }
    }

    alloc_candidates.retain(|c| !invalid_ptrs.contains(&c.alloc_info.ptr));
    phi_blocks.retain(|&ptr, _| !invalid_ptrs.contains(&ptr));
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
/// - Fills the phi operands of the successors.
fn process_rename_block(
    state: &mut RenameState,
    block: Ptr<BasicBlock>,
    new_phis_in_block: &HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>>,
) -> Result<()> {
    let alloc_candidates = state.alloc_candidates;
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

/// Rename uses of allocation pointers to SSA values across the dominator tree,
/// replacing loads/stores with reaching definitions and filling successor phi
/// operands for each processed block.
fn rename_blocks(
    ctx: &mut Context,
    dom_tree: &DomTree<Ptr<Region>, Context>,
    new_phis_in_block: &HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>>,
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
    };

    let res = dom_tree.walk(
        &mut state,
        Direction::Forward,
        |state, block| match process_rename_block(state, block, new_phis_in_block) {
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
        // Dominator tree and dominance frontier, once per region.
        let dom_tree = dom_info.get_dom_tree(ctx, region);
        let df_map = DomFrontierMap::new(ctx, &region, dom_tree);

        // Compute liveness and phi-placement per candidate.
        let user_blocks_info = compute_user_blocks_info(ctx, &alloc_candidates);
        let mut phi_blocks: HMap<Value, ISet<Ptr<BasicBlock>>> = HMap::default();
        for (cand, cand_user_blocks_info) in alloc_candidates.iter().zip(&user_blocks_info) {
            let ptr = cand.alloc_info.ptr;
            let (live_in, defining_blocks) =
                compute_candidate_live_in_and_defining_blocks(ctx, cand_user_blocks_info);
            let candidate_phi_blocks =
                compute_candidate_phi_blocks(&df_map, &live_in, &defining_blocks);
            phi_blocks.insert(ptr, candidate_phi_blocks);
        }

        // Remove candidates where phi predecessors cannot forward values.
        prune_candidates_with_unknown_branch_from_pred(ctx, &mut alloc_candidates, &mut phi_blocks);
        if alloc_candidates.is_empty() {
            continue;
        }
        opt_status |= IRStatus::Changed;

        // Add block arguments for phis, record arg indices.
        let mut new_phis_in_block: HMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>> =
            HMap::default();
        for cand in alloc_candidates.iter() {
            let ptr = cand.alloc_info.ptr;
            if let Some(needed_blocks) = phi_blocks.get(&ptr) {
                let needed_blocks: Vec<Ptr<BasicBlock>> = needed_blocks.iter().cloned().collect();
                for phi_block in needed_blocks {
                    let arg_idx = BasicBlock::push_argument(phi_block, ctx, cand.alloc_info.ty);
                    set_block_arg_name(ctx, phi_block, arg_idx, ptr.given_name(ctx));
                    new_phis_in_block
                        .entry(phi_block)
                        .or_default()
                        .push((cand.clone(), arg_idx));
                }
            }
        }

        // SSA rename via dominator tree walk, starting from the entry block.
        rename_blocks(ctx, dom_tree, &new_phis_in_block, &alloc_candidates)?;

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
