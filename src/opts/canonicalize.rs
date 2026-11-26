// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Canonicalization: Dialect specified op-local simplifications.
//!
//! [Canonicalize] applies [CanonicalizeInterface::canonicalize] until fixpoint.

use pliron_derive::op_interface;

use crate::{
    context::{Context, Ptr},
    irbuild::{
        IRStatus,
        match_rewrite::{MatchRewrite, MatchRewriter, RewriterOrder, apply_match_rewrite},
        rewriter::Rewriter,
    },
    op::{Op, op_cast, op_impls},
    operation::Operation,
    pass::{AnalysisManager, Pass, PassResult},
    result::Result,
};

/// Interface for op-local simplifications of operations.
#[op_interface]
pub trait CanonicalizeInterface {
    /// Simplify `self`. `rewriter` is positioned just before this op.
    ///
    /// Implementations must make IR modifications through `rewriter`:
    /// when an operation is modified directly (bypassing `rewriter`),
    /// it must be [reported](Rewriter::notify_operation_modified).
    ///
    /// All canonicalization sequences must reach a fixpoint
    /// (i.e., no further modification is applicable) for termination.
    fn canonicalize(&self, ctx: &mut Context, rewriter: &mut dyn Rewriter) -> IRStatus;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// [MatchRewrite] for [canonicalization](CanonicalizeInterface).
#[derive(Default)]
pub struct CanonicalizeRewrite;

impl MatchRewrite for CanonicalizeRewrite {
    fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
        op_impls::<dyn CanonicalizeInterface>(Operation::get_op_dyn(op, ctx).as_ref())
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut MatchRewriter,
        op: Ptr<Operation>,
    ) -> Result<()> {
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let canonicalizable = op_cast::<dyn CanonicalizeInterface>(op_dyn.as_ref())
            .expect("Matched operation must implement CanonicalizeInterface");
        if canonicalizable.canonicalize(ctx, rewriter) == IRStatus::Changed {
            rewriter.mark_modified();
        }
        Ok(())
    }
}

/// A pass that [canonicalizes](CanonicalizeInterface) operations.
#[derive(Default)]
pub struct Canonicalize {
    rewrite_order: RewriterOrder,
}

impl Canonicalize {
    /// Set the rewrite order.
    pub fn set_rewrite_order(mut self, order: RewriterOrder) -> Self {
        self.rewrite_order = order;
        self
    }
}

impl Pass for Canonicalize {
    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut pass_result = PassResult::default();
        pass_result.ir_changed |= canonicalize_with_order(op, ctx, &self.rewrite_order)?;
        Ok(pass_result)
    }

    fn name(&self) -> &str {
        "canonicalize"
    }
}

/// Canonicalize `root_op` and all its nested ops, until fixpoint.
pub fn canonicalize(root_op: Ptr<Operation>, ctx: &mut Context) -> Result<IRStatus> {
    canonicalize_with_order(root_op, ctx, &RewriterOrder::default())
}

/// Maximum number of iterations in [canonicalize_with_order].
const MAX_ITERATIONS: usize = 10;

/// [canonicalize] with the given rewrite order.
fn canonicalize_with_order(
    root_op: Ptr<Operation>,
    ctx: &mut Context,
    order: &RewriterOrder,
) -> Result<IRStatus> {
    // Repeat until fixpoint (or MAX_ITERATIONS is hit).
    let mut status = IRStatus::Unchanged;
    for _ in 0..MAX_ITERATIONS {
        if apply_match_rewrite(ctx, &mut CanonicalizeRewrite, order.clone(), root_op)?
            == IRStatus::Unchanged
        {
            return Ok(status);
        }
        status = IRStatus::Changed;
        // A single iteration does not guarantee fixpoint: a rule can depend on any IR it reads,
        // but a rewrite triggers matches only on ops that it inserts or modifies.
    }
    log::info!("Canonicalization did not reach a fixpoint in {MAX_ITERATIONS} iterations");
    Ok(status)
}
