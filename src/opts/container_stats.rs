// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Collects container-size statistics for every `Operation` and `BasicBlock`
//! reachable from a root operation.
//!
//! The pass walks the IR and emits one CSV row per `Operation` and one per
//! `BasicBlock`, with columns:
//! 1. `node_type`: `op` or `block`.
//! 2. `kind`: the operation's `OpId`, or `block` for a basic block.
//! 3. `id`: a per-walk sequential id (separate numbering for ops and blocks).
//! 4. `num_results`: number of results of an operation.
//! 5. `num_operands`: number of operands (value uses) of an operation.
//! 6. `num_successors`: number of successors (basic block uses) of an operation.
//! 7. `num_attrs`: number of entries in the [AttributeDict](crate::attribute::AttributeDict)
//!    (of the operation or the basic block).
//! 8. `result_uses`: `;`-separated number of uses of each result of an operation,
//!    in result order.
//! 9. `num_args`: number of arguments of a basic block.
//! 10. `arg_uses`: `;`-separated number of uses of each argument of a basic
//!     block, in argument order.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write as _;

use crate::{
    context::{Context, Ptr},
    graph::walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    irbuild::IRStatus,
    operation::Operation,
    pass::{AnalysisManager, Pass, PassResult},
    result::Result,
};

/// One row of the CSV emitted by [ContainerStatsPass]: all the information
/// collected about a single [Operation] or `BasicBlock`.
enum Row {
    Op {
        kind: String,
        id: u64,
        num_results: usize,
        num_operands: usize,
        num_successors: usize,
        num_attrs: usize,
        /// Number of uses of each result, in result order.
        result_uses: Vec<usize>,
    },
    Block {
        id: u64,
        num_attrs: usize,
        num_args: usize,
        /// Number of uses of each argument, in argument order.
        arg_uses: Vec<usize>,
    },
}

#[derive(Default)]
struct StatsState {
    next_op_id: u64,
    next_block_id: u64,
    rows: Vec<Row>,
}

fn visit(ctx: &Context, state: &mut StatsState, node: IRNode) {
    match node {
        IRNode::Operation(op) => {
            let id = state.next_op_id;
            state.next_op_id += 1;
            let kind = Operation::get_opid(op, ctx).to_string();
            let op = op.deref(ctx);

            state.rows.push(Row::Op {
                kind,
                id,
                num_results: op.get_num_results(),
                num_operands: op.get_num_operands(),
                num_successors: op.get_num_successors(),
                num_attrs: op.attributes.0.len(),
                result_uses: op.results().map(|result| result.num_uses(ctx)).collect(),
            });
        }
        IRNode::BasicBlock(block) => {
            let id = state.next_block_id;
            state.next_block_id += 1;
            let block = block.deref(ctx);

            state.rows.push(Row::Block {
                id,
                num_attrs: block.attributes.0.len(),
                num_args: block.get_num_arguments(),
                arg_uses: block.arguments().map(|arg| arg.num_uses(ctx)).collect(),
            });
        }
        IRNode::Region(_) => {}
    }
}

/// Format a list of counts as a single CSV field, `;`-separated.
fn join_counts(counts: &[usize]) -> String {
    let mut s = String::new();
    for (i, count) in counts.iter().enumerate() {
        if i > 0 {
            s.push(';');
        }
        let _ = write!(s, "{count}");
    }
    s
}

/// Render the collected rows as CSV text (including the header line).
fn render_csv(rows: &[Row]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "node_type,kind,id,num_results,num_operands,num_successors,num_attrs,result_uses,num_args,arg_uses"
    );
    for row in rows {
        match row {
            Row::Op {
                kind,
                id,
                num_results,
                num_operands,
                num_successors,
                num_attrs,
                result_uses,
            } => {
                let _ = writeln!(
                    out,
                    "op,{kind},{id},{num_results},{num_operands},{num_successors},{num_attrs},{},0,0",
                    join_counts(result_uses)
                );
            }
            Row::Block {
                id,
                num_attrs,
                num_args,
                arg_uses,
            } => {
                let _ = writeln!(
                    out,
                    "block,block,{id},0,0,0,{num_attrs},0,{num_args},{}",
                    join_counts(arg_uses)
                );
            }
        }
    }
    out
}

#[cfg(feature = "std")]
fn print_csv(csv: &str) {
    std::print!("{csv}");
}

#[cfg(not(feature = "std"))]
fn print_csv(_csv: &str) {}

#[derive(Default)]
/// A [Pass] that walks every [Operation] and `BasicBlock` nested (transitively)
/// inside the operation it runs on, and prints CSV container-size statistics
/// to stdout. See the module-level documentation for details.
///
/// This pass never changes the IR.
pub struct ContainerStatsPass;

impl Pass for ContainerStatsPass {
    fn name(&self) -> &str {
        "container-stats"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut state = StatsState::default();
        walk_op(ctx, &mut state, &WALKCONFIG_PREORDER_FORWARD, op, visit);
        print_csv(&render_csv(&state.rows));

        let mut result = PassResult::default();
        result.ir_changed = IRStatus::Unchanged;
        Ok(result)
    }
}
