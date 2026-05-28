use crate::{
    attribute::{AttrObj, attr_cast},
    basic_block::BasicBlock,
    builtin::{attr_interfaces::MaterializableAttr, op_interfaces::BranchOpInterface},
    context::{Context, Ptr},
    graph::walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    irbuild::{
        IRStatus,
        inserter::Inserter,
        listener::Recorder,
        rewriter::{IRRewriter, Rewriter},
    },
    linked_list::ContainsLinkedList,
    op::{Op, op_cast},
    operation::Operation,
    result::Result,
    value::Value,
};
use pliron_derive::op_interface;
use rustc_hash::FxHashSet;

mod state;
use state::{BlockState, SccpState, ValState};

/// Interface for constant folding of operations.
#[op_interface]
pub trait ConstFoldInterface {
    /// Given a slice `operand_attrs` corresponding to each operand, indicating a known
    /// compile time constant value for that operand (if any), returns a vector corresponding
    /// to each result, indicating the folded (inferred constant) value, if any.
    fn check_fold(&self, ctx: &Context, operand_attrs: &[Option<AttrObj>]) -> Vec<Option<AttrObj>>;

    /// Given a slice `operand_attrs` corresponding to each operand, indicating a known
    /// compile time constant value for that operand (if any), attempts to fold the op in
    /// place using the provided `rewriter`. Assumes that `rewriter` is positioned just
    /// before the op to be folded.
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        operand_attrs: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Interface for ruling out branch destinations based on static information about branch conditions.
#[op_interface]
pub trait BranchOpFoldInterface: BranchOpInterface {
    /// Return the list of possible successor blocks given that `operands`
    /// contains `Some(attr)` for each operand known to be constant, where `attr` contains
    /// the known constant value.
    fn check_fold(&self, ctx: &Context, operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>>;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// For ops that do not implement a `RegionBranchOpInterface` analog,
/// propagate information from the op to its nested regions conservatively
/// by marking each of their entry blocks as Reachable and each of their
/// entry blocks' arguments as Unknown.
fn seed_nested_regions(op: Ptr<Operation>, ctx: &Context, state: &mut SccpState) {
    let regions: Vec<_> = op.deref(ctx).regions().collect();
    for region in regions {
        let Some(entry) = region.deref(ctx).get_head() else {
            continue;
        };
        state.merge_block_state(ctx, entry, BlockState::Reachable);
        let entry_args: Vec<Value> = entry.deref(ctx).arguments().collect();
        for arg in entry_args {
            state.merge_val_state(ctx, arg, ValState::Unknown);
        }
    }
}

/// Get the [ValState]s of `op`'s operands as a vector of optional attributes.
fn operand_attrs(op: Ptr<Operation>, ctx: &Context, state: &SccpState) -> Vec<Option<AttrObj>> {
    op.deref(ctx)
        .operands()
        .map(|v| match state.get_val_state(v) {
            ValState::Constant { val } => Some(val.clone()),
            ValState::Unknown => None,
            ValState::Undefinable => {
                panic!("SCCP algorithm won't process op until operands have been assigned")
            }
        })
        .collect()
}

/// Compute the [ValState]s of the results of `fold_op` given the current operand [ValState]s, and merge them
/// into the state.
fn process_fold_op(fold_op: &dyn ConstFoldInterface, ctx: &Context, state: &mut SccpState) {
    let op = fold_op.get_operation();
    let attrs = operand_attrs(op, ctx, state);
    let results: Vec<Value> = op.deref(ctx).results().collect();
    let folded_results = fold_op.check_fold(ctx, &attrs);
    for (res, attr) in results.iter().zip(folded_results) {
        let new_state = match attr {
            Some(val) => ValState::Constant { val },
            None => ValState::Unknown,
        };
        state.merge_val_state(ctx, *res, new_state);
    }
}

/// Compute which successor edges are traversable given the current [ValState]s of
/// `branch_op`'s operands, forward operand [ValState]s into the successor blocks' arguments, and mark
/// newly-reachable successor blocks for processing.
fn process_branch_op(branch_op: &dyn BranchOpFoldInterface, ctx: &Context, state: &mut SccpState) {
    let op = branch_op.get_operation();
    let attrs = operand_attrs(op, ctx, state);
    let live_successors: FxHashSet<Ptr<BasicBlock>> =
        branch_op.check_fold(ctx, &attrs).into_iter().collect();
    let static_successors: Vec<Ptr<BasicBlock>> = op.deref(ctx).successors().collect();
    for (succ_idx, succ_block) in static_successors.into_iter().enumerate() {
        if !live_successors.contains(&succ_block) {
            continue;
        }
        let forwarded = branch_op.successor_operands(ctx, succ_idx);
        let target_args: Vec<Value> = succ_block.deref(ctx).arguments().collect();
        for (fwd_val, target_arg) in forwarded.into_iter().zip(target_args) {
            let incoming = state.get_val_state(fwd_val);
            state.merge_val_state(ctx, target_arg, incoming);
        }
        state.merge_block_state(ctx, succ_block, BlockState::Reachable);
    }
}

/// Mark all `op`'s results as `Unknown`.
fn process_generic_op(op: Ptr<Operation>, ctx: &Context, state: &mut SccpState) {
    let results: Vec<Value> = op.deref(ctx).results().collect();
    for res in results {
        state.merge_val_state(ctx, res, ValState::Unknown);
    }
}

/// Attempt to inflate `state` and update worklists by using information from `op` and `state`.
fn process_op(op: Ptr<Operation>, ctx: &Context, state: &mut SccpState) {
    let op_dyn = Operation::get_op_dyn(op, ctx);
    let opt_branch = op_cast::<dyn BranchOpInterface>(op_dyn.as_ref());
    let opt_fold = op_cast::<dyn ConstFoldInterface>(op_dyn.as_ref());
    // TODO: add RegionBranchOpInterface and RegionBranchTerminatorOpInterface cases
    // once those interfaces exist in pliron.
    assert!(
        (opt_branch.is_some() as u8) + (opt_fold.is_some() as u8) <= 1,
        "SCCP requires BranchOpInterface, ConstFoldInterface (and future region-branch \
         interfaces) to be mutually exclusive on any given op"
    );
    seed_nested_regions(op, ctx, state);
    if opt_branch.is_some() {
        let opt_op_foldable = op_cast::<dyn BranchOpFoldInterface>(op_dyn.as_ref());
        match opt_op_foldable {
            Some(op_foldable) => {
                process_branch_op(op_foldable, ctx, state);
            }
            None => panic!(
                "the `constants` optimizer requires all branch ops to implement BranchOpFoldableInterface"
            ),
        }
    } else if let Some(op) = opt_fold {
        process_fold_op(op, ctx, state);
    } else {
        process_generic_op(op, ctx, state);
    }
}

/// Infer (into `state`) what we can from the operations of `block` given
/// the current information in `state`.
fn process_block(block: Ptr<BasicBlock>, ctx: &Context, state: &mut SccpState) {
    for op in block.deref(ctx).iter(ctx) {
        process_op(op, ctx, state);
    }
}

/// Infer (into `state`) what we can from the users of `val` given the
/// current information in `state`.
fn process_val(val: Value, ctx: &Context, state: &mut SccpState) {
    for user in val.uses(ctx).into_iter().map(|u| u.user_op()) {
        let user_block = user
            .deref(ctx)
            .get_parent_block()
            .expect("ops that use values have parents");
        if matches!(state.get_block_state(user_block), BlockState::Reachable) {
            process_op(user, ctx, state);
        }
    }
}

/// Perform sparse conditional constant propagation on `op` and its nested operations.
/// Assumes that `op` contains no free variables.
pub fn sccp(root_op: Ptr<Operation>, ctx: &mut Context) -> Result<IRStatus> {
    let mut state = SccpState::new(root_op, ctx);

    while !state.are_worklists_empty() {
        if let Some(block) = state.pop_block() {
            process_block(block, ctx, &mut state);
            continue;
        }
        if let Some(val) = state.pop_val() {
            process_val(val, ctx, &mut state);
            continue;
        }
    }

    let mut fold_candidates: Vec<(Ptr<Operation>, Vec<Option<AttrObj>>)> = Vec::new();
    let mut const_block_args: Vec<(Ptr<BasicBlock>, Value, AttrObj)> = Vec::new();
    walk_op(
        ctx,
        &mut (&state, &mut fold_candidates, &mut const_block_args),
        &WALKCONFIG_PREORDER_FORWARD,
        root_op,
        |ctx, (state, candidates, const_args), node| match node {
            IRNode::Operation(op) => {
                let Some(parent) = op.deref(ctx).get_parent_block() else {
                    return;
                };
                if !matches!(state.get_block_state(parent), BlockState::Reachable) {
                    return;
                }
                let op_dyn = Operation::get_op_dyn(op, ctx);
                if op_cast::<dyn ConstFoldInterface>(op_dyn.as_ref()).is_none() {
                    return;
                }
                candidates.push((op, operand_attrs(op, ctx, state)));
            }
            IRNode::BasicBlock(block) => {
                if !matches!(state.get_block_state(block), BlockState::Reachable) {
                    return;
                }
                for arg in block.deref(ctx).arguments() {
                    if let ValState::Constant { val } = state.get_val_state(arg) {
                        const_args.push((block, arg, val));
                    }
                }
            }
            IRNode::Region(_) => {}
        },
    );

    let mut rewriter = IRRewriter::<Recorder>::default();
    let mut status = IRStatus::Unchanged;
    for (op, attrs) in fold_candidates {
        rewriter.set_insertion_point_before_operation(op);
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let fold_interface = op_cast::<dyn ConstFoldInterface>(op_dyn.as_ref()).unwrap();
        status |= fold_interface.fold_in_place(ctx, &attrs, &mut rewriter);
    }

    for (block, arg, val) in const_block_args {
        let materialized_op = attr_cast::<dyn MaterializableAttr>(&*val)
            .expect(
                "SCCP requires constant block arguments' attributes to implement MaterializableAttr",
            )
            .materialize(ctx);
        rewriter.set_insertion_point_to_block_start(block);
        rewriter.insert_operation(ctx, materialized_op);
        let new_value = materialized_op.deref(ctx).get_result(0);
        rewriter.replace_value_uses_with(ctx, arg, new_value);
        status |= IRStatus::Changed;
    }

    Ok(status)
}
