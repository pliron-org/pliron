// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Sparse Conditional Constant Propagation (SCCP).
//!
//! This pass performs sparse conditional constant propagation over an operation
//! and its nested IR. The implementation roughly follows the presentations in
//! "Constant Propagation with Conditional Branches" by Wegman and Zadeck, and
//! Chapter 19 of Andrew Appel's "Modern Compiler Implementation in ML".
//!
//! SCCP relies on four key interfaces that operations must implement for it to be effective.
//! 1. [BranchOpFoldInterface]: To rule out branch destinations based on inferred constant
//!    values of its operands.
//! 2. [ConstFoldInterface]: To infer constant results from operand `Constness`.
//! 3. [MaterializableAttr]: To materialize inferred constant values (attributes) as operations.
//! 4. [RegionBranchOpFoldInterface]: To rule out control-flow edges of a
//!    [RegionBranchOpInterface] op based on inferred constant values of its operands
//!    and of the operands of its terminators.
//!
//! The analysis is sparse (SSA based) and worklist-driven:
//! 1. Reachable operations are analyzed. The analysis marks an operation as reachable
//!    along a branch edge or a region branch edge, or from the operation before it.
//! 2. [BranchOpInterface] ops can provide [BranchOpFoldInterface] to prune infeasible
//!    successors while forwarding operand `Constness` into successor block arguments.
//! 3. Constant-foldable operations provide [ConstFoldInterface] to infer constant
//!    results from operand `Constness`.
//! 4. [RegionBranchOpInterface] ops can provide [RegionBranchOpFoldInterface]
//!    to prune infeasible [RegionEdge](crate::builtin::op_interfaces::RegionEdge)s,
//!    while forwarding operand `Constness` into the entry block arguments of their
//!    regions and into their results.
//!
//! After convergence, this pass rewrites IR by:
//! 1. Folding reachable [ConstFoldInterface] operations in place.
//! 2. Materializing other reachable constant values when their inferred constant
//!    attributes implement [MaterializableAttr], and replacing uses accordingly.

use alloc::{
    format,
    string::{String, ToString},
};

use crate::{
    attribute::{AttrObj, attr_cast},
    basic_block::BasicBlock,
    builtin::{
        attr_interfaces::MaterializableAttr,
        op_interfaces::{
            BranchOpInterface, IsTerminatorInterface, RegionBranchOpInterface, RegionBranchPoint,
            RegionSuccessor,
        },
    },
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
    op::{op_cast, op_impls},
    operation::{OpDbg, Operation},
    opts::constants::{BranchOpFoldInterface, ConstFoldInterface, RegionBranchOpFoldInterface},
    pass::{AnalysisManager, Pass, PassResult},
    result::Result,
    utils::table::HSet,
    value::{DefiningEntity, Value},
};
use alloc::vec::Vec;

use super::state::{Constness, SccpState};

/// A wrapper over [SccpState::get_val_state], for the operands of an op being processed.
/// An [Undetermined](Constness::Undetermined) value:
///   - in a graph region, becomes [Constness::NotAConstant].
///   - in a region with SSA dominance, is asserted not to occur.
fn operand_state(val: Value, ctx: &Context, state: &SccpState) -> Constness {
    match state.get_val_state(val) {
        Constness::Undetermined => {
            assert!(
                !val.get_parent_region(ctx)
                    .unwrap()
                    .deref(ctx)
                    .has_ssa_dominance(ctx),
                "SCCP must process a definition before its uses in an SSA dominance region"
            );
            // In a graph region, a use can be processed before its definition.
            // Such a value is approximated as `NotAConstant`.
            Constness::NotAConstant
        }
        constness => constness,
    }
}

/// Get the [Constness] of `op`'s operands as a vector of optional attributes.
fn operand_attrs(op: Ptr<Operation>, ctx: &Context, state: &SccpState) -> Vec<Option<AttrObj>> {
    op.deref(ctx)
        .operands()
        .map(|v| match operand_state(v, ctx, state) {
            Constness::Constant { val } => Some(val),
            _ => None,
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

/// Given the [Constness] of the operands of `branch_op`:
///   - compute the traversable edges to its successor blocks
///   - forward operand [Constness] along these edges, into the successor blocks' arguments
///   - mark newly-reachable successor blocks for processing
fn process_branch_op(branch_op: &dyn BranchOpInterface, ctx: &Context, state: &mut SccpState) {
    let op = branch_op.get_operation();
    let op_dyn = Operation::get_op_dyn(op, ctx);
    let attrs = operand_attrs(op, ctx, state);
    let feasible_successors: HSet<Ptr<BasicBlock>> =
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
            let incoming = operand_state(fwd_val, ctx, state);
            state.merge_val_state(ctx, target_arg, incoming);
        }
        state.mark_block_reachable(ctx, succ_block);
    }
}

/// Given the [Constness] of the operands of `region_branch` (`op_attrs`)
/// and of the terminator at `point` (`terminator_attrs`):
///   - compute the traversable edges that start from `point`
///   - forward operand [Constness] along these edges
///   - mark the targets of these edges as reachable:
///     - the entry blocks of target regions
///     - the operation after `region_branch`
fn process_region_branch_edges(
    region_branch: &dyn RegionBranchOpInterface,
    point: RegionBranchPoint,
    op_attrs: &[Option<AttrObj>],
    terminator_attrs: &[Option<AttrObj>],
    ctx: &Context,
    state: &mut SccpState,
) {
    let op = region_branch.get_operation();
    let op_dyn = Operation::get_op_dyn(op, ctx);
    let edges = region_branch.edges_from(ctx, point);
    let feasible_successors: Vec<RegionSuccessor> =
        match op_cast::<dyn RegionBranchOpFoldInterface>(op_dyn.as_ref()) {
            Some(fold) => fold.check_fold(ctx, point, op_attrs, terminator_attrs),
            None => {
                log::info!(
                    "Region branch operation '{}' does not implement RegionBranchOpFoldInterface,
                        weakening sccp optimization",
                    region_branch.disp(ctx)
                );
                edges.iter().map(|edge| edge.to).collect()
            }
        };
    for edge in edges
        .into_iter()
        .filter(|edge| feasible_successors.contains(&edge.to))
    {
        for (operand, receiver) in edge.flows {
            let incoming = operand_state(operand.get_def(ctx), ctx, state);
            state.merge_val_state(ctx, receiver, incoming);
        }
        match edge.to {
            RegionSuccessor::Region(region) => {
                let entry = region.deref(ctx).get_entry_block();
                if let Some(entry) = entry {
                    state.mark_block_reachable(ctx, entry);
                }
            }
            RegionSuccessor::After => state.mark_next_reachable(ctx, op),
        }
    }
}

/// Process the edges that start at `region_branch` itself.
fn process_region_branch_op(
    region_branch: &dyn RegionBranchOpInterface,
    ctx: &Context,
    state: &mut SccpState,
) {
    let op = region_branch.get_operation();
    let attrs = operand_attrs(op, ctx, state);
    let edges = region_branch.region_edges(ctx);
    // Results and entry block arguments that do not receive a value on any edge
    // can have any value.
    let receivers: HSet<Value> = edges
        .iter()
        .flat_map(|edge| edge.flows.iter().map(|(_, receiver)| *receiver))
        .collect();
    let results: Vec<Value> = op.deref(ctx).results().collect();
    let entry_args: Vec<Value> = op
        .deref(ctx)
        .regions()
        .filter_map(|region| region.deref(ctx).get_entry_block())
        .flat_map(|entry| entry.deref(ctx).arguments().collect::<Vec<_>>())
        .collect();
    for value in results.into_iter().chain(entry_args) {
        if !receivers.contains(&value) {
            state.merge_val_state(ctx, value, Constness::NotAConstant);
        }
    }

    process_region_branch_edges(
        region_branch,
        RegionBranchPoint::Parent,
        &attrs,
        &[],
        ctx,
        state,
    );

    // Process the edges that start at reachable terminators again,
    // because their feasibility can depend on the operands of `region_branch`.
    let mut terminators: Vec<Ptr<Operation>> = Vec::new();
    for edge in &edges {
        if let RegionBranchPoint::Terminator(terminator) = edge.from
            && !terminators.contains(&terminator)
        {
            terminators.push(terminator);
        }
    }
    for terminator in terminators {
        if state.is_reachable(terminator) {
            process_region_terminator(terminator, op, ctx, state);
        }
    }
}

/// If `op` starts a region edge of its parent op, return that parent op.
fn region_branch_parent(op: Ptr<Operation>, ctx: &Context) -> Option<Ptr<Operation>> {
    let op_dyn = Operation::get_op_dyn(op, ctx);
    // Quick check for early exit.
    if !op_impls::<dyn IsTerminatorInterface>(op_dyn.as_ref()) {
        return None;
    }
    let parent = op.deref(ctx).get_parent_op(ctx)?;
    let parent_dyn = Operation::get_op_dyn(parent, ctx);
    let region_branch = op_cast::<dyn RegionBranchOpInterface>(parent_dyn.as_ref())?;
    let starts_edges = !region_branch
        .edges_from(ctx, RegionBranchPoint::Terminator(op))
        .is_empty();
    starts_edges.then_some(parent)
}

/// Process the edges that start at `terminator`, an operation in a region of `parent`.
fn process_region_terminator(
    terminator: Ptr<Operation>,
    parent: Ptr<Operation>,
    ctx: &Context,
    state: &mut SccpState,
) {
    let parent_attrs = operand_attrs(parent, ctx, state);
    let attrs = operand_attrs(terminator, ctx, state);
    let parent_dyn = Operation::get_op_dyn(parent, ctx);
    let region_branch = op_cast::<dyn RegionBranchOpInterface>(parent_dyn.as_ref())
        .expect("parent must implement RegionBranchOpInterface");
    process_region_branch_edges(
        region_branch,
        RegionBranchPoint::Terminator(terminator),
        &parent_attrs,
        &attrs,
        ctx,
        state,
    );
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
    let opt_region_branch = op_cast::<dyn RegionBranchOpInterface>(op_dyn.as_ref());
    let opt_region_branch_parent = region_branch_parent(op, ctx);
    assert!(
        (opt_branch.is_some() as u8)
            + (opt_fold.is_some() as u8)
            + (opt_region_branch.is_some() as u8)
            + (opt_region_branch_parent.is_some() as u8)
            <= 1,
        "SCCP requires BranchOpInterface, ConstFoldInterface, RegionBranchOpInterface \
         and terminators of RegionBranchOpInterface ops to be mutually exclusive on any given op"
    );
    if let Some(op) = opt_region_branch {
        process_region_branch_op(op, ctx, state);
        return;
    }
    state.seed_nested_regions(op, ctx);
    if let Some(parent) = opt_region_branch_parent {
        process_region_terminator(op, parent, ctx, state);
    } else if let Some(op) = opt_branch {
        process_branch_op(op, ctx, state);
    } else if let Some(op) = opt_fold {
        process_fold_op(op, ctx, state);
    } else {
        process_generic_op(op, ctx, state);
    }
}

/// Does control continue from `op` to the next operation in its block?
fn falls_through(op: Ptr<Operation>, ctx: &Context) -> bool {
    let op_dyn = Operation::get_op_dyn(op, ctx);
    // A region branch op's execution need not continue to the next op.
    // Its region branch edges decide what executes next.
    if !op_impls::<dyn RegionBranchOpInterface>(op_dyn.as_ref()) {
        return true;
    }
    // A graph region has no control flow between its ops,
    // so we consider all of its ops reachable.
    op.deref(ctx)
        .get_parent_region(ctx)
        .is_some_and(|region| !region.deref(ctx).has_ssa_dominance(ctx))
}

/// Perform sparse conditional constant propagation on `op` and its nested operations.
pub fn sccp(root_op: Ptr<Operation>, ctx: &mut Context) -> Result<IRStatus> {
    let mut state = SccpState::new(root_op, ctx);

    // 1. The analysis
    while let Some(op) = state.pop_op() {
        process_op(op, ctx, &mut state);
        if falls_through(op, ctx) {
            state.mark_next_reachable(ctx, op);
        }
    }

    // 2. Collect optimization candidates
    let mut fold_candidates: Vec<(Ptr<Operation>, Vec<Option<AttrObj>>)> = Vec::new();
    let mut const_values: Vec<(Value, AttrObj)> = Vec::new();
    walk_op(
        ctx,
        &mut (&state, &mut fold_candidates, &mut const_values),
        &WALKCONFIG_PREORDER_FORWARD,
        root_op,
        |ctx, (state, candidates, const_values), node| {
            let values: Vec<Value> = match node {
                IRNode::Operation(op) => {
                    if !state.is_reachable(op) {
                        return;
                    }
                    let op_dyn = Operation::get_op_dyn(op, ctx);
                    if op_impls::<dyn ConstFoldInterface>(op_dyn.as_ref()) {
                        // The op folds itself, together with its results.
                        candidates.push((op, operand_attrs(op, ctx, state)));
                        return;
                    }
                    op.deref(ctx).results().collect()
                }
                IRNode::BasicBlock(block) => {
                    if !state.is_block_reachable(ctx, block) {
                        return;
                    }
                    block.deref(ctx).arguments().collect()
                }
                IRNode::Region(_) => return,
            };
            for value in values {
                if let Constness::Constant { val } = state.get_val_state(value) {
                    const_values.push((value, val));
                }
            }
        },
    );

    // 3. Optimize / rewrite
    let mut rewriter = IRRewriter::<Recorder>::default();
    let mut status = IRStatus::Unchanged;

    // 3(a). Fold operations that implement `ConstFoldInterface`
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

    // 3(b). Materialize other constant values
    for (value, val) in const_values {
        let Some(materializable) = attr_cast::<dyn MaterializableAttr>(&*val) else {
            log::info!(
                "Attribute '{}' does not implement the MaterializableAttr interface, preventing optimization",
                val.disp(ctx)
            );
            continue;
        };
        log::debug!(
            "Materializing value '{}' with inferred constant value '{}'",
            value.unique_name(ctx),
            val.disp(ctx)
        );
        let materialized_op = materializable.materialize(ctx);
        match value.defining_entity() {
            DefiningEntity::Block(block) => rewriter.set_insertion_point_to_block_start(block),
            DefiningEntity::Op(op) => rewriter.set_insertion_point_before_operation(op),
        }
        rewriter.insert_operation(ctx, materialized_op);
        let new_value = materialized_op.deref(ctx).get_result(0);
        rewriter.replace_value_uses_with(ctx, value, new_value);
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
