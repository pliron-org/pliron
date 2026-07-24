// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

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
    pass::{AnalysisManager, OpPass, Pass, Passes},
    result::Result,
};
use pliron_llvm::{llvm_sys::core::LLVMContext, to_llvm_ir};

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

    // Run O1 passes (which also includes the builtin to LLVM conversion pass) on the module
    let mut passes = OpPass::<ModuleOp, Passes>::default();
    pliron_llvm::append_o1_passes(&mut passes);
    passes.run(op, ctx, &mut AnalysisManager::default())?;

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
        entry_block_1_0_block2v1:
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
        entry_block_1_0_block2v1:
          ret i64 42
        }
    "#]]
    .assert_eq(&after);

    Ok(())
}

#[test]
fn builtin_unit_func_converts_to_llvm_void_func() -> Result<()> {
    let input = r#"
        builtin.module @m {
        ^block_0_0():
          builtin.func @foo: builtin.function <() -> (builtin.unit)> {
          ^entry_block_1_0():
            llvm.return
          }
        }
    "#;

    let after = run_conversion_pipeline(input)?;

    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"

        define void @foo() {
        entry_block_1_0_block2v1:
          ret void
        }
    "#]]
    .assert_eq(&after);

    Ok(())
}

#[test]
fn float_select_preserves_fastmath_flags() -> Result<()> {
    let input = r#"
        builtin.module @m {
        ^block_0_0():
          llvm.func @foo: llvm.func <builtin.fp32(builtin.integer i1, builtin.fp32, builtin.fp32) variadic = false> [] {
          ^entry_block_1_0(c: builtin.integer i1, a: builtin.fp32, b: builtin.fp32):
            s = llvm.select <NNAN> c ? a : b : builtin.fp32;
            llvm.return s
          }
        }
    "#;

    let after = run_conversion_pipeline(input)?;

    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"

        define float @foo(i1 %0, float %1, float %2) {
        entry_block_1_0_block2v1:
          %s_v3 = select nnan i1 %0, float %1, float %2
          ret float %s_v3
        }
    "#]]
    .assert_eq(&after);

    Ok(())
}

/// Fast-math flags are only valid on selects of floating-point type; the
/// verifier must reject them on an integer select.
#[test]
fn int_select_with_fastmath_flags_is_rejected() {
    let input = r#"
        builtin.module @m {
        ^block_0_0():
          llvm.func @foo: llvm.func <builtin.integer i64(builtin.integer i1, builtin.integer i64, builtin.integer i64) variadic = false> [] {
          ^entry_block_1_0(c: builtin.integer i1, a: builtin.integer i64, b: builtin.integer i64):
            s = llvm.select <NNAN> c ? a : b : builtin.integer i64;
            llvm.return s
          }
        }
    "#;

    let err = run_conversion_pipeline(input).expect_err("verifier must reject the flags");
    assert!(
        err.to_string()
            .contains("Fast-math flags are only allowed on selects of floating-point type"),
        "unexpected error: {err}"
    );
}

/// A `select` with fast-math flags imported from LLVM IR must carry the flags
/// through pliron and back out to LLVM IR.
#[test]
fn llvm_ir_select_fastmath_flags_roundtrip() -> Result<()> {
    use pliron::{op::Op, printable::Printable};
    use pliron_llvm::{
        from_llvm_ir,
        llvm_sys::core::{LLVMMemoryBuffer, LLVMModule},
    };

    init_env_logger_for_tests!();
    let input = r#"
        define float @choose(i1 %c, float %a, float %b) {
        entry:
          %r = select nnan nsz i1 %c, float %a, float %b
          ret float %r
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let buf = LLVMMemoryBuffer::from_str(input, "select_fmf");
    let llvm_mod =
        LLVMModule::from_ir_in_memory_buffer(&llvm_ctx, buf).expect("LLVM IR input should parse");

    let ctx = &mut Context::new();
    let module_op = from_llvm_ir::convert_module(ctx, &llvm_mod)?;
    verify_operation(module_op.get_operation(), ctx)?;

    let pliron_text = module_op.get_operation().disp(ctx).to_string();
    assert!(
        pliron_text.contains("NNAN"),
        "fast-math flags lost on import:\n{pliron_text}"
    );

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = to_llvm_ir::convert_module(ctx, &out_llvm_ctx, module_op)?;
    let out = out_mod.to_string();
    assert!(
        out.contains("select nnan nsz"),
        "fast-math flags lost on export:\n{out}"
    );
    Ok(())
}
