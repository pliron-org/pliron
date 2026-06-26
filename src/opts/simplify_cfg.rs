//! Control flow graph (CFG) simplification.
//!
//! This optimization performs three tasks in sequence:
//!
//! 1. It rewrites any conditional branch operation that implements [BranchOpFoldInterface] and
//!    whose condition operand is defined as a constant to an unconditional branch.
//!
//! 2. It detects unreachable blocks by performing a DFS on every nested SSA region, removing
//!    all unreachable blocks it detects.
//!
//! 3. It merges every pair of blocks `A` and `B`, where `B` is the sole successor of `A` and
//!    `A` is the sole predecessor of `B`, removing `A`'s terminator and forwarding the actual
//!    branch arguments of `A` into the formal arguments of `B`.

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    builtin::op_interfaces::BranchOpInterface,
    context::{Context, Ptr},
    deps::hash::FxHashSet,
    graph::{
        HasLabel,
        walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    },
    irbuild::{
        IRStatus,
        inserter::{Inserter, OpInsertionPoint},
        listener::Recorder,
        rewriter::{IRRewriter, Rewriter},
    },
    linked_list::{ContainsLinkedList, LinkedList},
    op::{op_cast, op_impls},
    operation::{OpDbg, Operation},
    opts::constants::{BranchOpFoldInterface, ConstFoldInterface},
    pass::{AnalysisManager, Pass, PassResult},
    region::Region,
    result::Result,
    value::Value,
};

/// For each operand of `op`, return the constant value it carries if the operand
/// is defined by a [ConstantOp], or `None` otherwise.
fn constant_operand_attrs(op: Ptr<Operation>, ctx: &Context) -> Vec<Option<AttrObj>> {
    // Assumes `v` is defined by `def`.
    // Returns `Some(attr_obj)` if `def` implements `ConstFoldInterface` and the
    // result corresponding to `v` is determined to be the constant `attr_obj`.
    let get_def_const = |def: Ptr<Operation>, v: Value| {
        let def_dyn = Operation::get_op_dyn(def, ctx);
        match op_cast::<dyn ConstFoldInterface>(def_dyn.as_ref()) {
            Some(fold_interface) => {
                let def_ops_nonconst = vec![None; def.deref(ctx).get_num_operands()];
                let results = fold_interface.check_fold(ctx, def_ops_nonconst.as_slice());
                let ind = v.find_index(ctx);
                results[ind].clone()
            }
            None => None,
        }
    };
    op.deref(ctx)
        .operands()
        .map(|v| v.defining_op().and_then(|def| get_def_const(def, v)))
        .collect()
}

/// Merge `succ` into `pred` when
/// * `pred` has a single successor `succ`, and
/// * `succ`'s only predecessor is `pred`, and
/// * `succ` is not the region `entry`
///
/// Returns `true` on success.
fn try_merge_succ(
    pred: Ptr<BasicBlock>,
    entry: Ptr<BasicBlock>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> bool {
    let succs = pred.deref(ctx).succs(ctx);
    let [succ] = succs[..] else {
        return false;
    };
    if succ == entry {
        return false;
    }
    if succ.num_preds(ctx) != 1 {
        return false;
    }

    let pred_terminator = pred
        .deref(ctx)
        .get_terminator(ctx)
        .expect("all blocks must have terminators");
    let actual_args: Vec<Value> = {
        let terminator_dyn = Operation::get_op_dyn(pred_terminator, ctx);
        let Some(branch) = op_cast::<dyn BranchOpInterface>(terminator_dyn.as_ref()) else {
            log::info!(
                "Terminator operation '{}' does not implement BranchOpFoldInterface,
                    weakening simplify-cfg optimization",
                terminator_dyn.disp(ctx)
            );
            return false;
        };
        branch.successor_operands(ctx, 0)
    };

    log::debug!(
        "Merging block {} into its successor {}",
        pred.label(ctx),
        succ.label(ctx)
    );

    let formal_args: Vec<Value> = succ.deref(ctx).arguments().collect();
    assert_eq!(
        formal_args.len(),
        actual_args.len(),
        "branch must forward one operand per successor block argument"
    );
    for (formal, actual) in formal_args.iter().zip(actual_args.iter()) {
        rewriter.replace_value_uses_with(ctx, *formal, *actual);
    }

    rewriter.erase_operation(ctx, pred_terminator);
    let mut cur = succ.deref(ctx).get_head();
    while let Some(op) = cur {
        let next = op.deref(ctx).get_next();
        rewriter.move_operation(ctx, op, OpInsertionPoint::AtBlockEnd(pred));
        cur = next;
    }

    rewriter.erase_block(ctx, succ);
    true
}

/// Remove unreachable blocks nested inside `op`.
/// Returns whether the IR was changed.
pub fn remove_blocks_inside_op(
    op: Ptr<Operation>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> IRStatus {
    let regions: Vec<Ptr<Region>> = op.deref(ctx).regions().collect();
    let mut status = IRStatus::Unchanged;
    //TODO: RegionBranchOpInterface should allow us to handle this less conservatively
    for region in regions {
        status |= remove_blocks_inside_region(region, ctx, rewriter);
    }
    status
}

/// Remove unreachable blocks nested inside `block`.
/// Returns whether the IR was changed.
pub fn remove_blocks_inside_block(
    block: Ptr<BasicBlock>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> IRStatus {
    let ops: Vec<Ptr<Operation>> = block.deref(ctx).iter(ctx).collect();
    let mut status = IRStatus::Unchanged;
    for op in ops {
        status |= remove_blocks_inside_op(op, ctx, rewriter);
    }
    status
}

/// Remove unreachable blocks nested inside `region`.
/// Returns whether the IR was changed.
pub fn remove_blocks_inside_region(
    region: Ptr<Region>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> IRStatus {
    if !region.deref(ctx).has_ssa_dominance(ctx) {
        let head = region
            .deref(ctx)
            .get_head()
            .expect("all regions should have entry block");
        return remove_blocks_inside_block(head, ctx, rewriter);
    }

    let Some(entry) = region.deref(ctx).get_head() else {
        return IRStatus::Unchanged;
    };

    let mut status = IRStatus::Unchanged;
    let mut stack: Vec<Ptr<BasicBlock>> = vec![entry];
    let mut visited = FxHashSet::<Ptr<BasicBlock>>::default();
    while let Some(block) = stack.pop() {
        if !visited.insert(block) {
            continue;
        }

        status |= remove_blocks_inside_block(block, ctx, rewriter);

        for succ in block.deref(ctx).succs(ctx) {
            stack.push(succ);
        }
    }

    let dead_blocks: FxHashSet<Ptr<BasicBlock>> = region
        .deref(ctx)
        .iter(ctx)
        .filter(|b| !visited.contains(b))
        .collect();
    if !dead_blocks.is_empty() {
        status = IRStatus::Changed;
    }

    for dead_block in &dead_blocks {
        log::debug!("Removing unreachable block {}", dead_block.label(ctx));
        BasicBlock::drop_all_uses(*dead_block, ctx);
    }

    dead_blocks
        .iter()
        .for_each(|b| rewriter.erase_block(ctx, *b));

    status
}

/// Perform merging on blocks nested inside `op`.
/// Returns whether the IR was changed.
pub fn merge_inside_op(
    op: Ptr<Operation>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> IRStatus {
    let regions: Vec<Ptr<Region>> = op.deref(ctx).regions().collect();
    let mut status = IRStatus::Unchanged;
    //TODO: RegionBranchOpInterface should allow us to handle this less conservatively
    for region in regions {
        status |= merge_inside_region(region, ctx, rewriter);
    }
    status
}

/// Perform merging on blocks nested inside the operations of `block`.
/// Returns whether the IR was changed.
pub fn merge_inside_block(
    block: Ptr<BasicBlock>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> IRStatus {
    let ops: Vec<Ptr<Operation>> = block.deref(ctx).iter(ctx).collect();
    let mut status = IRStatus::Unchanged;
    for op in ops {
        status |= merge_inside_op(op, ctx, rewriter);
    }
    status
}

/// Perform merging on blocks nested inside `region`.
/// Returns whether the IR was changed.
pub fn merge_inside_region(
    region: Ptr<Region>,
    ctx: &mut Context,
    rewriter: &mut dyn Rewriter,
) -> IRStatus {
    if !region.deref(ctx).has_ssa_dominance(ctx) {
        let head = region
            .deref(ctx)
            .get_head()
            .expect("all regions should have entry block");
        return merge_inside_block(head, ctx, rewriter);
    }

    let Some(entry) = region.deref(ctx).get_head() else {
        return IRStatus::Unchanged;
    };

    let mut status = IRStatus::Unchanged;
    let mut stack: Vec<Ptr<BasicBlock>> = vec![entry];
    let mut visited = FxHashSet::<Ptr<BasicBlock>>::default();
    while let Some(block) = stack.pop() {
        if !visited.insert(block) {
            continue;
        }

        while try_merge_succ(block, entry, ctx, rewriter) {
            status = IRStatus::Changed;
        }

        status |= merge_inside_block(block, ctx, rewriter);

        for succ in block.deref(ctx).succs(ctx) {
            stack.push(succ);
        }
    }

    status
}

/// Simplifies the CFG, as described in the module-level documentation.
pub fn simplify_cfg(op: Ptr<Operation>, ctx: &mut Context) -> Result<IRStatus> {
    let mut fold_candidates: Vec<(Ptr<Operation>, Vec<Option<AttrObj>>)> = Vec::new();
    walk_op(
        ctx,
        &mut fold_candidates,
        &WALKCONFIG_PREORDER_FORWARD,
        op,
        |ctx, candidates, node| {
            if let IRNode::Operation(op) = node {
                let op_dyn = Operation::get_op_dyn(op, ctx);
                if op_impls::<dyn BranchOpFoldInterface>(op_dyn.as_ref()) {
                    candidates.push((op, constant_operand_attrs(op, ctx)));
                }
            }
        },
    );

    let mut rewriter = IRRewriter::<Recorder>::default();
    let mut status = IRStatus::Unchanged;
    for (op, attrs) in fold_candidates {
        rewriter.set_insertion_point_before_operation(op);
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let fold_interface = op_cast::<dyn BranchOpFoldInterface>(op_dyn.as_ref()).unwrap();
        let log_message = if log::log_enabled!(log::Level::Debug) {
            // Some implementations of `BranchOpFoldInterface` (such as with `BrOp`)
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
                "Folding branch operation '{}' with inferred operand attributes {}",
                op_dbg,
                attr_strs.join(", ")
            )
        } else {
            String::new()
        };

        let fold_result = fold_interface.fold_in_place(ctx, &attrs, &mut rewriter);
        if fold_result == IRStatus::Changed {
            log::debug!("{}", log_message);
        }
        status |= fold_result;
    }

    status |= remove_blocks_inside_op(op, ctx, &mut rewriter);
    status |= merge_inside_op(op, ctx, &mut rewriter);

    Ok(status)
}

#[derive(Default)]
/// A [Pass] that performs CFG simplification
/// as described in the module-level documentation.
pub struct SimplifyCFGPass;

impl Pass for SimplifyCFGPass {
    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_res = PassResult::default();
        pass_res.ir_changed |= simplify_cfg(op, ctx)?;
        Ok(pass_res)
    }

    fn name(&self) -> &str {
        "simplify-cfg"
    }
}
