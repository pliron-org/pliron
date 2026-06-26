//! Tests for dialect conversion from builtin to LLVM ops.

#![cfg(feature = "llvm-sys")]

use expect_test::expect;

use pliron::{
    builtin::ops::ModuleOp,
    combine::Parser,
    context::Context,
    init_env_logger_for_tests,
    irfmt::parsers::spaced,
    operation::{Operation, verify_operation},
    parsable::{self, state_stream_from_iterator},
    pass_manager::{AnalysisManager, OpPassManager, Pass, PassManager},
    result::Result,
};
use pliron_llvm::{builtin_to_llvm, llvm_sys::core::LLVMContext, to_llvm_ir};

fn run_conversion_pipeline(input: &str) -> Result<String> {
    init_env_logger_for_tests!();

    let ctx = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        input.chars(),
        parsable::State::new(ctx, pliron::location::Source::InMemory),
    );
    let op = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .expect("textual IR should parse")
        .0;
    let module_op = Operation::get_op::<ModuleOp>(op, ctx).unwrap();

    verify_operation(op, ctx)?;

    let mut o1_passes = OpPassManager::<ModuleOp>::default();
    pliron_llvm::append_o1_passes(&mut o1_passes);
    let builtin_to_llvm_pass = builtin_to_llvm::builtin_to_llvm_pass();

    // Run a nested O1 pipeline and a builtin to LLVM conversion pass on the module operation.
    let mut pm = PassManager::default();
    pm.add_pass(o1_passes);
    pm.add_pass(builtin_to_llvm_pass);
    pm.run(op, ctx, &mut AnalysisManager::default())?;

    verify_operation(op, ctx)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = to_llvm_ir::convert_module(ctx, &llvm_ctx, module_op)?;
    Ok(llvm_mod.to_string())
}

#[test]
fn mixed_constant_ops_fold_then_lower_to_llvm() -> Result<()> {
    let input = r#"
        builtin.module @m {
        ^block_0_0():
          llvm.func @foo: llvm.func <builtin.integer i64() variadic = false> [] {
          ^entry_block_1_0():
            a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
            b = llvm.constant <builtin.integer <4: i64>> : builtin.integer i64;
            sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
            llvm.return sum
          }
        }
    "#;

    let after = run_conversion_pipeline(input)?;

    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"

        define i64 @foo() {
        entry_block_1_0_block1v1:
          ret i64 7
        }
    "#]]
    .assert_eq(&after);

    Ok(())
}

#[test]
fn builtin_func_converts_to_llvm_func() -> Result<()> {
    let input = r#"
        builtin.module @m {
        ^block_0_0():
          builtin.func @foo: builtin.function <() -> (builtin.integer i64)> {
          ^entry_block_1_0():
            c0 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
            llvm.return c0
          }
        }
    "#;

    let after = run_conversion_pipeline(input)?;

    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"

        define i64 @foo() {
        entry_block_1_0_block1v1:
          ret i64 42
        }
    "#]]
    .assert_eq(&after);

    Ok(())
}
