//! Implementation of various op interfaces for LLVM IR instructions.

use std::num::NonZero;

use pliron::{
    attribute::AttrObj,
    basic_block::BasicBlock,
    builtin::{attributes::IntegerAttr, op_interfaces::BranchOpInterface},
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

/// Assumes `operand_attrs` has length 2. If both elements are `Some(x)` where `x` can
/// be casted to an [IntegerAttr], return the casted results. Otherwise, return `None`.
fn get_int_bin_operands(operand_attrs: &[Option<AttrObj>]) -> Option<(IntegerAttr, IntegerAttr)> {
    assert!(operand_attrs.len() == 2);
    let [Some(lhs), Some(rhs)] = operand_attrs else {
        return None;
    };
    let lhs_int = lhs
        .downcast_ref::<IntegerAttr>()
        .expect("invalid operand type: typecheck before optimizing");
    let rhs_int = rhs
        .downcast_ref::<IntegerAttr>()
        .expect("invalid operand type: typecheck before optimizing");
    Some((lhs_int.clone(), rhs_int.clone()))
}

/// Constant fold this binary integer operation into a singleton vector
/// containing its result type if folding is successful, or None otherwise.
fn check_fold_int_bin_op(
    operand_attrs: &[Option<AttrObj>],
    combine: impl Fn(&APInt, &APInt) -> APInt,
) -> Vec<Option<AttrObj>> {
    let Some((lhs, rhs)) = get_int_bin_operands(operand_attrs) else {
        return vec![None];
    };
    let res = Box::new(IntegerAttr::new(
        lhs.get_type(),
        combine(&lhs.value(), &rhs.value()),
    )) as AttrObj;
    vec![Some(res)]
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
        self.fold_with_materialization(ctx, ops, rw)
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
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for MulOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op(ops, APInt::mul)
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for ShlOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        match get_int_bin_operands(ops) {
            Some((lhs, rhs)) => {
                let shamt = rhs.value();
                let lhs_bw: usize = lhs.value().bw();
                let lhs_bw: APInt = APInt::from_usize(lhs_bw, NonZero::new(lhs_bw).unwrap());
                if shamt.ult(&lhs_bw) {
                    check_fold_int_bin_op(ops, APInt::shl)
                } else {
                    vec![None]
                }
            }
            None => vec![None],
        }
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for UDivOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        match get_int_bin_operands(ops) {
            Some((_, rhs)) if rhs.value().is_zero() => vec![None],
            _ => check_fold_int_bin_op(ops, APInt::udiv),
        }
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for SDivOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        match get_int_bin_operands(ops) {
            Some((_, rhs)) if rhs.value().is_zero() => vec![None],
            _ => check_fold_int_bin_op(ops, APInt::sdiv),
        }
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for URemOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        match get_int_bin_operands(ops) {
            Some((_, rhs)) if rhs.value().is_zero() => vec![None],
            _ => check_fold_int_bin_op(ops, APInt::urem),
        }
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl ConstFoldInterface for SRemOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        match get_int_bin_operands(ops) {
            Some((_, rhs)) if rhs.value().is_zero() => vec![None],
            _ => check_fold_int_bin_op(ops, APInt::srem),
        }
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        self.fold_with_materialization(ctx, ops, rw)
    }
}

#[op_interface_impl]
impl BranchOpFoldInterface for BrOp {
    fn check_fold(&self, ctx: &Context, _operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>> {
        self.get_operation().deref(ctx).successors().collect()
    }
    fn fold_in_place(
        &self,
        _ctx: &mut Context,
        _ops: &[Option<AttrObj>],
        _rw: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
}

impl CondBrOp {
    fn possible_successor_indices(
        &self,
        ctx: &Context,
        operands: &[Option<AttrObj>],
    ) -> Vec<usize> {
        let Some(cond_attr) = operands.first().unwrap().as_ref() else {
            let num_successors = self.get_operation().deref(ctx).successors().count();
            return (0..num_successors).collect();
        };
        let cond_int = cond_attr
            .downcast_ref::<IntegerAttr>()
            .expect("CondBrOp condition operand must be an IntegerAttr");
        let taken = if cond_int.value().is_zero() { 1 } else { 0 };
        vec![taken]
    }
}

#[op_interface_impl]
impl BranchOpFoldInterface for CondBrOp {
    fn check_fold(&self, ctx: &Context, operands: &[Option<AttrObj>]) -> Vec<Ptr<BasicBlock>> {
        let successors: Vec<Ptr<BasicBlock>> =
            self.get_operation().deref(ctx).successors().collect();

        self.possible_successor_indices(ctx, operands)
            .iter()
            .map(|ind| successors[*ind])
            .collect()
    }

    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        let possible_successor_indices = self.possible_successor_indices(ctx, ops);
        if possible_successor_indices.len() != 1 {
            return IRStatus::Unchanged;
        };
        let successor_ind = possible_successor_indices[0];
        let successors: Vec<Ptr<BasicBlock>> =
            self.get_operation().deref(ctx).successors().collect();
        let new_op = BrOp::new(
            ctx,
            successors[successor_ind],
            self.successor_operands(ctx, successor_ind),
        )
        .get_operation();
        let old_op = self.get_operation();
        rewriter.insert_operation(ctx, new_op);
        rewriter.replace_operation(ctx, old_op, new_op);
        IRStatus::Changed
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

    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        let Some(cond_attr) = ops.first().unwrap().as_ref() else {
            return IRStatus::Unchanged;
        };
        let cond_int = cond_attr
            .downcast_ref::<IntegerAttr>()
            .expect("Switch condition operand must be an IntegerAttr")
            .value();
        let successor_ind = {
            let case_values = self
                .get_attr_switch_case_values(ctx)
                .expect("SwitchOp missing case values attribute");
            case_values
                .0
                .iter()
                .position(|case| case.value() == cond_int)
                // There is no case value corresponding to the default successor,
                // so case_values index 0 corresponds to succesors index 1, etc.
                .map(|i| i + 1)
                // successor index 0 is the default successor
                .unwrap_or(0)
        };
        let successors: Vec<Ptr<BasicBlock>> =
            self.get_operation().deref(ctx).successors().collect();
        let new_op = BrOp::new(
            ctx,
            successors[successor_ind],
            self.successor_operands(ctx, successor_ind),
        )
        .get_operation();
        let old_op = self.get_operation();
        rewriter.insert_operation(ctx, new_op);
        rewriter.replace_operation(ctx, old_op, new_op);
        IRStatus::Changed
    }
}
