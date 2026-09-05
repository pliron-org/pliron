//! Common test utilities for the pliron-llvm crate.

use pliron::{
    builtin::ops::ModuleOp,
    context::Context,
    irbuild::{
        cloning::IrMapping,
        equivalence::{EqResult, IGNORE_LOC_NAMES, operation_eq, operation_hash},
    },
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
    to_llvm_ir,
};

/// Parses an Op from the given input string, verifies, and returns it
#[allow(dead_code)]
pub fn parse_op_verify<O: Op>(ctx: &mut Context, input: &str) -> Result<O> {
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input)?;
    let module_op = Operation::get_op::<O>(op, ctx).unwrap();
    log::debug!("Parsed PLIR:\n{}", module_op.get_operation().disp(ctx));
    verify_operation(op, ctx)?;
    Ok(module_op)
}

/// Verifies `op`, prints it, parses the printed text back into `ctx`, verifies that too,
/// and asserts that the reparsed IR is structurally equal to (and hashes the same as) the
/// original. Returns the printed text along with the reparsed op.
#[allow(dead_code)]
pub fn print_parse_verify<O: Op>(ctx: &mut Context, op: O) -> Result<(String, O)> {
    verify_operation(op.get_operation(), ctx)?;

    let printed = op.get_operation().disp(ctx).to_string();
    let reparsed = parse_op_verify::<O>(ctx, &printed)?;

    assert_eq!(
        operation_eq(
            ctx,
            &mut IrMapping::default(),
            op.get_operation(),
            reparsed.get_operation(),
            &IGNORE_LOC_NAMES,
        ),
        EqResult::Eq,
        "IR parsed back from its printed form differs from the original"
    );
    assert_eq!(
        operation_hash(ctx, op.get_operation(), &IGNORE_LOC_NAMES),
        operation_hash(ctx, reparsed.get_operation(), &IGNORE_LOC_NAMES),
        "IR parsed back from its printed form hashes differently from the original"
    );

    Ok((printed, reparsed))
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

/// Converts `module_op` into an LLVM-IR module, and verifies it.
#[cfg(feature = "llvm-sys")]
#[allow(dead_code)]
pub fn to_llvm_ir_verify(
    ctx: &Context,
    llvm_ctx: &LLVMContext,
    module_op: ModuleOp,
) -> Result<LLVMModule> {
    let llvm_mod = to_llvm_ir::convert_module(ctx, llvm_ctx, module_op)?;
    llvm_mod.verify().map_err(|err| {
        pliron::verify_error_noloc!("Verification of the LLVM module failed: {}", err)
    })?;
    Ok(llvm_mod)
}
