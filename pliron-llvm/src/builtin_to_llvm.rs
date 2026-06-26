//! Dialect conversion from builtin to LLVM dialect

use pliron::{
    builtin::{
        op_interfaces::{OneRegionInterface, SymbolOpInterface},
        ops::{ConstantOp as BuiltinConstantOp, FuncOp as BuiltinFuncOp, ModuleOp},
        type_interfaces::FunctionTypeInterface,
        types::FunctionType as BuiltinFunctionType,
    },
    common_traits::Verify,
    context::{Context, Ptr},
    derive::{op_interface_impl, type_interface_impl},
    input_err_noloc, input_error_noloc,
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
    r#type::{TypeHandle, TypedHandle, type_cast},
};

use crate::{
    ToLLVMDialect, ToLLVMType, ToLLVMTypeFn,
    ops::{ConstantOp as LLVMConstantOp, FuncOp as LLVMFuncOp},
    types::FuncType as LLVMFuncType,
};

#[derive(thiserror::Error, Debug)]
pub enum BuiltinToLLVMConversionError {
    #[error("Invalid function type, cannot be converted to LLVM function type")]
    InvalidFunctionType,
}

#[type_interface_impl]
impl ToLLVMType for BuiltinFunctionType {
    fn converter(&self) -> ToLLVMTypeFn {
        |self_ty: TypeHandle, ctx: &mut Context| {
            let func_type = self_ty.deref(ctx);
            let func_type_ref = type_cast::<dyn FunctionTypeInterface>(&*func_type)
                .expect("Expected a FunctionTypeInterface");

            let arg_types = func_type_ref.arg_types();
            let res_types = func_type_ref.res_types();

            if res_types.is_empty() || res_types.len() > 1 {
                return input_err_noloc!(BuiltinToLLVMConversionError::InvalidFunctionType);
            }
            let result_type = res_types[0];

            let llvm_func_type = LLVMFuncType::get(ctx, result_type, arg_types, false);
            Ok(llvm_func_type.into())
        }
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
        let const_value = self.get_value(ctx);

        // Create the LLVM constant operation with the same value
        let llvm_const = LLVMConstantOp::new(ctx, const_value);

        if let Err(e @ Error { .. }) = llvm_const.verify(ctx) {
            return Err(Error {
                kind: ErrorKind::InvalidInput,
                // We reset the error origin to be from here
                backtrace: pliron::deps::backtrace::Backtrace::capture(),
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

        // Get the function type from builtin.func
        let builtin_func_type = self.get_type(ctx);
        let llvm_converter = type_cast::<dyn ToLLVMType>(&*builtin_func_type.deref(ctx))
            .ok_or_else(|| {
                input_error_noloc!("builtin.func type does not implement ToLLVMType interface")
            })?
            .converter();
        let llvm_func_type = llvm_converter(builtin_func_type, ctx)?;
        let llvm_func_type = TypedHandle::from_handle(llvm_func_type, ctx)?;

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

/// Dialect conversion pattern for converting builtin ops to LLVM ops
#[derive(Default)]
pub struct BuiltinToLLVMConversion;

impl DialectConversion for BuiltinToLLVMConversion {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        let op_dyn = Operation::get_op_dyn(op, ctx);
        let op_ref = op_dyn.op_ref();

        // Check if this operation implements ToLLVMDialect
        op_impls::<dyn ToLLVMDialect>(op_ref)
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
