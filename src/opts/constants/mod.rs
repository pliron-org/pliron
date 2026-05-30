use pliron_derive::op_interface;

use crate::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    builtin::op_interfaces::BranchOpInterface,
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
