//! Memory to register promotion optimization pass.

use core::panic;

use alloc::{rc::Rc, vec::Vec};
use pliron_derive::op_interface;

use crate::{
    basic_block::BasicBlock,
    builtin::op_interfaces::BranchOpInterface,
    common_traits::Named,
    context::{Context, Ptr},
    debug_info::set_block_arg_name,
    graph::{
        dominance::{DomFrontierMap, DomInfo, DomTree},
        walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
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
    std_deps::hash::{FxHashMap, FxHashSet, hash_map::Entry},
    r#type::TypeHandle,
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
    /// The inserter is positioned to be just before the alloc op.
    fn default_value(
        &self,
        ctx: &mut Context,
        inserter: &mut dyn Inserter,
        alloc_info: &AllocInfo,
    ) -> Result<Value>;

    /// Promote allocations. This is called after all uses have been promoted,
    /// and should remove the allocation (not necessary the operation itself) from the IR.
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

/// Compute per-candidate liveness anchors for mem2reg analysis.
///
/// Returns:
/// - `live_in`: blocks where the candidate's value is live-in.
/// - `defining_blocks`: blocks that contain a store defining the candidate.
fn compute_candidate_live_in_and_defining_blocks(
    ctx: &Context,
    cand: &AllocCandidate,
) -> (FxHashSet<Ptr<BasicBlock>>, FxHashSet<Ptr<BasicBlock>>) {
    let ptr = cand.alloc_info.ptr;

    let mut defining_blocks: FxHashSet<Ptr<BasicBlock>> = FxHashSet::default();
    let mut live_in: FxHashSet<Ptr<BasicBlock>> = FxHashSet::default();
    let mut live_in_worklist: Vec<Ptr<BasicBlock>> = Vec::new();

    // Compute blocks that contain uses of this pointer.
    let mut user_blocks: FxHashSet<Ptr<BasicBlock>> = FxHashSet::default();
    for u in ptr.uses(ctx) {
        if let Some(block) = u.user_op().deref(ctx).get_parent_block() {
            user_blocks.insert(block);
        }
    }

    // A block is a defining block if it has any store to this candidate.
    // A block seeds liveness if it has a load/eliminatable use before the first store.
    for block in user_blocks {
        let mut has_store = false;
        let mut load_before_store = false;
        for op in block.deref(ctx).iter(ctx) {
            let op_obj = Operation::get_op_dyn(op, ctx);
            let Some(op_promotable) = op_cast::<dyn PromotableOpInterface>(op_obj.as_ref()) else {
                continue;
            };
            match op_promotable.promotion_kind(ctx, &cand.alloc_info) {
                PromotableOpKind::Load | PromotableOpKind::EliminatableUse => {
                    if !has_store {
                        load_before_store = true;
                    }
                }
                PromotableOpKind::Store(_) => {
                    has_store = true;
                }
                PromotableOpKind::NonPromotableUse => {
                    // Filtered by prune_candidates.
                }
            }
        }

        if has_store {
            defining_blocks.insert(block);
        }
        if load_before_store {
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

/// Compute phi placement blocks for one candidate using liveness-pruned IDF.
///
/// Starting from the candidate's defining blocks, this computes the iterated
/// dominance frontier and keeps only blocks where the candidate is live-in.
fn compute_candidate_phi_blocks(
    df_map: &DomFrontierMap<Ptr<Region>, Context>,
    live_in: &FxHashSet<Ptr<BasicBlock>>,
    defining_blocks: &FxHashSet<Ptr<BasicBlock>>,
) -> FxHashSet<Ptr<BasicBlock>> {
    // Compute liveness-pruned IDF for this candidate.
    let mut phi_blocks: FxHashSet<Ptr<BasicBlock>> = FxHashSet::default();
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
    phi_blocks: &mut FxHashMap<Value, FxHashSet<Ptr<BasicBlock>>>,
) {
    // Track invalid individual candidates by their allocation pointer.
    let mut invalid_ptrs: FxHashSet<Value> = FxHashSet::default();
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
    default_defs: &mut FxHashMap<Value, Value>,
) -> Result<Value> {
    match default_defs.entry(alloc_cand.alloc_info.ptr) {
        Entry::Occupied(entry) => Ok(*entry.get()),
        Entry::Vacant(entry) => {
            let alloc_op = alloc_cand.alloc_op;
            let alloc_obj = Operation::get_op_dyn(alloc_op, ctx);
            let alloc_iface = op_cast::<dyn PromotableAllocationInterface>(alloc_obj.as_ref())
                .expect("Alloc op must implement PromotableAllocationInterface");
            let default_val = alloc_iface.default_value(
                ctx,
                &mut IRInserter::<Recorder>::new_before_operation(alloc_op),
                &alloc_cand.alloc_info,
            )?;
            entry.insert(default_val);
            Ok(default_val)
        }
    }
}

/// Process the events in the recorder to note down erased operations.
fn note_erased_ops(recorder: &mut Recorder, erased: &mut FxHashSet<Ptr<Operation>>) {
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

/// For each promotable allocation pointer, stores the current reaching definition.
type ReachingDefMap = FxHashMap<Value, Option<Value>>;

/// Process one block during SSA rename for mem2reg.
///
/// Builds per-candidate reaching-definition stacks from the incoming map,
/// applies promotion rewrites in the block, fills successor phi operands,
/// and returns the updated outgoing reaching definition for each candidate.
fn process_rename_block(
    ctx: &mut Context,
    block: Ptr<BasicBlock>,
    new_phis_in_block: &FxHashMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>>,
    incoming_reaching_def_map: &ReachingDefMap,
    default_def_map: &mut FxHashMap<Value, Value>,
    alloc_candidates: &[AllocCandidate],
) -> Result<ReachingDefMap> {
    let mut reaching_def_map = incoming_reaching_def_map
        .iter()
        .map(|(&ptr, maybe_def)| {
            (ptr, {
                let mut stack = Vec::new();
                if let Some(def) = *maybe_def {
                    stack.push(def);
                }
                stack
            })
        })
        .collect::<FxHashMap<_, _>>();

    // Push phi args for this block.
    for &(ref cand, arg_idx) in new_phis_in_block.get(&block).into_iter().flatten() {
        let new_val = block.deref(ctx).get_argument(arg_idx);
        reaching_def_map
            .get_mut(&cand.alloc_info.ptr)
            .unwrap()
            .push(new_val);
    }

    let ops: Vec<Ptr<Operation>> = block.deref(ctx).iter(ctx).collect();
    let mut erased_ops = FxHashSet::default();
    for &op in &ops {
        if erased_ops.contains(&op) {
            continue;
        }
        let op_obj = Operation::get_op_dyn(op, ctx);
        let Some(piface) = op_cast::<dyn PromotableOpInterface>(op_obj.as_ref()) else {
            continue;
        };

        let mut promote_queue = Vec::new();
        for cand in alloc_candidates {
            let ptr = cand.alloc_info.ptr;
            match piface.promotion_kind(ctx, &cand.alloc_info) {
                PromotableOpKind::Load | PromotableOpKind::EliminatableUse => {
                    let reaching_def_stack = reaching_def_map.get_mut(&ptr).unwrap();
                    if reaching_def_stack.is_empty() {
                        // No reaching definition: use default value
                        let default_val = get_or_create_default_def(cand, ctx, default_def_map)?;
                        reaching_def_stack.push(default_val);
                    }
                    let current_def = *reaching_def_stack.last().unwrap();
                    promote_queue.push((cand.alloc_info.clone(), current_def));
                }
                PromotableOpKind::Store(stored_val) => {
                    reaching_def_map.get_mut(&ptr).unwrap().push(stored_val);
                    promote_queue.push((cand.alloc_info.clone(), stored_val));
                }
                // Intentionally no-op: this includes the common case where `op`
                // does not use `cand.alloc_info.ptr` (required by the interface contract).
                PromotableOpKind::NonPromotableUse => {}
            }
        }
        if !promote_queue.is_empty() {
            let rewriter = &mut IRRewriter::default();
            rewriter.set_insertion_point_before_operation(op);
            log::trace!("Promoting op {}", OpDbg { op, ctx });
            piface.promote(ctx, &promote_queue, rewriter)?;
            note_erased_ops(rewriter.get_listener_mut(), &mut erased_ops);
        }
    }

    // Fill phi operands in successor branch ops.
    let succs = block.deref(ctx).succs(ctx);
    for (succ_idx, new_phis_in_succ) in succs.iter().enumerate().filter_map(|(succ_idx, succ)| {
        new_phis_in_block
            .get(succ)
            .map(|new_phis| (succ_idx, new_phis))
    }) {
        let term = block
            .deref(ctx)
            .get_terminator(ctx)
            .expect("Block has successors but no terminator");
        let term_obj = Operation::get_op_dyn(term, ctx);
        let branch_iface = op_cast::<dyn BranchOpInterface>(term_obj.as_ref())
            .expect("Terminator must implement BranchOpInterface for phi blocks");
        for &(ref cand, arg_idx) in new_phis_in_succ {
            let reaching_def_stack = reaching_def_map.get_mut(&cand.alloc_info.ptr).unwrap();
            if reaching_def_stack.is_empty() {
                // No reaching definition: use default value
                let default_val = get_or_create_default_def(cand, ctx, default_def_map)?;
                reaching_def_stack.push(default_val);
            }
            let current_def = *reaching_def_stack.last().unwrap();
            let succ_opd_idx = branch_iface.add_successor_operand(ctx, succ_idx, current_def);
            // The operand index returned by add_successor_operand should match the phi argument index.
            assert!(succ_opd_idx == arg_idx, "Mismatched phi argument index");
        }
    }

    let outgoing_reaching_def_map = reaching_def_map
        .into_iter()
        .map(|(ptr, stack)| (ptr, stack.last().copied()))
        .collect();

    Ok(outgoing_reaching_def_map)
}

/// Rename uses of allocation pointers to SSA values across the dominator tree,
/// replacing loads/stores with reaching definitions and filling successor phi
/// operands for each processed block.
fn rename_blocks(
    ctx: &mut Context,
    entry_block: Ptr<BasicBlock>,
    dom_tree: &DomTree<Ptr<Region>, Context>,
    new_phis_in_block: &FxHashMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>>,
    reaching_def_map: &ReachingDefMap,
    default_def_map: &mut FxHashMap<Value, Value>,
    alloc_candidates: &[AllocCandidate],
) -> Result<()> {
    type RenameWorkItem = (Ptr<BasicBlock>, Rc<ReachingDefMap>);
    let mut worklist: Vec<RenameWorkItem> = Vec::new();
    worklist.push((entry_block, Rc::new(reaching_def_map.clone())));

    while let Some((block, incoming_reaching_def_map)) = worklist.pop() {
        let outgoing_reaching_def_map = process_rename_block(
            ctx,
            block,
            new_phis_in_block,
            incoming_reaching_def_map.as_ref(),
            default_def_map,
            alloc_candidates,
        )?;
        let outgoing_reaching_def_map = Rc::new(outgoing_reaching_def_map);

        let mut children: Vec<Ptr<BasicBlock>> = dom_tree.children(&block).collect();
        // Reverse so pop() preserves the original child iteration order.
        children.reverse();
        for child in children {
            worklist.push((child, Rc::clone(&outgoing_reaching_def_map)));
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

    if candidates.is_empty() {
        return Ok(IRStatus::Unchanged);
    }

    // Categorize by region for efficiency.
    // We process all candidates in a region together to amortize
    // the cost of computing dominator trees, frontiers, etc.
    let mut by_region: FxHashMap<Ptr<Region>, Vec<AllocCandidate>> = FxHashMap::default();
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
        let mut phi_blocks: FxHashMap<Value, FxHashSet<Ptr<BasicBlock>>> = FxHashMap::default();
        for cand in alloc_candidates.iter() {
            let ptr = cand.alloc_info.ptr;
            let (live_in, defining_blocks) =
                compute_candidate_live_in_and_defining_blocks(ctx, cand);
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
        let mut new_phis_in_block: FxHashMap<Ptr<BasicBlock>, Vec<(AllocCandidate, usize)>> =
            FxHashMap::default();
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

        // Initialize reaching def map for this region's candidates.
        let reaching_def_map: ReachingDefMap = alloc_candidates
            .iter()
            .map(|c| (c.alloc_info.ptr, None))
            .collect();
        let mut default_def_map: FxHashMap<Value, Value> = FxHashMap::default();

        // SSA rename via dominator tree walk, starting from the entry block.
        let entry_block = region
            .deref(ctx)
            .get_head()
            .expect("No entry block in region");
        rename_blocks(
            ctx,
            entry_block,
            dom_tree,
            &new_phis_in_block,
            &reaching_def_map,
            &mut default_def_map,
            &alloc_candidates,
        )?;

        // "Promote" (remove) the allocations themselves. Group them into
        // a single promote call per alloc op and then invoke the interface method.
        let mut alloc_op_to_infos: FxHashMap<Ptr<Operation>, Vec<AllocInfo>> = FxHashMap::default();
        let rewriter = &mut IRRewriter::default();

        for cand in alloc_candidates.iter() {
            alloc_op_to_infos
                .entry(cand.alloc_op)
                .or_default()
                .push(cand.alloc_info.clone());
        }

        let mut erased_ops = FxHashSet::default();
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
