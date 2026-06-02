//! Implementation of various op interfaces for LLVM IR instructions.

use pliron::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    builtin::{attributes::IntegerAttr, ops::ConstantOp},
    context::{Context, Ptr},
    derive::op_interface_impl,
    irbuild::{IRStatus, rewriter::Rewriter},
    op::Op,
    opts::{
        constants::{BranchOpFoldInterface, ConstFoldInterface},
        dce::{BlockArgRemoval, SideEffects},
    },
    utils::apint::APInt,
};

use crate::ops::{
    AShrOp, AddOp, AddressOfOp, AllocaOp, AndOp, BitcastOp, BrOp, CondBrOp, ExtractElementOp,
    ExtractValueOp, FAddOp, FCmpOp, FDivOp, FMulOp, FNegOp, FPExtOp, FPToSIOp, FPToUIOp, FPTruncOp,
    FRemOp, FSubOp, FreezeOp, FuncOp, GetElementPtrOp, ICmpOp, InsertElementOp, InsertValueOp,
    IntToPtrOp, LShrOp, MulOp, OrOp, PoisonOp, PtrToIntOp, SDivOp, SExtOp, SIToFPOp, SRemOp,
    SelectOp, ShlOp, ShuffleVectorOp, SubOp, SwitchOp, TruncOp, UDivOp, UIToFPOp, URemOp, UndefOp,
    XorOp, ZExtOp, ZeroOp,
};

// Implement [SideEffects] with `has_side_effects` returning `false`
macro_rules! impl_side_effects_false {
  ($($op:ty),+ $(,)?) => {
    $(
      #[op_interface_impl]
      impl SideEffects for $op {
        fn has_side_effects(&self, _ctx: &Context) -> bool {
          false
        }
      }
    )+
  };
}

// Pure value-producing ops with no memory/control side effects.
// We don't need to implement [SideEffects] for the other ops,
// because the assumption is that the absense of the interface
// implies the presence of side effects, which is a safe default for DCE.
impl_side_effects_false!(
    AddOp,
    SubOp,
    MulOp,
    ShlOp,
    UDivOp,
    SDivOp,
    URemOp,
    SRemOp,
    AndOp,
    OrOp,
    XorOp,
    LShrOp,
    AShrOp,
    ICmpOp,
    AllocaOp,
    BitcastOp,
    IntToPtrOp,
    PtrToIntOp,
    UndefOp,
    PoisonOp,
    FreezeOp,
    ZeroOp,
    AddressOfOp,
    SExtOp,
    ZExtOp,
    FPExtOp,
    TruncOp,
    FPTruncOp,
    FPToSIOp,
    FPToUIOp,
    SIToFPOp,
    UIToFPOp,
    InsertValueOp,
    ExtractValueOp,
    InsertElementOp,
    ExtractElementOp,
    ShuffleVectorOp,
    SelectOp,
    FNegOp,
    FAddOp,
    FSubOp,
    FMulOp,
    FDivOp,
    FRemOp,
    FCmpOp,
    GetElementPtrOp,
);

#[op_interface_impl]
impl BlockArgRemoval for FuncOp {
    fn can_remove_block_args(&self, ctx: &Context, block: Ptr<BasicBlock>) -> bool {
        !matches!(self.get_entry_block(ctx), Some(entry) if entry == block)
    }
}

/// If all elements of `operand_attrs` are `Some(x)`, combine the operands
/// and return the result. Otherwise, return None. Assumes that the
/// concrete type of the attributes are `IntegerAttr`.
fn fold_int_bin_operands(
    operand_attrs: &[Option<AttrObj>],
    combine: impl Fn(&APInt, &APInt) -> APInt,
) -> Option<AttrObj> {
    let [Some(lhs), Some(rhs)] = operand_attrs else {
        return None;
    };
    let lhs_int = lhs
        .downcast_ref::<IntegerAttr>()
        .expect("invalid operand type: typecheck before optimizing");
    let rhs_int = rhs
        .downcast_ref::<IntegerAttr>()
        .expect("invalid operand type: typecheck before optimizing");
    Some(Box::new(IntegerAttr::new(
        lhs_int.get_type(),
        combine(&lhs_int.value(), &rhs_int.value()),
    )) as AttrObj)
}

/// Constant fold this binary operation into a singleton vector containing
/// its result type if folding is successful, or None otherwise.
fn check_fold_int_bin_op(
    operand_attrs: &[Option<AttrObj>],
    combine: impl Fn(&APInt, &APInt) -> APInt,
) -> Vec<Option<AttrObj>> {
    vec![fold_int_bin_operands(operand_attrs, combine)]
}

/// Attempt to perform constant folding the given operation
fn fold_in_place_int_bin_op(
    op: &impl Op,
    ctx: &mut Context,
    operand_attrs: &[Option<AttrObj>],
    rewriter: &mut dyn Rewriter,
    combine: impl Fn(&APInt, &APInt) -> APInt,
) -> IRStatus {
    let Some(folded) = fold_int_bin_operands(operand_attrs, combine) else {
        return IRStatus::Unchanged;
    };
    let new_const = ConstantOp::new(ctx, folded);
    let old_op = op.get_operation();
    let new_op = new_const.get_operation();
    rewriter.insert_operation(ctx, new_op);
    rewriter.replace_operation(ctx, old_op, new_op);
    IRStatus::Changed
}

#[op_interface_impl]
impl ConstFoldInterface for AddOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op(ops, APInt::add)
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        fold_in_place_int_bin_op(self, ctx, ops, rw, APInt::add)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for SubOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op(ops, APInt::sub)
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        fold_in_place_int_bin_op(self, ctx, ops, rw, APInt::sub)
    }
}

#[op_interface_impl]
impl BranchOpFoldInterface for BrOp {
    fn check_fold(&self, ctx: &Context, _operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>> {
        self.get_operation().deref(ctx).successors().collect()
    }
}

#[op_interface_impl]
impl BranchOpFoldInterface for CondBrOp {
    fn check_fold(&self, ctx: &Context, operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>> {
        let successors: Vec<Ptr<BasicBlock>> =
            self.get_operation().deref(ctx).successors().collect();
        let Some(cond_attr) = operands.first().and_then(|o| o.as_ref()) else {
            return successors;
        };
        let cond_int = cond_attr
            .downcast_ref::<IntegerAttr>()
            .expect("CondBrOp condition operand must be an IntegerAttr");
        let taken = if cond_int.value().is_zero() { 1 } else { 0 };
        vec![successors[taken]]
    }
}

#[op_interface_impl]
impl BranchOpFoldInterface for SwitchOp {
    fn check_fold(&self, ctx: &Context, operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>> {
        let successors: Vec<Ptr<BasicBlock>> =
            self.get_operation().deref(ctx).successors().collect();
        let Some(cond_attr) = operands.first().and_then(|o| o.as_ref()) else {
            return successors;
        };
        let cond_int = cond_attr
            .downcast_ref::<IntegerAttr>()
            .expect("Switch condition operand must be an IntegerAttr")
            .value();
        // Successor 0 is the default destination; successors 1..N correspond to case_values[0..N-1].
        let case_values = self
            .get_attr_switch_case_values(ctx)
            .expect("SwitchOp missing case values attribute");
        let taken = case_values
            .0
            .iter()
            .position(|case| case.value() == cond_int)
            .map(|i| i + 1)
            .unwrap_or(0);
        vec![successors[taken]]
    }
}
