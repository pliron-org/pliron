//! Common test utilities for the pliron-llvm crate.

use pliron::{
    builtin::ops::ModuleOp,
    context::Context,
    irfmt::parsers::spaced,
    op::{Op, verify_op},
    operation::{Operation, verify_operation},
    parsable::parse_from_str,
    pass::{AnalysisManager, OpPass, Pass, Passes},
    printable::Printable,
    result::Result,
};

#[cfg(feature = "llvm-sys")]
use pliron_llvm::{
    from_llvm_ir,
    llvm_sys::core::{LLVMContext, LLVMMemoryBuffer, LLVMModule},
};

/// Parses an Op from the given input string, verifies, and returns it
pub fn parse_op_verify<O: Op>(ctx: &mut Context, input: &str) -> Result<O> {
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input)?;
    let module_op = Operation::get_op::<O>(op, ctx).unwrap();
    log::debug!("Parsed PLIR:\n{}", module_op.get_operation().disp(ctx));
    verify_operation(op, ctx)?;
    Ok(module_op)
}

/// Run O1 passes on `module_op`
#[allow(dead_code)]
pub fn run_o1_passes_verify(ctx: &mut Context, module_op: ModuleOp) -> Result<()> {
    // Run O1 passes (which also includes the builtin to LLVM conversion pass) on the module
    let mut passes = OpPass::<ModuleOp, Passes>::default();
    pliron_llvm::append_o1_passes(&mut passes);
    passes.run(
        module_op.get_operation(),
        ctx,
        &mut AnalysisManager::default(),
    )?;
    log::debug!(
        "O1 optimized PLIR: \n{}",
        module_op.get_operation().disp(ctx)
    );
    verify_op(&module_op, ctx)?;
    Ok(())
}

/// Parses `input` as LLVM IR text, converts it into a pliron [ModuleOp], and verifies it.
#[cfg(feature = "llvm-sys")]
#[allow(dead_code)]
pub fn parse_llvm_ir_verify(
    ctx: &mut Context,
    llvm_ctx: &LLVMContext,
    input: &str,
    name: &str,
) -> Result<ModuleOp> {
    let buf = LLVMMemoryBuffer::from_str(input, name);
    let llvm_mod =
        LLVMModule::from_ir_in_memory_buffer(llvm_ctx, buf).expect("LLVM IR input should parse");

    let module_op = from_llvm_ir::convert_module(ctx, &llvm_mod)?;
    verify_operation(module_op.get_operation(), ctx)?;
    Ok(module_op)
}
