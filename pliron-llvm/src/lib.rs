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
    pass_manager::{OpPass, OpPassManager, PassGroup},
    result::Result,
    r#type::{Type, TypeHandle},
};

use crate::ops::FuncOp;

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

/// Append -O1 passes to the given pass manager.
pub fn append_o1_passes(pm: &mut OpPassManager<ModuleOp>) {
    pm.add_pass(OpPass::<Mem2RegPass, FuncOp>::default());
    pm.add_pass(OpPass::<SCCPPass, FuncOp>::default());
    pm.add_pass(OpPass::<SimplifyCFGPass, FuncOp>::default());
    pm.add_pass(OpPass::<DCEPass, FuncOp>::default());
}
