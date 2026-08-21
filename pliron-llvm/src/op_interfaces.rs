// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! [Op] Interfaces defined in the LLVM dialect.

use alloc::{
    string::{String, ToString},
    vec,
};

use pliron::{
    builtin::{
        attributes::{BoolAttr, UnitAttr},
        op_interfaces::{
            NOpdsInterface, OneOpdInterface, ResultNOfType, SymbolOpInterface,
            verify_get_operand_n, verify_get_result_n,
        },
        type_interfaces::FloatTypeInterface,
    },
    derive::op_interface,
    dict_key,
    printable::Printable,
    r#type::{Type, TypeInterfaceMarker, TypedHandle, type_impls},
    utils::const_bound_n::I,
};
use thiserror::Error;

use pliron::{
    builtin::{
        op_interfaces::{OneResultInterface, SameOperandsAndResultType},
        types::{IntegerType, Signedness},
    },
    context::Context,
    location::{Located, Location},
    op::{Op, op_cast},
    operation::Operation,
    result::Result,
    r#type::{TypeHandle, Typed},
    value::Value,
    verify_err,
};

use crate::{
    attributes::{AlignmentAttr, FastmathFlagsAttr},
    types::{VectorType, VectorTypeKind},
};

use super::{attributes::IntegerOverflowFlagsAttr, types::PointerType};

/// Binary arithmetic [Op].
#[op_interface]
pub trait BinArithOp: SameOperandsAndResultType + OneResultInterface + NOpdsInterface<2> {
    /// Create a new binary arithmetic operation given the operands.
    fn new(ctx: &mut Context, lhs: Value, rhs: Value) -> Self
    where
        Self: Sized,
    {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![lhs.get_type(ctx)],
            vec![lhs, rhs],
            vec![],
            0,
        );
        Self::from_operation(op)
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }

    /// Get the left-hand side operand of this binary arithmetic [Op].
    fn lhs(&self, ctx: &Context) -> Value
    where
        Self: Sized,
    {
        self.get_operand_i(ctx, I::<0>.into())
    }

    /// Get the right-hand side operand of this binary arithmetic [Op].
    fn rhs(&self, ctx: &Context) -> Value
    where
        Self: Sized,
    {
        self.get_operand_i(ctx, I::<1>.into())
    }
}

#[derive(Error, Debug)]
#[error("Integer binary arithmetic Op can only have signless integer result/operand type")]
pub struct IntBinArithOpErr;

/// Integer binary arithmetic [Op]
#[op_interface]
pub trait IntBinArithOp: BinArithOp + ScalarOrVectorOpd<IntegerType, 0> {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let int_ty = op_cast::<dyn ScalarOrVectorOpd<IntegerType, 0>>(op)
            .expect("Op must impl ScalarOrVectorOpd<IntegerType, 0>")
            .scalar_or_vector_elem_ty(ctx);
        let int_ty = int_ty.deref(ctx);
        if int_ty.signedness() != Signedness::Signless {
            return verify_err!(op.loc(ctx), IntBinArithOpErr);
        }

        Ok(())
    }
}

dict_key!(
    /// Attribute key for integer overflow flags.
    ATTR_KEY_INTEGER_OVERFLOW_FLAGS,
    "llvm_integer_overflow_flags"
);

#[derive(Error, Debug)]
#[error("IntegerOverflowFlag missing on Op")]
pub struct IntBinArithOpWithOverflowFlagErr;

/// Integer binary arithmetic [Op] with [IntegerOverflowFlagsAttr]
#[op_interface]
pub trait IntBinArithOpWithOverflowFlag: IntBinArithOp {
    /// Create a new integer binary op with overflow flags set.
    fn new_with_overflow_flag(
        ctx: &mut Context,
        lhs: Value,
        rhs: Value,
        flag: IntegerOverflowFlagsAttr,
    ) -> Self
    where
        Self: Sized,
    {
        let op = Self::new(ctx, lhs, rhs);
        op.set_integer_overflow_flag(ctx, flag);
        op
    }

    /// Get the integer overflow flag on this [Op].
    fn integer_overflow_flag(&self, ctx: &Context) -> IntegerOverflowFlagsAttr
    where
        Self: Sized,
    {
        self.get_operation()
            .deref(ctx)
            .attributes
            .get::<IntegerOverflowFlagsAttr>(&ATTR_KEY_INTEGER_OVERFLOW_FLAGS)
            .expect("Integer overflow flag missing or is of incorrect type")
            .clone()
    }

    /// Set the integer overflow flag for this [Op].
    fn set_integer_overflow_flag(&self, ctx: &Context, flag: IntegerOverflowFlagsAttr)
    where
        Self: Sized,
    {
        self.get_operation()
            .deref_mut(ctx)
            .attributes
            .set(ATTR_KEY_INTEGER_OVERFLOW_FLAGS.clone(), flag);
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        if op
            .attributes
            .get::<IntegerOverflowFlagsAttr>(&ATTR_KEY_INTEGER_OVERFLOW_FLAGS)
            .is_none()
        {
            return verify_err!(op.loc(), IntBinArithOpWithOverflowFlagErr);
        }

        Ok(())
    }
}

/// Floating point binary arithmetic [Op]
#[op_interface]
pub trait FloatBinArithOp: BinArithOp + ScalarOrVectorOpdImpls<dyn FloatTypeInterface, 0> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

dict_key!(
    /// Attribute key for fastmath flags.
    ATTR_KEY_FAST_MATH_FLAGS,
    "llvm_fast_math_flags"
);

#[derive(Error, Debug)]
#[error("Fastmath flag missing on Op")]
pub struct FastMathFlagMissingErr;

/// Ops that have fast math flags.
#[op_interface]
pub trait FastMathFlags {
    /// Get the fast math flags on this [Op].
    fn fast_math_flags(&self, ctx: &Context) -> FastmathFlagsAttr
    where
        Self: Sized,
    {
        *self
            .get_operation()
            .deref(ctx)
            .attributes
            .get::<FastmathFlagsAttr>(&ATTR_KEY_FAST_MATH_FLAGS)
            .expect("Fast math flags missing or is of incorrect type")
    }

    /// Set the fast math flags for this [Op].
    fn set_fast_math_flags(&self, ctx: &Context, flag: FastmathFlagsAttr)
    where
        Self: Sized,
    {
        self.get_operation()
            .deref_mut(ctx)
            .attributes
            .set(ATTR_KEY_FAST_MATH_FLAGS.clone(), flag);
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        if op
            .attributes
            .get::<FastmathFlagsAttr>(&ATTR_KEY_FAST_MATH_FLAGS)
            .is_none()
        {
            return verify_err!(op.loc(), FastmathFlagMissingErr);
        }

        Ok(())
    }
}

/// Floating point binary arithmetic [Op] with [FastmathFlagsAttr]
#[op_interface]
pub trait FloatBinArithOpWithFastMathFlags: FloatBinArithOp + FastMathFlags {
    /// Create a new floating point binary op with fast math flags set.
    fn new_with_fast_math_flags(
        ctx: &mut Context,
        lhs: Value,
        rhs: Value,
        flag: FastmathFlagsAttr,
    ) -> Self
    where
        Self: Sized,
    {
        let op = Self::new(ctx, lhs, rhs);
        op.set_fast_math_flags(ctx, flag);
        op
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("Fastmath flag missing on Op")]
pub struct FastmathFlagMissingErr;

dict_key!(
    /// Attribute key for nneg flag.
    ATTR_KEY_NNEG_FLAG,
    "llvm_nneg_flag"
);

#[op_interface]
pub trait NNegFlag {
    // Get the current NNEG flag value.
    fn nneg(&self, ctx: &Context) -> bool {
        self.get_operation()
            .deref(ctx)
            .attributes
            .get::<BoolAttr>(&ATTR_KEY_NNEG_FLAG)
            .expect("NNEG flag missing or is of incorrect type")
            .clone()
            .into()
    }
    // Set the current NNEG flag value.
    fn set_nneg(&self, ctx: &Context, flag: bool) {
        self.get_operation()
            .deref_mut(ctx)
            .attributes
            .set(ATTR_KEY_NNEG_FLAG.clone(), BoolAttr::new(flag));
    }
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let op = op.get_operation().deref(ctx);
        if op.attributes.get::<BoolAttr>(&ATTR_KEY_NNEG_FLAG).is_none() {
            return verify_err!(op.loc(), NNegFlagMissingErr);
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("NNEG flag missing on Op")]
pub struct NNegFlagMissingErr;

#[derive(Error, Debug)]
#[error("Result must be a pointer type, but is not")]
pub struct PointerTypeResultVerifyErr;

/// An [Op] with a single result whose type is [PointerType]
#[op_interface]
pub trait PointerTypeResult: OneResultInterface + ResultNOfType<0, PointerType> {
    /// Get the pointee type of the result pointer.
    fn result_pointee_type(&self, ctx: &Context) -> TypeHandle;

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        if !op_cast::<dyn OneResultInterface>(op)
            .expect("An Op here must impl OneResultInterface")
            .result_type(ctx)
            .deref(ctx)
            .is::<PointerType>()
        {
            return verify_err!(op.loc(ctx), PointerTypeResultVerifyErr);
        }

        Ok(())
    }
}

/// A Cast [Op] has one argument and one result.
#[op_interface]
pub trait CastOpInterface: OneResultInterface + OneOpdInterface {
    /// Create a new cast operation given the operand.
    fn new(ctx: &mut Context, operand: Value, res_type: TypeHandle) -> Self
    where
        Self: Sized,
    {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_type],
            vec![operand],
            vec![],
            0,
        );
        Self::from_operation(op)
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// A Cast [Op] with NNEG flag.
#[op_interface]
pub trait CastOpWithNNegInterface: CastOpInterface + NNegFlag {
    /// Create a new cast operation with nneg flag
    fn new_with_nneg(ctx: &mut Context, operand: Value, res_type: TypeHandle, nneg: bool) -> Self
    where
        Self: Sized,
    {
        let op = Self::new(ctx, operand, res_type);
        op.set_nneg(ctx, nneg);
        op
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Is a global value (variable or function) declaration.
#[op_interface]
pub trait IsDeclaration {
    /// Check if this global value (variable or function) is a declaration.
    fn is_declaration(&self, ctx: &Context) -> bool
    where
        Self: Sized;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

dict_key!(
    /// Attribute key for LLVM symbol name.
    ATTR_KEY_LLVM_SYMBOL_NAME,
    "llvm_symbol_name"
);

/// Since LLVM symbols can have characters that are illegal in pliron,
/// this interface provides a way to get the original LLVM symbol name.
#[op_interface]
pub trait LlvmSymbolName: SymbolOpInterface {
    /// Get the original LLVM symbol name, if it's different from the pliron symbol name.
    fn llvm_symbol_name(&self, ctx: &Context) -> Option<String> {
        self.get_operation()
            .deref(ctx)
            .attributes
            .get::<pliron::builtin::attributes::StringAttr>(&ATTR_KEY_LLVM_SYMBOL_NAME)
            .map(|attr| attr.clone().into())
    }

    /// Set the original LLVM symbol name.
    fn set_llvm_symbol_name(&self, ctx: &Context, name: String) {
        self.get_operation().deref_mut(ctx).attributes.set(
            ATTR_KEY_LLVM_SYMBOL_NAME.clone(),
            pliron::builtin::attributes::StringAttr::new(name),
        );
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

dict_key!(
    /// Attribute key for alignment.
    ATTR_KEY_LLVM_ALIGNMENT,
    "llvm_alignment"
);

/// Ops that can have an alignment set.
#[op_interface]
pub trait AlignableOpInterface {
    /// Get the alignment of this [Op], if set.
    fn alignment(&self, ctx: &Context) -> Option<u32>
    where
        Self: Sized,
    {
        self.get_operation()
            .deref(ctx)
            .attributes
            .get::<AlignmentAttr>(&ATTR_KEY_LLVM_ALIGNMENT)
            .map(|attr| attr.0)
    }

    /// Set the alignment of this [Op].
    fn set_alignment(&self, ctx: &Context, alignment: u32)
    where
        Self: Sized,
    {
        self.get_operation()
            .deref_mut(ctx)
            .attributes
            .set(ATTR_KEY_LLVM_ALIGNMENT.clone(), AlignmentAttr(alignment));
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

dict_key!(
    /// Attribute key for the non-temporal hint.
    ATTR_KEY_LLVM_NON_TEMPORAL,
    "llvm_non_temporal"
);

/// Ops that can be marked non-temporal, i.e. LLVM's `!nontemporal` metadata.
///
/// The hint tells the backend that the accessed data is unlikely to be reused soon, so it may
/// bypass the cache. It is only a hint: backends that cannot honor it emit an ordinary access.
#[op_interface]
pub trait NonTemporalOpInterface {
    /// Whether this [Op] is marked non-temporal.
    fn is_non_temporal(&self, ctx: &Context) -> bool
    where
        Self: Sized,
    {
        self.get_operation()
            .deref(ctx)
            .attributes
            .get::<UnitAttr>(&ATTR_KEY_LLVM_NON_TEMPORAL)
            .is_some()
    }

    /// Mark this [Op] as non-temporal.
    fn set_non_temporal(&self, ctx: &Context)
    where
        Self: Sized,
    {
        self.get_operation()
            .deref_mut(ctx)
            .attributes
            .set(ATTR_KEY_LLVM_NON_TEMPORAL.clone(), UnitAttr::new());
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ScalarOrVectorErr {
    #[error("{0} is not {1} or a vector of it")]
    NotTOrVectorOfT(String, String),
    #[error("{0} or its vector element type does not implement interface")]
    TyOrElemNotImplsI(String),
}

/// Get the vector shape of `ty`, or `None` if it is not a [VectorType].
fn vector_shape_of(ty: TypeHandle, ctx: &Context) -> Option<(u32, VectorTypeKind)> {
    ty.deref(ctx)
        .downcast_ref::<VectorType>()
        .map(|vec_ty| (vec_ty.num_elements(), vec_ty.kind()))
}

/// Get `ty`, or its element type if it is a [VectorType], typed as `T`.
///
/// Only valid to call once `verify_t_or_vec_of_t::<T>` has passed for `ty`.
fn elem_ty_of<T: Type>(ty: TypeHandle, ctx: &Context) -> TypedHandle<T> {
    if let Ok(typed_handle) = TypedHandle::from_handle(ty, ctx) {
        return typed_handle;
    }
    let ty_ref = &*ty.deref(ctx);
    let elem_ty = ty_ref
        .downcast_ref::<VectorType>()
        .expect("verify() guarantees type is T or a vector of T")
        .elem_type();
    TypedHandle::from_handle(elem_ty, ctx).expect("verify() guarantees element type is T")
}

/// Verify that `ty` is `T`, or a [VectorType] of `T`.
fn verify_t_or_vec_of_t<T: Type>(loc: Location, ty: &dyn Type, ctx: &Context) -> Result<()> {
    if ty.is::<T>() {
        // ty is equal to `T`, we're good.
        return Ok(());
    }
    // See if `ty` is a vector of `T`.
    let Some(vec_ty) = ty.downcast_ref::<VectorType>() else {
        return verify_err!(
            loc,
            ScalarOrVectorErr::NotTOrVectorOfT(
                ty.get_type_id().disp(ctx).to_string(),
                T::get_type_id_static().disp(ctx).to_string()
            )
        );
    };
    let elem_ty = &*vec_ty.elem_type().deref(ctx);
    if !elem_ty.is::<T>() {
        return verify_err!(
            loc,
            ScalarOrVectorErr::NotTOrVectorOfT(
                ty.get_type_id().disp(ctx).to_string(),
                T::get_type_id_static().disp(ctx).to_string()
            )
        );
    }
    Ok(())
}

/// Get `ty`, or its element type if it is a [VectorType], whichever implements `I`.
///
/// Only valid to call once `verify_impls_i_or_vec_of_impls_i::<I>` has passed for `ty`.
fn elem_ty_of_impls<I: ?Sized + TypeInterfaceMarker + 'static>(
    ty: TypeHandle,
    ctx: &Context,
) -> TypeHandle {
    let ty_ref = &*ty.deref(ctx);
    if type_impls::<I>(ty_ref) {
        return ty;
    }
    ty_ref
        .downcast_ref::<VectorType>()
        .expect("verify() guarantees type impls I or is a vector whose elem impls I")
        .elem_type()
}

/// Verify that `ty` implements `I`, or is a [VectorType] whose element type implements `I`.
fn verify_impls_i_or_vec_of_impls_i<I: ?Sized + TypeInterfaceMarker + 'static>(
    loc: Location,
    ty: &dyn Type,
    ctx: &Context,
) -> Result<()> {
    if type_impls::<I>(ty) {
        // ty implements `I`, we're good.
        return Ok(());
    }
    // See if `ty` is a vector whose element type implements `I`.
    let Some(vec_ty) = ty.downcast_ref::<VectorType>() else {
        return verify_err!(
            loc,
            ScalarOrVectorErr::TyOrElemNotImplsI(ty.get_type_id().disp(ctx).to_string())
        );
    };
    let elem_ty = &*vec_ty.elem_type().deref(ctx);
    if !type_impls::<I>(elem_ty) {
        return verify_err!(
            loc,
            ScalarOrVectorErr::TyOrElemNotImplsI(ty.get_type_id().disp(ctx).to_string())
        );
    }
    Ok(())
}

/// An operand whose type is either `T` or [`VectorType<T>`](VectorType).
#[op_interface]
pub trait ScalarOrVectorOpd<T: Type, const N: usize> {
    /// Get the type of operand N, or its element type if its [VectorType].
    fn scalar_or_vector_elem_ty(&self, ctx: &Context) -> TypedHandle<T> {
        let op = &*self.get_operation().deref(ctx);
        elem_ty_of(op.get_operand(N).get_type(ctx), ctx)
    }

    /// Get the vector shape of operand N, or `None` if it is not a [VectorType].
    fn vector_shape(&self, ctx: &Context) -> Option<(u32, VectorTypeKind)> {
        let op = &*self.get_operation().deref(ctx);
        vector_shape_of(op.get_operand(N).get_type(ctx), ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opd = verify_get_operand_n::<N>(op.get_operation(), ctx)?;
        verify_t_or_vec_of_t::<T>(op.loc(ctx), &*opd.get_type(ctx).deref(ctx), ctx)
    }
}

/// An operand whose type either impls `I` or is [`VectorType<T: I>`](VectorType).
#[op_interface]
pub trait ScalarOrVectorOpdImpls<I: ?Sized + TypeInterfaceMarker + 'static, const N: usize> {
    /// Get the type of operand N, or its element type if its [VectorType].
    fn scalar_or_vector_elem_ty(&self, ctx: &Context) -> TypeHandle {
        let op = &*self.get_operation().deref(ctx);
        elem_ty_of_impls::<I>(op.get_operand(N).get_type(ctx), ctx)
    }

    /// Get the vector shape of operand N, or `None` if it is not a [VectorType].
    fn vector_shape(&self, ctx: &Context) -> Option<(u32, VectorTypeKind)> {
        let op = &*self.get_operation().deref(ctx);
        vector_shape_of(op.get_operand(N).get_type(ctx), ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let opd = verify_get_operand_n::<N>(op.get_operation(), ctx)?;
        verify_impls_i_or_vec_of_impls_i::<I>(op.loc(ctx), &*opd.get_type(ctx).deref(ctx), ctx)
    }
}

/// A result whose type is either `T` or [`VectorType<T>`](VectorType).
#[op_interface]
pub trait ScalarOrVectorRes<T: Type, const N: usize> {
    /// Get the type of result N, or its element type if its [VectorType].
    fn scalar_or_vector_elem_ty(&self, ctx: &Context) -> TypedHandle<T> {
        let op = &*self.get_operation().deref(ctx);
        elem_ty_of(op.get_result(N).get_type(ctx), ctx)
    }

    /// Get the vector shape of result N, or `None` if it is not a [VectorType].
    fn vector_shape(&self, ctx: &Context) -> Option<(u32, VectorTypeKind)> {
        let op = &*self.get_operation().deref(ctx);
        vector_shape_of(op.get_result(N).get_type(ctx), ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let res = verify_get_result_n::<N>(op.get_operation(), ctx)?;
        verify_t_or_vec_of_t::<T>(op.loc(ctx), &*res.get_type(ctx).deref(ctx), ctx)
    }
}

/// A result whose type either impls `I` or is [`VectorType<T: I>`](VectorType).
#[op_interface]
pub trait ScalarOrVectorResImpls<I: ?Sized + TypeInterfaceMarker + 'static, const N: usize> {
    /// Get the type of result N, or its element type if its [VectorType].
    fn scalar_or_vector_elem_ty(&self, ctx: &Context) -> TypeHandle {
        let op = &*self.get_operation().deref(ctx);
        elem_ty_of_impls::<I>(op.get_result(N).get_type(ctx), ctx)
    }

    /// Get the vector shape of result N, or `None` if it is not a [VectorType].
    fn vector_shape(&self, ctx: &Context) -> Option<(u32, VectorTypeKind)> {
        let op = &*self.get_operation().deref(ctx);
        vector_shape_of(op.get_result(N).get_type(ctx), ctx)
    }

    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let res = verify_get_result_n::<N>(op.get_operation(), ctx)?;
        verify_impls_i_or_vec_of_impls_i::<I>(op.loc(ctx), &*res.get_type(ctx).deref(ctx), ctx)
    }
}
