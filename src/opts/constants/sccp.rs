//! Sparse Conditional Constant Propagation (SCCP).
//!
//! This pass performs sparse conditional constant propagation over an operation
//! and its nested IR. The implementation roughly follows the presentations in
//! "Constant Propagation with Conditional Branches" by Wegman and Zadeck, and
//! Chapter 19 of Andrew Appel's "Modern Compiler Implementation in ML".
//!
//! SCCP relies on three key interfaces that operations must implement for it to be effective.
//! 1. [BranchOpFoldInterface]: To rule out branch destinations based on inferred constant
//!    values of its operands.
//! 2. [ConstFoldInterface]: To infer constant results from operand constness.
//! 3. [MaterializableAttr]: To materialize operations from constant block arguments
//!    (whose inferred constant values (attributes) must implement this interface).
//!
//! The analysis is sparse (SSA based) and worklist-driven:
//! 1. Reachable blocks are visited and their operations are analyzed.
//! 2. Branch operations can provide [BranchOpFoldInterface] to prune infeasible
//!    successors while forwarding operand facts to successor block arguments.
//! 3. Constant-foldable operations provide [ConstFoldInterface] to infer constant
//!    results from operand constness.
//!
//! After convergence, this pass rewrites IR by:
//! 1. Folding operations in reachable blocks in place.
//! 2. Materializing constant block arguments when the inferred constant attributes
//!    implement [MaterializableAttr], and replacing uses accordingly.

use alloc::{
    format,
    string::{String, ToString},
};

use crate::{
    attribute::{AttrObj, attr_cast},
    basic_block::BasicBlock,
    builtin::{attr_interfaces::MaterializableAttr, op_interfaces::BranchOpInterface},
    common_traits::Named,
    context::{Context, Ptr},
    graph::{
        dominance::DomInfo,
        walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    },
    irbuild::{
        IRStatus,
        inserter::Inserter,
        listener::Recorder,
        rewriter::{IRRewriter, Rewriter},
    },
    linked_list::ContainsLinkedList,
    op::op_cast,
    operation::{OpDbg, Operation},
    opts::constants::{BranchOpFoldInterface, ConstFoldInterface},
    pass::{AnalysisManager, Pass, PassResult},
    result::Result,
    value::Value,
};
use alloc::vec::Vec;
use rustc_hash::FxHashSet;

use super::state::{BlockState, Constness, SccpState};

/// Get the [Constness] of `op`'s operands as a vector of optional attributes.
fn operand_attrs(op: Ptr<Operation>, ctx: &Context, state: &SccpState) -> Vec<Option<AttrObj>> {
    op.deref(ctx)
        .operands()
        .map(|v| match state.get_val_state(v) {
            Constness::Constant { val } => Some(val.clone()),
            Constness::NotAConstant => None,
            Constness::Undetermined => {
                // This means `v` was defined in a scope outside of the root operation
                // we're applying sccp to. These variables aren't really Undetermined;
                // we just never added them to the state.
                None
            }
        })
        .collect()
}

/// Compute the [Constness] of the results of `fold_op` given the current operands'
/// [Constness], and merge them into the state.
fn process_fold_op(fold_op: &dyn ConstFoldInterface, ctx: &Context, state: &mut SccpState) {
    let op = fold_op.get_operation();
    let attrs = operand_attrs(op, ctx, state);
    let results: Vec<Value> = op.deref(ctx).results().collect();
    let folded_results = fold_op.check_fold(ctx, &attrs);
    for (res, attr) in results.iter().zip(folded_results) {
        let new_state = match attr {
            Some(val) => Constness::Constant { val },
            None => Constness::NotAConstant,
        };
        state.merge_val_state(ctx, *res, new_state);
    }
}

/// Compute which successor edges are traversable given the current [Constness] of
/// `branch_op`'s operands, forward operand [Constness] into the successor blocks'
/// arguments, and mark newly-reachable successor blocks for processing.
fn process_branch_op(branch_op: &dyn BranchOpInterface, ctx: &Context, state: &mut SccpState) {
    let op = branch_op.get_operation();
    let op_dyn = Operation::get_op_dyn(op, ctx);
    let attrs = operand_attrs(op, ctx, state);
    let feasible_successors: FxHashSet<Ptr<BasicBlock>> =
        match op_cast::<dyn BranchOpFoldInterface>(op_dyn.as_ref()) {
            Some(branch_op_fold) => branch_op_fold.check_fold(ctx, &attrs).into_iter().collect(),
            None => {
                log::info!(
                    "Branch operation '{}' does not implement BranchOpFoldInterface,
                        weakening sccp optimization",
                    branch_op.disp(ctx)
                );
                op.deref(ctx).successors().collect()
            }
        };
    let static_successors: Vec<Ptr<BasicBlock>> = op.deref(ctx).successors().collect();
    for (succ_idx, succ_block) in static_successors.into_iter().enumerate() {
        if !feasible_successors.contains(&succ_block) {
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

/// Mark all `op`'s results as [Constness::NotAConstant].
fn process_generic_op(op: Ptr<Operation>, ctx: &Context, state: &mut SccpState) {
    let results: Vec<Value> = op.deref(ctx).results().collect();
    for res in results {
        state.merge_val_state(ctx, res, Constness::NotAConstant);
    }
}

/// Attempt to deflate `state` and update worklists by using information from `op` and `state`.
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
    state.seed_nested_regions(op, ctx);
    if let Some(op) = opt_branch {
        process_branch_op(op, ctx, state);
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
                    if let Constness::Constant { val } = state.get_val_state(arg) {
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
        let log_message = if log::log_enabled!(log::Level::Debug) {
            // Some implementations of `ConstFoldInterface` (such as with `ConstantOp`)
            // will not actually fold the operation as they're already folded.
            // So we log the message only if there was an actual folding.
            let op_dbg = OpDbg { op, ctx };
            let attr_strs: Vec<String> = attrs
                .iter()
                .map(|a| {
                    a.as_ref().map_or("undetermined".to_string(), |attr| {
                        attr.disp(ctx).to_string()
                    })
                })
                .collect();
            format!(
                "Folding operation '{}' with inferred operand attributes {}",
                op_dbg,
                attr_strs.join(", ")
            )
        } else {
            String::new()
        };
        rewriter.set_insertion_point_before_operation(op);
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let fold_interface = op_cast::<dyn ConstFoldInterface>(op_dyn.as_ref()).unwrap();
        let fold_result = fold_interface.fold_in_place(ctx, &attrs, &mut rewriter);
        if fold_result == IRStatus::Changed {
            log::debug!("{}", log_message);
        }
        status |= fold_result;
    }

    for (block, arg, val) in const_block_args {
        let Some(materializable) = attr_cast::<dyn MaterializableAttr>(&*val) else {
            log::info!(
                "Attribute '{}' does not implement the MaterializableAttr interface, preventing optimization",
                val.disp(ctx)
            );
            continue;
        };
        log::debug!(
            "Materializing block argument '{}' with inferred constant value '{}'",
            arg.unique_name(ctx),
            val.disp(ctx)
        );
        let materialized_op = materializable.materialize(ctx);
        rewriter.set_insertion_point_to_block_start(block);
        rewriter.insert_operation(ctx, materialized_op);
        let new_value = materialized_op.deref(ctx).get_result(0);
        rewriter.replace_value_uses_with(ctx, arg, new_value);
        status |= IRStatus::Changed;
    }

    Ok(status)
}

#[derive(Default)]
/// A [Pass] that performs sparse conditional constant propagation
/// as described in the module-level documentation.
pub struct SCCPPass;

impl Pass for SCCPPass {
    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_res = PassResult::default();
        // Run SCCP on the entire operation tree rooted at `op`
        pass_res.ir_changed |= sccp(op, ctx)?;
        // SCCP does not touch the CFG structure, so we can preserve dominator info if it exists.
        pass_res.set_preserved::<DomInfo>();
        Ok(pass_res)
    }

    fn name(&self) -> &str {
        "sccp"
    }
}
