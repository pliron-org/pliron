//! [Op] Interfaces defined in the LLVM dialect.

use pliron::{
    builtin::{
        attributes::BoolAttr,
        op_interfaces::{
            NOpdsInterface, NResultsInterface, OneOpdInterface, ResultNOfType, SymbolOpInterface,
        },
        type_interfaces::FloatTypeInterface,
    },
    derive::op_interface,
    dict_key,
    r#type::type_cast,
};
use thiserror::Error;

use pliron::{
    builtin::{
        op_interfaces::{OneResultInterface, SameOperandsAndResultType},
        types::{IntegerType, Signedness},
    },
    context::Context,
    location::Located,
    op::{Op, op_cast},
    operation::Operation,
    result::Result,
    r#type::{TypeHandle, Typed},
    value::Value,
    verify_err,
};

use crate::{
    attributes::{AlignmentAttr, FastmathFlagsAttr},
    types::VectorType,
};

use super::{attributes::IntegerOverflowFlagsAttr, types::PointerType};

/// Binary arithmetic [Op].
#[op_interface]
pub trait BinArithOp:
    SameOperandsAndResultType + OneResultInterface + NOpdsInterface<2> + NResultsInterface<1>
{
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
        self.get_operand_i(ctx, 0)
    }

    /// Get the right-hand side operand of this binary arithmetic [Op].
    fn rhs(&self, ctx: &Context) -> Value
    where
        Self: Sized,
    {
        self.get_operand_i(ctx, 1)
    }
}

#[derive(Error, Debug)]
#[error("Integer binary arithmetic Op can only have signless integer result/operand type")]
pub struct IntBinArithOpErr;

/// Integer binary arithmetic [Op]
#[op_interface]
pub trait IntBinArithOp: BinArithOp {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let mut ty = op_cast::<dyn SameOperandsAndResultType>(op)
            .expect("Op must impl SameOperandsAndResultType")
            .get_type(ctx);

        if let Some(vec_ty) = ty.deref(ctx).downcast_ref::<VectorType>() {
            ty = vec_ty.elem_type();
        }

        let ty = ty.deref(ctx);
        let Some(int_ty) = ty.downcast_ref::<IntegerType>() else {
            return verify_err!(op.loc(ctx), IntBinArithOpErr);
        };

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

#[derive(Error, Debug)]
#[error("Floating point arithmetic Op can only have signless floating point result/operand type")]
pub struct FloatBinArithOpErr;

/// Floating point binary arithmetic [Op]
#[op_interface]
pub trait FloatBinArithOp: BinArithOp {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        let mut ty = op_cast::<dyn SameOperandsAndResultType>(op)
            .expect("Op must impl SameOperandsAndResultType")
            .get_type(ctx);

        if let Some(vec_ty) = ty.deref(ctx).downcast_ref::<VectorType>() {
            ty = vec_ty.elem_type();
        }

        let ty = ty.deref(ctx);
        if type_cast::<dyn FloatTypeInterface>(&*ty).is_none() {
            return verify_err!(op.loc(ctx), FloatBinArithOpErr);
        }
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
pub trait CastOpInterface:
    OneResultInterface + OneOpdInterface + NResultsInterface<1> + NOpdsInterface<1>
{
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
pub trait CastOpWithNNegInterface:
    CastOpInterface + NNegFlag + NResultsInterface<1> + NOpdsInterface<1>
{
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
