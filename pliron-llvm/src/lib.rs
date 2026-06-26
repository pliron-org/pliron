//! LLVM Dialect for [pliron]

use pliron::{
    builtin::ops::ModuleOp,
    context::Context,
    derive::{op_interface, type_interface},
    irbuild::dialect_conversion::{DialectConversionRewriter, OperandsInfo},
    op::Op,
    opts::{
        constants::sccp::SCCPPass, dce::DCEPass, mem2reg::Mem2RegPass,
        simplify_cfg::SimplifyCFGPass,
    },
    pass::{NestedOpsPass, OpPass, Passes},
    result::Result,
    r#type::{Type, TypeHandle},
};

use crate::{builtin_to_llvm::builtin_to_llvm_pass, ops::FuncOp};

pub mod attributes;
pub mod builtin_to_llvm;
pub mod function_call_utils;
pub mod interface_impls;
pub mod op_interfaces;
pub mod ops;
pub mod types;

#[cfg(feature = "llvm-sys")]
pub mod from_llvm_ir;
#[cfg(feature = "llvm-sys")]
pub mod llvm_sys;
#[cfg(feature = "llvm-sys")]
pub mod to_llvm_ir;

/// Interface for rewriting to LLVM dialect.
#[op_interface]
pub trait ToLLVMDialect {
    /// Rewrite [self] to LLVM dialect.
    fn rewrite(
        &self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        operands_info: &OperandsInfo,
    ) -> Result<()>;

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// A function pointer type for the [ToLLVMType] interface.
pub type ToLLVMTypeFn = fn(self_ty: TypeHandle, &mut Context) -> Result<TypeHandle>;

/// Interface for converting to an LLVM type.
#[type_interface]
pub trait ToLLVMType {
    /// Get a function to convert [self] to an LLVM type.
    // We don't directly specify a conversion function here because
    // the caller cannot get `&dyn ToLLVMType` (&self) while also
    // passing `&mut Context` to the conversion function.
    fn converter(&self) -> ToLLVMTypeFn;

    fn verify(_ty: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// Append -O1 passes to the given list of passes.
pub fn append_o1_passes(module_passes: &mut OpPass<ModuleOp, Passes>) {
    let mut passes = Passes::default();
    passes.add_pass(OpPass::<FuncOp, Mem2RegPass>::default());
    passes.add_pass(OpPass::<FuncOp, SCCPPass>::default());
    passes.add_pass(OpPass::<FuncOp, SimplifyCFGPass>::default());
    passes.add_pass(OpPass::<FuncOp, DCEPass>::default());

    module_passes.add_pass(NestedOpsPass::new(passes));
    // Optimizations may introduce builtin ops that need to be converted to LLVM ops
    module_passes.add_pass(builtin_to_llvm_pass());
}
