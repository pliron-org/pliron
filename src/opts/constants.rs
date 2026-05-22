use pliron_derive::op_interface;

use crate::{
    attribute::AttrObj,
    context::Context,
    irbuild::{IRStatus, rewriter::Rewriter},
    op::Op,
    result::Result,
};

#[op_interface]
pub trait ConstFoldInterface {
    /// Takes a slice `operand_attrs` containing `Some(attr)`` at position `i` if operand `i` is a compile-time
    /// constant (where `attr` contains the constant value) and `None` at position `i` otherwise.
    /// Produces a similar vector whose elements convey whether each of this operations's results
    /// are known compile-time constants.
    fn check_fold(&self, ctx: &Context, operand_attrs: &[Option<AttrObj>]) -> Vec<Option<AttrObj>>;

    /// Takes a slice `operand_attrs` containing `Some(attr)` at position `i` if operand `i` is a compile-time
    /// constant (where `attr` contains the constant value) and `None` at position `i` otherwise.
    /// Uses this knowledge to rewrite this operation into a cheaper form
    /// (e.g., perform compile-time arithmetic on compile-time constants,
    /// rewriting an add op to a constant op). Assumes `rewriter`'s insertion point starts before the operation.
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
