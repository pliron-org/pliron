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
