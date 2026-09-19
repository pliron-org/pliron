// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Implementation of various op interfaces for LLVM IR instructions.

use alloc::{boxed::Box, vec, vec::Vec};
use core::{cmp::Ordering, num::NonZero};
use thiserror::Error;

use pliron::{
    arg_err,
    attribute::{AttrObj, Attribute, attr_cast},
    basic_block::BasicBlock,
    builtin::{
        attr_interfaces::{FloatAttr, TypedAttrInterface},
        attributes::{FPDoubleAttr, FPHalfAttr, FPSingleAttr, IntegerAttr},
        op_interfaces::{BranchOpInterface, OneResultInterface},
        types::{FP16Type, FP32Type, FP64Type, IntegerType, Signedness},
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
    r#type::{TypeHandle, Typed, TypedHandle},
    utils::{
        apfloat::{Double, Float, FloatConvert, Half, Round, Single, Status},
        apint::{APInt, bw},
    },
    value::Value,
};

use crate::{
    attributes::{
        AggregateAttr, FCmpPredicateAttr, FastmathFlags, FastmathFlagsAttr, ICmpPredicateAttr,
        IntegerOverflowFlagsAttr, SplatAttr,
    },
    op_interfaces::{FastMathFlags, IntBinArithOpWithOverflowFlag, NNegFlag, PointerTypeResult},
    ops::{
        AShrOp, AddOp, AddressOfOp, AllocaOp, AndOp, BitcastOp, BrOp, CondBrOp, ConstantOp,
        ExtractElementOp, ExtractValueOp, FAddOp, FCmpOp, FDivOp, FMulOp, FNegOp, FPExtOp,
        FPToSIOp, FPToUIOp, FPTruncOp, FRemOp, FSubOp, FreezeOp, FuncOp, GetElementPtrOp, ICmpOp,
        InsertElementOp, InsertValueOp, IntToPtrOp, LShrOp, LoadOp, MulOp, OrOp, PoisonOp,
        PtrToIntOp, SDivOp, SExtOp, SIToFPOp, SRemOp, SelectOp, ShlOp, ShuffleVectorOp, StoreOp,
        SubOp, SwitchOp, TruncOp, UDivOp, UIToFPOp, URemOp, UndefOp, XorOp, ZExtOp, ZeroOp,
    },
    types::VectorType,
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
    ConstantOp,
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

/// Checks for and return two constant integer operands.
fn get_int_bin_operands(operand_attrs: &[Option<AttrObj>]) -> Option<(IntegerAttr, IntegerAttr)> {
    let [Some(lhs), Some(rhs)] = operand_attrs else {
        assert!(operand_attrs.len() == 2);
        return None;
    };
    let lhs_int = lhs.downcast_ref::<IntegerAttr>()?;
    let rhs_int = rhs.downcast_ref::<IntegerAttr>()?;
    Some((lhs_int.clone(), rhs_int.clone()))
}

/// Create an `i1` attribute.
fn bool_attr(ctx: &Context, value: bool) -> AttrObj {
    let bool_ty = IntegerType::get(ctx, 1, Signedness::Signless);
    Box::new(IntegerAttr::new(
        bool_ty,
        APInt::from_u8(value as u8, bw(1)),
    )) as AttrObj
}

#[op_interface_impl]
impl ConstFoldInterface for ConstantOp {
    fn check_fold(
        &self,
        ctx: &Context,
        _operand_attrs: &[Option<AttrObj>],
    ) -> Vec<Option<AttrObj>> {
        vec![Some(
            pliron::dyn_clone::clone_box(&*self.get_value(ctx)) as AttrObj
        )]
    }

    fn fold_in_place(
        &self,
        _ctx: &mut Context,
        _operand_attrs: &[Option<AttrObj>],
        _rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
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

/// Constant fold a binary integer operation.
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

/// Is signed-dividing/remaindering `lhs` by `rhs` undefined behavior in LLVM?
fn is_signed_div_ub(lhs: &APInt, rhs: &APInt) -> bool {
    let bw = NonZero::new(rhs.bw()).expect("operand has zero bitwidth");
    // `-1` is the all-ones bit pattern, i.e. the unsigned max.
    rhs.is_zero() || (*lhs == APInt::imin(bw) && *rhs == APInt::umax(bw))
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
            let Some(int) = op.downcast_ref::<IntegerAttr>() else {
                return vec![None];
            };
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
            let Some(int) = op.downcast_ref::<IntegerAttr>() else {
                return vec![None];
            };
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

/// Evaluate an integer comparison `lhs <pred> rhs`.
fn eval_icmp(pred: &ICmpPredicateAttr, lhs: &APInt, rhs: &APInt) -> bool {
    assert!(lhs.bw() == rhs.bw());
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

#[op_interface_impl]
impl ConstFoldInterface for ICmpOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let Some((lhs, rhs)) = get_int_bin_operands(ops) else {
            return vec![None];
        };
        let result = eval_icmp(&self.predicate(ctx), &lhs.value(), &rhs.value());
        vec![Some(bool_attr(ctx, result))]
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

/// Return a fixed vector constant's type and length.
fn fixed_vector_shape(ctx: &Context, vector: &dyn Attribute) -> Option<(TypeHandle, usize)> {
    if let Some(aggregate) = vector.downcast_ref::<AggregateAttr>() {
        let ty = aggregate.ty();
        if ty.deref(ctx).downcast_ref::<VectorType>()?.is_scalable() {
            return None;
        }
        return Some((ty, aggregate.elements().len()));
    }

    let splat = vector.downcast_ref::<SplatAttr>()?;
    let ty = splat.ty();
    let vector_ty = ty.deref(ctx);
    if vector_ty.is_scalable() {
        return None;
    }
    Some((ty.into(), vector_ty.num_elements() as usize))
}

/// Clone a fixed vector element. Panics if `index` is out of bounds.
fn fixed_vector_element(vector: &dyn Attribute, index: usize) -> Box<dyn TypedAttrInterface> {
    if let Some(aggregate) = vector.downcast_ref::<AggregateAttr>() {
        return aggregate.elements()[index].clone();
    }
    let splat = vector
        .downcast_ref::<SplatAttr>()
        .expect("not a fixed-length vector constant");
    pliron::dyn_clone::clone_box(splat.element())
}

/// Fold a shuffle of two fixed vector constants.
fn fold_shuffle_vector(
    ctx: &Context,
    lhs: &dyn Attribute,
    rhs: &dyn Attribute,
    mask: &[i32],
    result_ty: TypeHandle,
) -> Option<AttrObj> {
    let (_, lhs_len) = fixed_vector_shape(ctx, lhs)?;
    let (_, rhs_len) = fixed_vector_shape(ctx, rhs)?;

    // The mask indexes the concatenated inputs.
    let mut elements: Vec<Box<dyn TypedAttrInterface>> = Vec::with_capacity(mask.len());
    for &mask_index in mask {
        // Negative entries are poison and remain unfolded.
        let index = usize::try_from(mask_index).ok()?;
        let element = if index < lhs_len {
            fixed_vector_element(lhs, index)
        } else if index - lhs_len < rhs_len {
            fixed_vector_element(rhs, index - lhs_len)
        } else {
            return None;
        };
        elements.push(element);
    }

    Some(Box::new(AggregateAttr::new(elements, result_ty)) as AttrObj)
}

#[op_interface_impl]
impl ConstFoldInterface for ShuffleVectorOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(lhs), Some(rhs)] = ops else {
            assert!(ops.len() == 2);
            return vec![None];
        };
        let mask = self
            .get_attr_llvm_shuffle_vector_mask(ctx)
            .expect("ShuffleVectorOp missing mask attribute");
        let result_ty = self.get_result(ctx).get_type(ctx);
        vec![fold_shuffle_vector(ctx, &**lhs, &**rhs, &mask.0, result_ty)]
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

/// Convert `index` to an in-bounds vector index.
fn constant_vector_index(index: &IntegerAttr, length: usize) -> Option<usize> {
    let value = index.value();
    // Conversion supports at most 128 bits.
    if value.bw() > 128 {
        return None;
    }
    let index: usize = value.to_u128().try_into().ok()?;
    (index < length).then_some(index)
}

#[op_interface_impl]
impl ConstFoldInterface for ExtractElementOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(vector), Some(index)] = ops else {
            assert!(ops.len() == 2);
            return vec![None];
        };
        let Some(index) = index.downcast_ref::<IntegerAttr>() else {
            return vec![None];
        };
        let folded = fixed_vector_shape(ctx, &**vector)
            .and_then(|(_, length)| constant_vector_index(index, length))
            .map(|index| fixed_vector_element(&**vector, index) as AttrObj);
        vec![folded]
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

/// Return a fixed vector constant's elements and type.
fn fixed_vector_elements(
    ctx: &Context,
    vector: &dyn Attribute,
) -> Option<(Vec<Box<dyn TypedAttrInterface>>, TypeHandle)> {
    let (ty, length) = fixed_vector_shape(ctx, vector)?;
    let elements = (0..length)
        .map(|index| fixed_vector_element(vector, index))
        .collect();
    Some((elements, ty))
}

#[op_interface_impl]
impl ConstFoldInterface for InsertElementOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(vector), Some(element), Some(index)] = ops else {
            assert!(ops.len() == 3);
            return vec![None];
        };
        let element = attr_cast::<dyn TypedAttrInterface>(&**element)
            .expect("invalid operand type: typecheck before optimizing");
        let Some(index) = index.downcast_ref::<IntegerAttr>() else {
            return vec![None];
        };
        let folded = fixed_vector_elements(ctx, &**vector).and_then(|(mut elements, ty)| {
            let index = constant_vector_index(index, elements.len())?;
            *elements.get_mut(index)? = pliron::dyn_clone::clone_box(element);
            Some(Box::new(AggregateAttr::new(elements, ty)) as AttrObj)
        });
        vec![folded]
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

/// Extract an element along `indices`.
fn extract_from_aggregate_attr(aggregate: &AggregateAttr, indices: &[u32]) -> Option<AttrObj> {
    let (&index, rest) = indices.split_first()?;
    let element = aggregate.elements().get(index as usize)?;

    if rest.is_empty() {
        return Some(pliron::dyn_clone::clone_box(&**element as &dyn Attribute));
    }

    let nested = (&**element as &dyn Attribute).downcast_ref::<AggregateAttr>()?;
    extract_from_aggregate_attr(nested, rest)
}

#[op_interface_impl]
impl ConstFoldInterface for ExtractValueOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(aggregate)] = ops else {
            assert!(ops.len() == 1);
            return vec![None];
        };
        let Some(aggregate) = aggregate.downcast_ref::<AggregateAttr>() else {
            return vec![None];
        };
        vec![extract_from_aggregate_attr(aggregate, &self.indices(ctx))]
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

/// Insert `value` along `indices`.
fn insert_into_aggregate_attr(
    aggregate: &AggregateAttr,
    value: &dyn TypedAttrInterface,
    indices: &[u32],
) -> Option<AggregateAttr> {
    let (&index, rest) = indices.split_first()?;
    let index = index as usize;

    let replacement: Box<dyn TypedAttrInterface> = if rest.is_empty() {
        pliron::dyn_clone::clone_box(value)
    } else {
        let current = aggregate.elements().get(index)?;
        let nested = (&**current as &dyn Attribute).downcast_ref::<AggregateAttr>()?;
        Box::new(insert_into_aggregate_attr(nested, value, rest)?)
    };

    let mut elements = aggregate.elements().to_vec();
    *elements.get_mut(index)? = replacement;
    Some(AggregateAttr::new(elements, aggregate.ty()))
}

#[op_interface_impl]
impl ConstFoldInterface for InsertValueOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(aggregate), Some(value)] = ops else {
            assert!(ops.len() == 2);
            return vec![None];
        };
        let Some(aggregate) = aggregate.downcast_ref::<AggregateAttr>() else {
            return vec![None];
        };
        let value = attr_cast::<dyn TypedAttrInterface>(&**value)
            .expect("invalid operand type: typecheck before optimizing");
        vec![
            insert_into_aggregate_attr(aggregate, value, &self.indices(ctx))
                .map(|result| Box::new(result) as AttrObj),
        ]
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

/// For an integer type, return its signless integer type and its width.
fn cast_dest_to_signless(
    ctx: &Context,
    res_ty: TypeHandle,
) -> Option<(TypedHandle<IntegerType>, NonZero<usize>)> {
    let dest_width = res_ty.deref(ctx).downcast_ref::<IntegerType>()?.width();
    Some((
        IntegerType::get(ctx, dest_width, Signedness::Signless),
        NonZero::new(dest_width as usize).expect("result has zero bitwidth"),
    ))
}

/// Fold a scalar integer cast with `resize`.
fn check_fold_int_cast(
    ctx: &Context,
    operand_attrs: &[Option<AttrObj>],
    res_ty: TypeHandle,
    resize: impl Fn(&APInt, NonZero<usize>) -> APInt,
) -> Vec<Option<AttrObj>> {
    let [Some(operand)] = operand_attrs else {
        assert!(operand_attrs.len() == 1);
        return vec![None];
    };
    let Some(operand) = operand.downcast_ref::<IntegerAttr>() else {
        return vec![None];
    };
    let Some((dest_ty, dest_width)) = cast_dest_to_signless(ctx, res_ty) else {
        return vec![None];
    };
    vec![Some(Box::new(IntegerAttr::new(
        dest_ty,
        resize(&operand.value(), dest_width),
    )) as AttrObj)]
}

#[op_interface_impl]
impl ConstFoldInterface for SExtOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_cast(ctx, ops, self.result_type(ctx), APInt::sext)
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
        // `zext nneg` asserts the operand is non-negative; if it isn't, the
        // result is poison, so we must not fold it to a concrete value.
        if self.nneg(ctx)
            && let [Some(operand)] = ops
            && let Some(operand) = operand.downcast_ref::<IntegerAttr>()
            && operand.value().is_negative()
        {
            return vec![None];
        }
        check_fold_int_cast(ctx, ops, self.result_type(ctx), APInt::zext)
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
impl ConstFoldInterface for TruncOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_cast(ctx, ops, self.result_type(ctx), APInt::trunc)
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

/// Fold a scalar float-to-integer cast, rounding toward zero.
fn check_fold_float_to_int(
    ctx: &Context,
    operand_attrs: &[Option<AttrObj>],
    result_ty: TypeHandle,
    signed: bool,
) -> Vec<Option<AttrObj>> {
    let [Some(operand)] = operand_attrs else {
        assert!(operand_attrs.len() == 1);
        return vec![None];
    };
    let Some(value) = attr_cast::<dyn FloatAttr>(&**operand) else {
        return vec![None];
    };

    let Some((dest_ty, dest_width)) = cast_dest_to_signless(ctx, result_ty) else {
        return vec![None];
    };
    let dest_width = dest_width.get();
    if dest_width > 128 {
        return vec![None];
    }

    let mut is_exact = false;
    let converted = if signed {
        // Avoid rustc_apfloat's unsupported zero-bit conversion for signed i1.
        let conversion_width = dest_width.max(2);
        let converted = value.to_i128_r(conversion_width, Round::TowardZero, &mut is_exact);
        if converted.status.contains(Status::INVALID_OP)
            || (dest_width == 1 && converted.value != -1 && converted.value != 0)
        {
            return vec![None];
        }
        APInt::from_i128(converted.value, bw(dest_width))
    } else {
        let converted = value.to_u128_r(dest_width, Round::TowardZero, &mut is_exact);
        if converted.status.contains(Status::INVALID_OP) {
            return vec![None];
        }
        APInt::from_u128(converted.value, bw(dest_width))
    };

    vec![Some(
        Box::new(IntegerAttr::new(dest_ty, converted)) as AttrObj
    )]
}

#[op_interface_impl]
impl ConstFoldInterface for FPToSIOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_to_int(ctx, ops, self.result_type(ctx), true)
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
impl ConstFoldInterface for FPToUIOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_to_int(ctx, ops, self.result_type(ctx), false)
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

/// Return zero value of a builtin float type `ty`.
fn zero_float_attr(ctx: &Context, ty: TypeHandle) -> Option<Box<dyn FloatAttr>> {
    let ty = ty.deref(ctx);
    if ty.is::<FP16Type>() {
        Some(Box::new(FPHalfAttr(Half::ZERO)))
    } else if ty.is::<FP32Type>() {
        Some(Box::new(FPSingleAttr(Single::ZERO)))
    } else if ty.is::<FP64Type>() {
        Some(Box::new(FPDoubleAttr(Double::ZERO)))
    } else {
        None
    }
}

/// Fold a scalar integer-to-float cast.
fn check_fold_int_to_float(
    ctx: &Context,
    operand_attrs: &[Option<AttrObj>],
    result_ty: TypeHandle,
    signed: bool,
    nneg: bool,
) -> Vec<Option<AttrObj>> {
    let [Some(operand)] = operand_attrs else {
        assert!(operand_attrs.len() == 1);
        return vec![None];
    };
    let Some(value) = operand.downcast_ref::<IntegerAttr>() else {
        return vec![None];
    };
    let value = value.value();

    // A violated `nneg` produces poison.
    if value.bw() > 128 || (nneg && value.is_negative()) {
        return vec![None];
    }

    let Some(result) = zero_float_attr(ctx, result_ty) else {
        return vec![None];
    };
    let result = if signed {
        result.build_from_i128(value.to_i128())
    } else {
        result.build_from_u128(value.to_u128())
    };
    vec![Some(result.value as AttrObj)]
}

#[op_interface_impl]
impl ConstFoldInterface for SIToFPOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_to_float(ctx, ops, self.result_type(ctx), true, false)
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
impl ConstFoldInterface for UIToFPOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_int_to_float(ctx, ops, self.result_type(ctx), false, self.nneg(ctx))
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

/// Return whether any `value` violates the fast-math flags.
fn fast_math_forbids_fold(flags: FastmathFlagsAttr, values: &[&dyn FloatAttr]) -> bool {
    let flags = flags.0;
    (flags.contains(FastmathFlags::NNAN) && values.iter().any(|v| v.is_nan()))
        || (flags.contains(FastmathFlags::NINF) && values.iter().any(|v| v.is_infinite()))
}

/// Convert a scalar float attribute to `result_ty`.
fn convert_float_attr(ctx: &Context, operand: &AttrObj, result_ty: TypeHandle) -> Option<AttrObj> {
    /// Convert `value` to `result_ty`, rounding to nearest-even.
    fn convert<S>(ctx: &Context, value: S, result_ty: TypeHandle) -> Option<AttrObj>
    where
        S: FloatConvert<Half> + FloatConvert<Single> + FloatConvert<Double>,
    {
        let result_ty = result_ty.deref(ctx);
        if result_ty.is::<FP16Type>() {
            Some(Box::new(FPHalfAttr(value.convert(&mut false).value)) as AttrObj)
        } else if result_ty.is::<FP32Type>() {
            Some(Box::new(FPSingleAttr(value.convert(&mut false).value)) as AttrObj)
        } else if result_ty.is::<FP64Type>() {
            Some(Box::new(FPDoubleAttr(value.convert(&mut false).value)) as AttrObj)
        } else {
            None
        }
    }

    if let Some(operand) = operand.downcast_ref::<FPHalfAttr>() {
        convert(ctx, operand.0, result_ty)
    } else if let Some(operand) = operand.downcast_ref::<FPSingleAttr>() {
        convert(ctx, operand.0, result_ty)
    } else if let Some(operand) = operand.downcast_ref::<FPDoubleAttr>() {
        convert(ctx, operand.0, result_ty)
    } else {
        None
    }
}

/// Fold a scalar float cast.
fn check_fold_float_cast(
    ctx: &Context,
    operand_attrs: &[Option<AttrObj>],
    result_ty: TypeHandle,
    flags: FastmathFlagsAttr,
) -> Vec<Option<AttrObj>> {
    let [Some(operand)] = operand_attrs else {
        assert!(operand_attrs.len() == 1);
        return vec![None];
    };
    let Some(source) = attr_cast::<dyn FloatAttr>(&**operand) else {
        return vec![None];
    };
    let Some(result) = convert_float_attr(ctx, operand, result_ty) else {
        return vec![None];
    };
    let result_float = attr_cast::<dyn FloatAttr>(&*result)
        .expect("floating-point conversion must produce a floating-point attribute");

    if fast_math_forbids_fold(flags, &[source, result_float]) {
        return vec![None];
    }
    vec![Some(result)]
}

#[op_interface_impl]
impl ConstFoldInterface for FPExtOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_cast(ctx, ops, self.result_type(ctx), self.fast_math_flags(ctx))
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
impl ConstFoldInterface for FPTruncOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_cast(ctx, ops, self.result_type(ctx), self.fast_math_flags(ctx))
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
impl ConstFoldInterface for FNegOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [Some(operand)] = ops else {
            assert!(ops.len() == 1);
            return vec![None];
        };
        let Some(float_val) = attr_cast::<dyn FloatAttr>(&**operand) else {
            return vec![None];
        };
        let negated = float_val.neg();
        // Negation cannot create or destroy NaN/Inf, so checking the operand
        // covers the result too.
        if fast_math_forbids_fold(self.fast_math_flags(ctx), &[float_val]) {
            return vec![None];
        }
        vec![Some(negated as AttrObj)]
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

/// An accessor to get the two constant operands of a binary float op.
fn get_float_bin_operands(
    operand_attrs: &[Option<AttrObj>],
) -> Option<(&dyn FloatAttr, &dyn FloatAttr)> {
    let [Some(lhs), Some(rhs)] = operand_attrs else {
        assert!(operand_attrs.len() == 2);
        return None;
    };
    let lhs = attr_cast::<dyn FloatAttr>(&**lhs)?;
    let rhs = attr_cast::<dyn FloatAttr>(&**rhs)?;
    Some((lhs, rhs))
}

/// Constant fold a binary floating-point operation.
fn check_fold_float_bin_op(
    operand_attrs: &[Option<AttrObj>],
    flags: FastmathFlagsAttr,
    combine: impl Fn(&dyn FloatAttr, &dyn FloatAttr) -> Box<dyn FloatAttr>,
) -> Vec<Option<AttrObj>> {
    let Some((lhs, rhs)) = get_float_bin_operands(operand_attrs) else {
        return vec![None];
    };
    let res = combine(lhs, rhs);
    if fast_math_forbids_fold(flags, &[lhs, rhs, &*res]) {
        return vec![None];
    }
    vec![Some(res as AttrObj)]
}

#[op_interface_impl]
impl ConstFoldInterface for FAddOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_bin_op(ops, self.fast_math_flags(ctx), |lhs, rhs| {
            lhs.add(rhs).value
        })
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
impl ConstFoldInterface for FSubOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_bin_op(ops, self.fast_math_flags(ctx), |lhs, rhs| {
            lhs.sub(rhs).value
        })
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
impl ConstFoldInterface for FMulOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_bin_op(ops, self.fast_math_flags(ctx), |lhs, rhs| {
            lhs.mul(rhs).value
        })
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
impl ConstFoldInterface for FDivOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_bin_op(ops, self.fast_math_flags(ctx), |lhs, rhs| {
            lhs.div(rhs).value
        })
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
impl ConstFoldInterface for FRemOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        check_fold_float_bin_op(ops, self.fast_math_flags(ctx), |lhs, rhs| {
            lhs.rem(rhs).value
        })
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

/// Evaluate `lhs <pred> rhs` for same-typed floats.
fn eval_fcmp(pred: &FCmpPredicateAttr, lhs: &dyn FloatAttr, rhs: &dyn FloatAttr) -> bool {
    let ord = lhs.partial_cmp(rhs);
    match pred {
        FCmpPredicateAttr::False => false,
        FCmpPredicateAttr::True => true,
        FCmpPredicateAttr::ORD => ord.is_some(),
        FCmpPredicateAttr::UNO => ord.is_none(),
        FCmpPredicateAttr::OEQ => ord == Some(Ordering::Equal),
        FCmpPredicateAttr::OGT => ord == Some(Ordering::Greater),
        FCmpPredicateAttr::OGE => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
        FCmpPredicateAttr::OLT => ord == Some(Ordering::Less),
        FCmpPredicateAttr::OLE => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
        FCmpPredicateAttr::ONE => matches!(ord, Some(Ordering::Less | Ordering::Greater)),
        FCmpPredicateAttr::UEQ => !matches!(ord, Some(Ordering::Less | Ordering::Greater)),
        FCmpPredicateAttr::UGT => !matches!(ord, Some(Ordering::Less | Ordering::Equal)),
        FCmpPredicateAttr::UGE => ord != Some(Ordering::Less),
        FCmpPredicateAttr::ULT => !matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
        FCmpPredicateAttr::ULE => ord != Some(Ordering::Greater),
        FCmpPredicateAttr::UNE => ord != Some(Ordering::Equal),
    }
}

#[op_interface_impl]
impl ConstFoldInterface for FCmpOp {
    fn check_fold(&self, ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let Some((lhs, rhs)) = get_float_bin_operands(ops) else {
            return vec![None];
        };
        // Only the operands can violate fast-math assumptions.
        if fast_math_forbids_fold(self.fast_math_flags(ctx), &[lhs, rhs]) {
            return vec![None];
        }
        let result = eval_fcmp(&self.predicate(ctx), lhs, rhs);
        vec![Some(bool_attr(ctx, result))]
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
impl ConstFoldInterface for SelectOp {
    fn check_fold(&self, _ctx: &Context, ops: &[Option<AttrObj>]) -> Vec<Option<AttrObj>> {
        let [cond, true_val, false_val] = ops else {
            assert!(ops.len() == 3);
            return vec![None];
        };
        match cond {
            Some(cond_attr) => {
                // Vector conditions require element-wise folding.
                let Some(cond_int) = cond_attr.downcast_ref::<IntegerAttr>() else {
                    return vec![None];
                };
                let chosen = if cond_int.value().is_zero() {
                    false_val
                } else {
                    true_val
                };
                vec![chosen.clone()]
            }
            // Equal constants make the condition irrelevant.
            None => match (true_val, false_val) {
                (Some(t), Some(f)) if t == f => vec![Some(t.clone())],
                _ => vec![None],
            },
        }
    }
    fn fold_in_place(
        &self,
        ctx: &mut Context,
        ops: &[Option<AttrObj>],
        rw: &mut dyn Rewriter,
    ) -> IRStatus {
        // A constant condition forwards one operand directly.
        if let Some(cond_attr) = &ops[0]
            && let Some(cond_int) = cond_attr.downcast_ref::<IntegerAttr>()
        {
            let (chosen, result) = {
                let op = self.get_operation().deref(ctx);
                let chosen_idx = if cond_int.value().is_zero() { 2 } else { 1 };
                (op.get_operand(chosen_idx), op.get_result(0))
            };
            if !result.is_used(ctx) {
                return IRStatus::Unchanged;
            }
            rw.replace_value_uses_with(ctx, result, chosen);
            return IRStatus::Changed;
        }
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
        _operand_attrs: &[Option<AttrObj>],
        _rw: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
}

impl CondBrOp {
    /// Returns the possible successor indices based on the condition operand.
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
        operand_attrs: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        let possible_successor_indices = self.possible_successor_indices(ctx, operand_attrs);
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
            .get_attr_llvm_switch_case_values(ctx)
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
        operand_attrs: &[Option<AttrObj>],
        rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        let Some(cond_attr) = operand_attrs.first().unwrap().as_ref() else {
            return IRStatus::Unchanged;
        };
        let cond_int = cond_attr
            .downcast_ref::<IntegerAttr>()
            .expect("Switch condition operand must be an IntegerAttr")
            .value();
        let successor_ind = {
            let case_values = self
                .get_attr_llvm_switch_case_values(ctx)
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
