use alloc::vec::Vec;
use pliron_derive::op_interface;

use crate::{
    attribute::{AttrObj, attr_cast},
    basic_block::BasicBlock,
    builtin::{attr_interfaces::MaterializableAttr, op_interfaces::BranchOpInterface},
    context::{Context, Ptr},
    irbuild::{IRStatus, rewriter::Rewriter},
    op::Op,
    result::Result,
};

pub mod sccp;
mod state;

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
    ///
    /// Implementors that fold by materializing constant values for their results can
    /// usually delegate to [fold_with_materialization](Self::fold_with_materialization)
    /// rather than reimplementing the materialization logic.
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        operand_attrs: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus;

    /// A helper for implementing [fold_in_place](Self::fold_in_place) by materializing
    /// the constants inferred by [check_fold](Self::check_fold).
    ///
    /// Only constants whose attribute types implement [MaterializableAttr] get
    /// materialized.
    fn fold_with_materialization(
        &self,
        ctx: &mut Context,
        operand_attrs: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        let folded = self.check_fold(ctx, operand_attrs);
        let op = self.get_operation();

        let mut status = IRStatus::Unchanged;
        for (result_idx, attr) in folded.iter().enumerate() {
            let Some(attr) = attr else {
                continue;
            };
            let Some(materializable) = attr_cast::<dyn MaterializableAttr>(&**attr) else {
                log::info!(
                    "Constant propagation tried to materialize {}, but its type does not \
                     implement MaterializableAttr. This potentially prevents optimizations.",
                    attr.disp(ctx)
                );
                continue;
            };
            let const_op = materializable.materialize(ctx);
            rewriter.append_operation(ctx, const_op);
            let new_value = const_op.deref(ctx).get_result(0);
            let old_value = op.deref(ctx).get_result(result_idx);
            rewriter.replace_value_uses_with(ctx, old_value, new_value);
            status = IRStatus::Changed;
        }
        status
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Interface for ruling out branch destinations
/// based on static information about branch conditions.
#[op_interface]
pub trait BranchOpFoldInterface: BranchOpInterface {
    /// Return the list of possible successor blocks given that `operands`
    /// contains `Some(attr)` for each operand known to be constant, where `attr` contains
    /// the known constant value.
    fn check_fold(&self, ctx: &Context, operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>>;

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
