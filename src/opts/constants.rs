use pliron_derive::op_interface;

use crate::{
    attribute::AttrObj,
    context::Context,
    irbuild::{IRStatus, rewriter::Rewriter},
    op::Op,
    result::Result,
};

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
