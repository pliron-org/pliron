// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Dialect conversion from builtin to LLVM dialect

use alloc::{vec, vec::Vec};

use pliron::{
    builtin::{
        op_interfaces::{CallOpInterface, OneRegionInterface, SymbolOpInterface},
        ops::{
            CallOp as BuiltinCallOp, ConstantOp as BuiltinConstantOp,
            FuncConstantOp as BuiltinFuncConstantOp, FuncOp as BuiltinFuncOp,
            IndirectCallOp as BuiltinIndirectCallOp, ModuleOp, ReturnOp as BuiltinReturnOp,
        },
        type_interfaces::FunctionTypeInterface,
        types::{FunctionType as BuiltinFunctionType, UnitType},
    },
    common_traits::Verify,
    context::{Context, Ptr},
    derive::{op_interface_impl, type_interface_impl},
    input_err, input_err_noloc, input_error_noloc,
    irbuild::{
        dialect_conversion::{self, DialectConversion, DialectConversionRewriter, OperandsInfo},
        inserter::Inserter,
        rewriter::Rewriter,
    },
    op::{Op, op_impls},
    operation::Operation,
    pass::{GuardedPass, OpGuard, OpPass, Pass, PassResult},
    region::Region,
    result::{Error, ErrorKind, Result},
    r#type::{TypeHandle, Typed, TypedHandle, type_cast},
};

use crate::{
    ToLLVMDialect, ToLLVMType,
    ops::{
        AddressOfOp, CallOp as LLVMCallOp, ConstantOp as LLVMConstantOp, FuncOp as LLVMFuncOp,
        ReturnOp as LLVMReturnOp,
    },
    types::{FuncType as LLVMFuncType, PointerType, VoidType},
};

#[derive(thiserror::Error, Debug)]
pub enum BuiltinToLLVMConversionError {
    #[error("Invalid function type, cannot be converted to LLVM function type")]
    InvalidFunctionType,
    #[error("Cannot return more than one value from an LLVM function")]
    MultipleReturnValues,
    #[error("Function type does not implement ToLLVMType interface")]
    FuncTypeNotToLLVMType,
}

#[type_interface_impl]
impl ToLLVMType for BuiltinFunctionType {
    fn convert(&self, ctx: &Context) -> Result<TypeHandle> {
        let arg_types = self.arg_types();
        let res_types = self.res_types();

        let convert_type_to_llvm = |ty: TypeHandle| {
            if ty.deref(ctx).is::<BuiltinFunctionType>() {
                // Function values become function pointers.
                return Ok(PointerType::get(ctx, 0).into());
            }
            type_cast::<dyn ToLLVMType>(&*ty.deref(ctx))
                .map(|ty| ty.convert(ctx))
                .unwrap_or(Ok(ty))
        };

        let arg_types = arg_types
            .into_iter()
            .map(convert_type_to_llvm)
            .collect::<Result<Vec<_>>>()?;
        let res_types = res_types
            .into_iter()
            .map(convert_type_to_llvm)
            .collect::<Result<Vec<_>>>()?;

        let result_type = match res_types.as_slice() {
            [] => VoidType::get(ctx).to_handle(),
            [res_ty] => *res_ty,
            _ => return input_err_noloc!(BuiltinToLLVMConversionError::InvalidFunctionType),
        };

        let llvm_func_type = LLVMFuncType::get(ctx, result_type, arg_types, false);
        Ok(llvm_func_type.into())
    }
}

#[type_interface_impl]
impl ToLLVMType for UnitType {
    fn convert(&self, ctx: &Context) -> Result<TypeHandle> {
        Ok(VoidType::get(ctx).to_handle())
    }
}

/// Convert builtin.constant to llvm.constant
#[op_interface_impl]
impl ToLLVMDialect for BuiltinConstantOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let const_value = pliron::dyn_clone::clone_box(&*self.get_value(ctx));

        // Create the LLVM constant operation with the same value
        let llvm_const = LLVMConstantOp::new(ctx, const_value);

        if let Err(e @ Error { .. }) = llvm_const.verify(ctx) {
            return Err(Error {
                kind: ErrorKind::InvalidInput,
                // We reset the error origin to be from here
                backtrace: pliron::std_deps::backtrace::Backtrace::capture(),
                ..e
            });
        }

        // Insert the new operation before the current one
        rewriter.insert_operation(ctx, llvm_const.get_operation());

        // Replace the old operation with the new one
        let old_op = self.get_operation();
        rewriter.replace_operation(ctx, old_op, llvm_const.get_operation());

        Ok(())
    }
}

/// Convert builtin.func to llvm.func
#[op_interface_impl]
impl ToLLVMDialect for BuiltinFuncOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        // Get the function name
        let func_name = self.get_symbol_name(ctx);

        let llvm_func_type = convert_func_type(ctx, self.get_type(ctx).to_handle())?;

        // Create the LLVM function operation
        let llvm_func = LLVMFuncOp::new(ctx, func_name, llvm_func_type);

        // Move the region from the builtin.func to the llvm.func
        Region::move_to_op(self.get_region(ctx), llvm_func.get_operation(), ctx);

        // Get the old operation
        let old_op = self.get_operation();

        // Insert the new operation before the current one
        rewriter.insert_operation(ctx, llvm_func.get_operation());

        // Replace the old operation with the new one
        rewriter.replace_operation(ctx, old_op, llvm_func.get_operation());

        Ok(())
    }
}

/// Convert [`ty: FunctionType`](BuiltinFunctionType) to [LLVMFuncType].
fn convert_func_type(ctx: &Context, ty: TypeHandle) -> Result<TypedHandle<LLVMFuncType>> {
    let llvm_ty = type_cast::<dyn ToLLVMType>(&*ty.deref(ctx))
        .ok_or_else(|| input_error_noloc!(BuiltinToLLVMConversionError::FuncTypeNotToLLVMType))?
        .convert(ctx)?;
    TypedHandle::from_handle(llvm_ty, ctx)
}

/// Convert builtin.return to llvm.return
#[op_interface_impl]
impl ToLLVMDialect for BuiltinReturnOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let operands = self
            .get_operation()
            .deref(ctx)
            .operands()
            .collect::<Vec<_>>();
        let is_void = |ctx: &Context, value: &pliron::value::Value| {
            let ty = value.get_type(ctx);
            let ty = ty.deref(ctx);
            ty.is::<UnitType>() || ty.is::<VoidType>()
        };
        let retval = match operands.as_slice() {
            [] => None,
            // `llvm.return` must not have an argument when its function's return type is `void`.
            [opd] if is_void(ctx, opd) => None,
            [opd] => Some(*opd),
            _ => {
                return input_err!(
                    self.loc(ctx),
                    BuiltinToLLVMConversionError::MultipleReturnValues
                );
            }
        };

        let llvm_return = LLVMReturnOp::new(ctx, retval);
        rewriter.insert_operation(ctx, llvm_return.get_operation());
        rewriter.replace_operation(ctx, self.get_operation(), llvm_return.get_operation());

        Ok(())
    }
}

/// Convert a builtin call-like op to llvm.call
fn rewrite_call(
    ctx: &mut Context,
    rewriter: &mut DialectConversionRewriter,
    call: &dyn CallOpInterface,
) -> Result<()> {
    let callee_ty = convert_func_type(ctx, call.callee_type(ctx).to_handle())?;
    let llvm_call = LLVMCallOp::new(ctx, call.callee(ctx), callee_ty, call.args(ctx));
    rewriter.insert_operation(ctx, llvm_call.get_operation());

    let old_op = call.get_operation();
    if old_op.deref(ctx).get_num_results() == 0 {
        // `llvm_call` has a `void` result, but it can't replace any of `old_op`'s results.
        rewriter.replace_operation_with_values(ctx, old_op, vec![]);
    } else {
        rewriter.replace_operation(ctx, old_op, llvm_call.get_operation());
    }

    Ok(())
}

/// Convert builtin.call to llvm.call
#[op_interface_impl]
impl ToLLVMDialect for BuiltinCallOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        rewrite_call(ctx, rewriter, self)
    }
}

/// Convert builtin.indirect_call to llvm.call
#[op_interface_impl]
impl ToLLVMDialect for BuiltinIndirectCallOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        rewrite_call(ctx, rewriter, self)
    }
}

/// Convert builtin.func_constant to llvm.addressof
#[op_interface_impl]
impl ToLLVMDialect for BuiltinFuncConstantOp {
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let address_of = AddressOfOp::new(ctx, self.get_callee_name(ctx), 0);
        rewriter.insert_operation(ctx, address_of.get_operation());
        rewriter.replace_operation(ctx, self.get_operation(), address_of.get_operation());

        Ok(())
    }
}

/// Dialect conversion from builtin ops to LLVM ops.
#[derive(Default)]
pub struct BuiltinToLLVMConversion;

impl DialectConversion for BuiltinToLLVMConversion {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let op_ref = op_dyn.op_ref();

        // Check if this operation implements ToLLVMDialect
        op_impls::<dyn ToLLVMDialect>(op_ref)
    }

    fn can_convert_type(&self, ctx: &Context, ty: TypeHandle) -> bool {
        // `convert_type` converts a function value to a function pointer.
        ty.deref(ctx).is::<BuiltinFunctionType>()
    }

    fn convert_type(&mut self, ctx: &mut Context, ty: TypeHandle) -> Result<TypeHandle> {
        if ty.deref(ctx).is::<BuiltinFunctionType>() {
            // A function value becomes a pointer to the function.
            // Function signatures are converted by `ToLLVMType`.
            return Ok(PointerType::get(ctx, 0).into());
        }
        Ok(ty)
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()> {
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let op_ref = op_dyn.op_ref();

        // Cast to the ToLLVMDialect interface and call rewrite
        if let Some(to_llvm) = pliron::op::op_cast::<dyn ToLLVMDialect>(op_ref) {
            to_llvm.rewrite(ctx, rewriter, operands_info)?;
        }

        Ok(())
    }
}

/// Apply dialect conversion from builtin to LLVM on a [ModuleOp].
pub fn convert_builtin_to_llvm(ctx: &mut Context, module: ModuleOp) -> Result<PassResult> {
    builtin_to_llvm_pass().run(
        module.get_operation(),
        ctx,
        &mut pliron::pass::AnalysisManager::default(),
    )
}

/// A [ModuleOp] pass that applies the builtin to LLVM dialect conversion
/// on every [Operation] in the module.
pub fn builtin_to_llvm_pass()
-> OpPass<ModuleOp, dialect_conversion::PassWrapper<BuiltinToLLVMConversion>> {
    let pass = dialect_conversion::PassWrapper::<BuiltinToLLVMConversion>::new(
        "builtin_to_llvm",
        BuiltinToLLVMConversion,
    );
    GuardedPass::new(OpGuard::<ModuleOp>::default(), pass)
}
