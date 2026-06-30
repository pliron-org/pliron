//! Implementation of various op interfaces for LLVM IR instructions.

use std::num::NonZero;
use thiserror::Error;

use pliron::{
    arg_err,
    attribute::AttrObj,
    basic_block::BasicBlock,
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::{BranchOpInterface, OneResultInterface},
        types::{IntegerType, Signedness},
    },
    context::{Context, Ptr},
    derive::op_interface_impl,
    irbuild::{IRStatus, inserter::Inserter, rewriter::Rewriter},
    op::Op,
    opts::{
        constants::{BranchOpFoldInterface, ConstFoldInterface},
        dce::{BlockArgRemoval, SideEffects},
        mem2reg::{
            AllocInfo, PromotableAllocationInterface, PromotableOpInterface, PromotableOpKind,
        },
    },
    result::Result,
    utils::apint::{APInt, bw},
    value::Value,
};

use crate::{
    attributes::{ICmpPredicateAttr, IntegerOverflowFlagsAttr},
    op_interfaces::{IntBinArithOpWithOverflowFlag, NNegFlag, PointerTypeResult},
    ops::{
        AShrOp, AddOp, AddressOfOp, AllocaOp, AndOp, BitcastOp, BrOp, CondBrOp, ExtractElementOp,
        ExtractValueOp, FAddOp, FCmpOp, FDivOp, FMulOp, FNegOp, FPExtOp, FPToSIOp, FPToUIOp,
        FPTruncOp, FRemOp, FSubOp, FreezeOp, FuncOp, GetElementPtrOp, ICmpOp, InsertElementOp,
        InsertValueOp, IntToPtrOp, LShrOp, LoadOp, MulOp, OrOp, PoisonOp, PtrToIntOp, SDivOp,
        SExtOp, SIToFPOp, SRemOp, SelectOp, ShlOp, ShuffleVectorOp, StoreOp, SubOp, SwitchOp,
        TruncOp, UDivOp, UIToFPOp, URemOp, UndefOp, XorOp, ZExtOp, ZeroOp,
    },
};

#[derive(Error, Debug)]
#[error("Register Promotion: Allocation info provided is not related to this operation")]
pub struct UnrelatedAllocInfo;

#[op_interface_impl]
impl PromotableAllocationInterface for AllocaOp {
    fn alloc_info(&self, ctx: &Context) -> Vec<AllocInfo> {
        vec![AllocInfo {
            ptr: self.get_result(ctx),
            ty: self.result_pointee_type(ctx),
        }]
    }

    fn default_value(
        &self,
        ctx: &mut Context,
        inserter: &mut dyn Inserter,
        alloc_info: &AllocInfo,
    ) -> Result<Value> {
        if alloc_info.ptr != self.get_result(ctx) {
            return arg_err!(self.loc(ctx), UnrelatedAllocInfo);
        }
        let poison = PoisonOp::new(ctx, alloc_info.ty);
        let poison_val = poison.get_result(ctx);
        inserter.insert_op(ctx, &poison);
        Ok(poison_val)
    }

    fn promote(
        &self,
        ctx: &mut Context,
        rewriter: &mut dyn Rewriter,
        alloc_infos: &[AllocInfo],
    ) -> Result<()> {
        if alloc_infos.len() != 1 || alloc_infos[0].ptr != self.get_result(ctx) {
            return arg_err!(self.loc(ctx), UnrelatedAllocInfo);
        }
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}

#[op_interface_impl]
impl PromotableOpInterface for StoreOp {
    fn promotion_kind(&self, ctx: &Context, alloc_info: &AllocInfo) -> PromotableOpKind {
        if self.get_operand_address(ctx) == alloc_info.ptr {
            PromotableOpKind::Store(self.get_operand_value(ctx))
        } else {
            PromotableOpKind::NonPromotableUse
        }
    }

    fn promote(
        &self,
        ctx: &mut Context,
        alloc_info_reaching_defs: &[(AllocInfo, Value)],
        rewriter: &mut dyn Rewriter,
    ) -> Result<()> {
        if alloc_info_reaching_defs.len() != 1 {
            return arg_err!(self.loc(ctx), UnrelatedAllocInfo);
        }
        let (alloc_info, _reaching_def) = &alloc_info_reaching_defs[0];
        if self.get_operand_address(ctx) != alloc_info.ptr {
            return arg_err!(self.loc(ctx), UnrelatedAllocInfo);
        }
        rewriter.erase_operation(ctx, self.get_operation());
        Ok(())
    }
}

#[op_interface_impl]
impl PromotableOpInterface for LoadOp {
    fn promotion_kind(&self, ctx: &Context, alloc_info: &AllocInfo) -> PromotableOpKind {
        if self.get_operand_address(ctx) == alloc_info.ptr {
            PromotableOpKind::Load
        } else {
            PromotableOpKind::NonPromotableUse
        }
    }

    fn promote(
        &self,
        ctx: &mut Context,
        alloc_info_reaching_defs: &[(AllocInfo, Value)],
        rewriter: &mut dyn Rewriter,
    ) -> Result<()> {
        if alloc_info_reaching_defs.len() != 1 {
            return arg_err!(self.loc(ctx), UnrelatedAllocInfo);
        }
        let (alloc_info, reaching_def) = &alloc_info_reaching_defs[0];
        if self.get_operand_address(ctx) != alloc_info.ptr {
            return arg_err!(self.loc(ctx), UnrelatedAllocInfo);
        }
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![*reaching_def]);
        Ok(())
    }
}

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

/// Constant fold this binary integer operation, taking integer overflow flags
/// into account.
///
/// `operand_attrs` contains `Some(attr)` for operands inferred constant
/// and `None` for operands not inferred constant.
///
/// `flags` contains the llvm integer overflow flags associated with this operation
///
/// `combine` computes the wrapped result together with whether the operation
/// unsigned- and signed-overflowed (the two booleans, in that order). The
///
/// Returns a singleton vector containing the folded result, or `None` if folding
/// is not possible.
fn check_fold_int_bin_op_with_overflow(
    operand_attrs: &[Option<AttrObj>],
    flags: IntegerOverflowFlagsAttr,
    combine: impl Fn(&APInt, &APInt) -> (APInt, bool, bool),
) -> Vec<Option<AttrObj>> {
    let Some((lhs, rhs)) = get_int_bin_operands(operand_attrs) else {
        return vec![None];
    };
    let (res, unsigned_overflow, signed_overflow) = combine(&lhs.value(), &rhs.value());
    if (flags.nsw && signed_overflow) || (flags.nuw && unsigned_overflow) {
        return vec![None];
    }
    let res = Box::new(IntegerAttr::new(lhs.get_type(), res)) as AttrObj;
    vec![Some(res)]
}

/// Returns `true` if signed-dividing/remaindering `lhs` by `rhs` is undefined
/// behavior in LLVM, and so must not be constant folded. The two cases are
/// division by zero, and the signed overflow `INT_MIN / -1` (true quotient
/// `INT_MAX + 1`, not representable), whose result LLVM leaves as poison and
/// whose hardware behavior diverges (x86 traps, AArch64 wraps).
fn is_signed_div_ub(lhs: &APInt, rhs: &APInt) -> bool {
    let bw = NonZero::new(rhs.bw()).expect("operand has zero bitwidth");
    // `-1` is the all-ones bit pattern, i.e. the unsigned max.
    rhs.is_zero() || (*lhs == APInt::imin(bw) && *rhs == APInt::umax(bw))
}

/// Evaluate an integer comparison `lhs <pred> rhs`. `lhs` and `rhs` must have
/// the same bitwidth.
fn eval_icmp(pred: &ICmpPredicateAttr, lhs: &APInt, rhs: &APInt) -> bool {
    match pred {
        ICmpPredicateAttr::EQ => lhs == rhs,
        ICmpPredicateAttr::NE => lhs != rhs,
        ICmpPredicateAttr::SLT => lhs.slt(rhs),
        ICmpPredicateAttr::SLE => lhs.sle(rhs),
        ICmpPredicateAttr::SGT => lhs.sgt(rhs),
        ICmpPredicateAttr::SGE => lhs.sge(rhs),
        ICmpPredicateAttr::ULT => lhs.ult(rhs),
        ICmpPredicateAttr::ULE => lhs.ule(rhs),
        ICmpPredicateAttr::UGT => lhs.ugt(rhs),
        ICmpPredicateAttr::UGE => lhs.uge(rhs),
    }
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
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op_with_overflow(
            ops,
            self.integer_overflow_flag(ctx),
            APInt::add_overflow,
        )
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
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op_with_overflow(
            ops,
            self.integer_overflow_flag(ctx),
            APInt::sub_overflow,
        )
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
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op_with_overflow(
            ops,
            self.integer_overflow_flag(ctx),
            APInt::mul_overflow,
        )
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
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        match get_int_bin_operands(ops) {
            Some((lhs, rhs)) => {
                let shamt = rhs.value();
                let lhs_bw: usize = lhs.value().bw();
                let lhs_bw: APInt = APInt::from_usize(lhs_bw, NonZero::new(lhs_bw).unwrap());
                if shamt.ult(&lhs_bw) {
                    check_fold_int_bin_op_with_overflow(
                        ops,
                        self.integer_overflow_flag(ctx),
                        APInt::shl_overflow,
                    )
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
            Some((lhs, rhs)) if is_signed_div_ub(&lhs.value(), &rhs.value()) => vec![None],
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
            Some((lhs, rhs)) if is_signed_div_ub(&lhs.value(), &rhs.value()) => vec![None],
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
impl ConstFoldInterface for AndOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        assert!(ops.len() == 2);
        for op in ops.iter().flatten() {
            let int = op
                .downcast_ref::<IntegerAttr>()
                .expect("invalid operand type: typecheck before optimizing");
            if int.value().is_zero() {
                let zero = APInt::zero(NonZero::new(int.value().bw()).expect("zero bitwidth"));
                let res = Box::new(IntegerAttr::new(int.get_type(), zero)) as AttrObj;
                return vec![Some(res)];
            }
        }
        check_fold_int_bin_op(ops, APInt::and)
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
impl ConstFoldInterface for OrOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        assert!(ops.len() == 2);
        for op in ops.iter().flatten() {
            let int = op
                .downcast_ref::<IntegerAttr>()
                .expect("invalid operand type: typecheck before optimizing");
            let bw = NonZero::new(int.value().bw()).expect("zero bitwidth");
            if int.value() == APInt::umax(bw) {
                let all_ones = APInt::umax(bw);
                let res = Box::new(IntegerAttr::new(int.get_type(), all_ones)) as AttrObj;
                return vec![Some(res)];
            }
        }
        check_fold_int_bin_op(ops, APInt::or)
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
impl ConstFoldInterface for XorOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_bin_op(ops, APInt::xor)
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
impl ConstFoldInterface for LShrOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        // A shift amount >= the bitwidth is undefined behavior in LLVM, so it
        // must not be folded.
        match get_int_bin_operands(ops) {
            Some((lhs, rhs)) => {
                let lhs_bw = lhs.value().bw();
                let lhs_bw = APInt::from_usize(lhs_bw, NonZero::new(lhs_bw).unwrap());
                if rhs.value().ult(&lhs_bw) {
                    check_fold_int_bin_op(ops, APInt::lshr)
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
impl ConstFoldInterface for AShrOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        // A shift amount >= the bitwidth is undefined behavior in LLVM, so it
        // must not be folded.
        match get_int_bin_operands(ops) {
            Some((lhs, rhs)) => {
                let lhs_bw = lhs.value().bw();
                let lhs_bw = APInt::from_usize(lhs_bw, NonZero::new(lhs_bw).unwrap());
                if rhs.value().ult(&lhs_bw) {
                    check_fold_int_bin_op(ops, APInt::ashr)
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
impl ConstFoldInterface for ICmpOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let Some((lhs, rhs)) = get_int_bin_operands(ops) else {
            return vec![None];
        };
        let result = eval_icmp(&self.predicate(ctx), &lhs.value(), &rhs.value());
        let bool_ty = IntegerType::get_existing(ctx, 1, Signedness::Signless)
            .expect("i1 type must exist: it is the result type of this op");
        let res = Box::new(IntegerAttr::new(
            bool_ty,
            APInt::from_u8(result as u8, bw(1)),
        )) as AttrObj;
        vec![Some(res)]
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
impl ConstFoldInterface for SExtOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(operand)] = ops else {
            return vec![None];
        };
        let operand = operand
            .downcast_ref::<IntegerAttr>()
            .expect("invalid operand type: typecheck before optimizing");
        let res_ty = self.result_type(ctx);
        let dest_width = res_ty
            .deref(ctx)
            .downcast_ref::<IntegerType>()
            .expect("sext result must be an integer type")
            .width();
        let dest_ty = IntegerType::get_existing(ctx, dest_width, Signedness::Signless)
            .expect("result type must exist: it is the result type of this op");
        let extended = operand
            .value()
            .sext(NonZero::new(dest_width as usize).expect("result has zero bitwidth"));
        let res = Box::new(IntegerAttr::new(dest_ty, extended)) as AttrObj;
        vec![Some(res)]
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
impl ConstFoldInterface for ZExtOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(operand)] = ops else {
            return vec![None];
        };
        let operand = operand
            .downcast_ref::<IntegerAttr>()
            .expect("invalid operand type: typecheck before optimizing");
        // `zext nneg` asserts the operand is non-negative; if it isn't, the
        // result is poison, so we must not fold it to a concrete value.
        let value = operand.value();
        if self.nneg(ctx)
            && value.slt(&APInt::zero(
                NonZero::new(value.bw()).expect("operand has zero bitwidth"),
            ))
        {
            return vec![None];
        }
        let res_ty = self.result_type(ctx);
        let dest_width = res_ty
            .deref(ctx)
            .downcast_ref::<IntegerType>()
            .expect("zext result must be an integer type")
            .width();
        let dest_ty = IntegerType::get_existing(ctx, dest_width, Signedness::Signless)
            .expect("result type must exist: it is the result type of this op");
        let extended =
            value.zext(NonZero::new(dest_width as usize).expect("result has zero bitwidth"));
        let res = Box::new(IntegerAttr::new(dest_ty, extended)) as AttrObj;
        vec![Some(res)]
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
