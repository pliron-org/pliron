//! Dead code elimination. Safely eliminate unused operations and block args.
//!
//! This pass removes operations that satisfy both the following conditions:
//! 1. The operation has no side effects.
//! 2. The operation's (SSA) results are not used by any other operation.
//!
//! An operation is considered to have side effects if either:
//! 1. It does not implement the [SideEffects] interface, or
//! 2. It implements the [SideEffects] interface and returns
//!    `true` for the `has_side_effects` method.
//!
//! This pass also removes block arguments with no uses and
//! when the parent operation
//! 1. Implements the [BlockArgRemoval] interface, and
//! 2. Returns `true` for the `can_remove_block_args` method.

use alloc::vec::Vec;
use pliron_derive::op_interface;
use rustc_hash::FxHashSet;

use crate::{
    basic_block::BasicBlock,
    builtin::op_interfaces::BranchOpInterface,
    context::{Context, Ptr},
    graph::{
        HasLabel,
        dominance::DomInfo,
        walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::mutable::walk_op},
    },
    irbuild::{
        IRStatus,
        listener::{Recorder, RecorderEvent},
        rewriter::{IRRewriter, Rewriter},
    },
    op::{Op, op_cast, op_impls},
    operation::{OpDbg, Operation},
    pass::{AnalysisManager, Pass, PassResult},
    printable::Printable,
    result::Result,
    value::{DefiningEntity, Value},
};

/// Convey the presence of side effects in an operation.
#[op_interface]
pub trait SideEffects {
    /// Returns `true` if the operation has side effects, and `false` otherwise.
    fn has_side_effects(&self, ctx: &Context) -> bool;

    // Generally there's nothing to verify here. Can be overrided.
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Safety for removal of block arguments.
/// For example, removal of entry block args in a function requires updating
/// the function signature. In such cases, the [Op] can choose not to have
/// such arguments removed by this pass.
#[op_interface]
pub trait BlockArgRemoval {
    /// Returns `true` if arguments of `block` can be safely removed, and `false` otherwise.
    fn can_remove_block_args(&self, ctx: &Context, block: Ptr<BasicBlock>) -> bool;

    // Generally there's nothing to verify here. Can be overrided.
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Elimination candidate, which can be either a dead operation or a dead block argument.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DCECandidate {
    Op(Ptr<Operation>),
    BlockArg(Value),
}

impl Printable for DCECandidate {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &crate::printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            DCECandidate::Op(op) => write!(f, "Operation {}", OpDbg { op: *op, ctx }),
            DCECandidate::BlockArg(arg) => {
                let block = arg.defining_block().expect("Expected a block argument");
                write!(
                    f,
                    "Block argument {} of block {}",
                    arg.find_index(ctx),
                    block.label(ctx)
                )
            }
        }
    }
}

/// Can this [Value] be safely removed as per our DCE criteria?
fn is_safe_to_erase(cand: DCECandidate, ctx: &Context) -> bool {
    match cand {
        DCECandidate::Op(def_op) => {
            let def_op_ref = def_op.deref(ctx);
            if def_op_ref.has_use() {
                // If the operation has uses, we can't erase it.
                return false;
            }
            // Get the dynamic op interface to check for SideEffects
            let def_op_dyn = Operation::get_op_dyn(def_op, ctx);
            // Determine if operation has side effects
            let has_side_effects = match op_cast::<dyn SideEffects>(&*def_op_dyn) {
                Some(side_effects_op) => side_effects_op.has_side_effects(ctx),
                None => true, // If it doesn't implement SideEffects, assume it has side effects
            };
            !has_side_effects
        }
        DCECandidate::BlockArg(arg) => {
            let block = arg.defining_block().expect("Expected a block argument");
            if arg.is_used(ctx) {
                // If the block argument has uses, we can't erase it.
                return false;
            }
            // Get the parent operation of the block
            let parent_op = block
                .deref(ctx)
                .get_parent_op(ctx)
                .expect("Expected block to have a parent operation");
            let parent_op_dyn = Operation::get_op_dyn(parent_op, ctx);
            // Check if the parent operation allows block argument removal
            let Some(block_arg_removal_op) = op_cast::<dyn BlockArgRemoval>(&*parent_op_dyn) else {
                return false;
            };
            if !block_arg_removal_op.can_remove_block_args(ctx, block) {
                return false;
            }
            // Every predecessor must have a `BranchOpInterface` impl
            // that allows removal of the corresponding successor operand
            block.preds(ctx).iter().all(|pred| {
                let pred_terminator = pred
                    .deref(ctx)
                    .get_terminator(ctx)
                    .expect("Expected predecessor block to have a terminator");
                let pred_terminator_dyn = Operation::get_op_dyn(pred_terminator, ctx);
                op_impls::<dyn crate::builtin::op_interfaces::BranchOpInterface>(
                    &*pred_terminator_dyn,
                )
            })
        }
    }
}

/// Process the events in the recorder to note down erased operations.
fn note_erased_ops(
    recorder: &mut Recorder,
    erased_ops: &mut FxHashSet<Ptr<Operation>>,
    erased_blocks: &mut FxHashSet<Ptr<BasicBlock>>,
) {
    for event in recorder.events.drain(..) {
        match event {
            RecorderEvent::ErasedOperation(op) => {
                erased_ops.insert(op);
            }
            RecorderEvent::ErasedBlock(block) => {
                erased_blocks.insert(block);
            }
            RecorderEvent::ErasedRegion(_) => {
                // We don't need to track erased regions
            }
            RecorderEvent::UnlinkedBlock(_, _)
            | RecorderEvent::InsertedOperation(_)
            | RecorderEvent::InsertedBlock(_)
            | RecorderEvent::ReplacedValueUses { .. }
            | RecorderEvent::ValueTypeChanged { .. }
            | RecorderEvent::UnlinkedOperation(_, _) => {
                panic!("Unexpected event in DCE recorder: {:?}", event);
            }
        }
    }
}

/// Perform dead code elimination on the given operation and its nested operations.
/// See module-level documentation for details on the DCE criteria and interfaces.
pub fn dce(op: Ptr<Operation>, ctx: &mut Context) -> Result<IRStatus> {
    let mut rewriter = IRRewriter::<Recorder>::default();

    let mut cemetery: Vec<DCECandidate> = Vec::new();
    let mut erased_blocks: FxHashSet<Ptr<BasicBlock>> = FxHashSet::default();
    let mut erased_ops: FxHashSet<Ptr<Operation>> = FxHashSet::default();

    // Step 1: Recursively walk the operation tree to collect all dead operations
    walk_op(
        ctx,
        &mut cemetery,
        &WALKCONFIG_PREORDER_FORWARD,
        op,
        |ctx, cemetery, ir_node| match ir_node {
            IRNode::Operation(opr) => {
                let cand = DCECandidate::Op(opr);
                if is_safe_to_erase(cand, ctx) {
                    log::trace!("Adding to DCE cemetery: {}", cand.disp(ctx));
                    cemetery.push(cand);
                }
            }
            IRNode::BasicBlock(block) => {
                for arg in block.deref(ctx).arguments() {
                    let cand = DCECandidate::BlockArg(arg);
                    if is_safe_to_erase(cand, ctx) {
                        log::trace!("Adding to DCE cemetery: {}", cand.disp(ctx));
                        cemetery.push(cand);
                    }
                }
            }
            IRNode::Region(_) => {}
        },
    );

    // Step 2: Process the cemetery (dead operations)
    let mut modified = IRStatus::Unchanged;

    while let Some(dead) = cemetery.pop() {
        let operands_of_dead = match dead {
            DCECandidate::BlockArg(arg) => {
                let block = arg.defining_block().expect("Expected a block argument");
                if erased_blocks.contains(&block) {
                    // If the block is already erased, skip processing this block argument
                    continue;
                }
                let opd_idx = arg.find_index(ctx);
                let successor_operands = block
                    .uses(ctx)
                    .iter()
                    .map(|pred| {
                        let succ_idx = pred.find_index(ctx);
                        let pred_terminator_dyn = Operation::get_op_dyn(pred.user_op, ctx);
                        let branch_interface =
                            op_cast::<dyn BranchOpInterface>(&*pred_terminator_dyn)
                                .expect("Terminator must implement BranchOpInterface");
                        branch_interface.remove_successor_operand(ctx, succ_idx, opd_idx)
                    })
                    .collect::<Vec<_>>();
                log::debug!(
                    "Erasing block argument {} of block {}",
                    opd_idx,
                    block.label(ctx)
                );
                BasicBlock::remove_argument(block, ctx, opd_idx);
                successor_operands
            }
            DCECandidate::Op(dead_op) => {
                if erased_ops.contains(&dead_op) {
                    // If the operation is already erased, skip processing this operation
                    continue;
                }
                // Collect defining values before erasing this one
                let defining_vals: Vec<Value> = dead_op.deref(ctx).operands().collect();
                // Erase the dead operation
                log::debug!("Erasing dead operation: {}", OpDbg { op: dead_op, ctx });
                rewriter.erase_operation(ctx, dead_op);
                note_erased_ops(
                    rewriter.get_listener_mut(),
                    &mut erased_ops,
                    &mut erased_blocks,
                );
                defining_vals
            }
        };
        modified = IRStatus::Changed;

        // For each operand, check if the defining value is now dead
        for def_val in operands_of_dead {
            // If no side effects, add to cemetery for processing
            let dce_cand = match def_val.defining_entity() {
                DefiningEntity::Op(def_op) => DCECandidate::Op(def_op),
                DefiningEntity::Block(_) => DCECandidate::BlockArg(def_val),
            };
            if is_safe_to_erase(dce_cand, ctx) {
                log::trace!("Adding to DCE cemetery: {}", dce_cand.disp(ctx));
                cemetery.push(dce_cand);
            }
        }
    }

    Ok(modified)
}

#[derive(Default)]
/// A [Pass] that performs dead code elimination as described in the module-level documentation.
pub struct DCEPass;

impl Pass for DCEPass {
    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_res = PassResult::default();
        // Run DCE on the entire operation tree rooted at `op`
        pass_res.ir_changed |= dce(op, ctx)?;
        // DCE does not touch the CFG structure, so we can preserve dominator info if it exists.
        pass_res.set_preserved::<DomInfo>();
        Ok(pass_res)
    }

    fn name(&self) -> &str {
        "dce"
    }
}
