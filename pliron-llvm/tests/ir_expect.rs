// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! General tests that expect a known (golden) IR.

#![cfg(feature = "llvm-sys")]

use expect_test::expect;
use pliron::{
    context::Context, init_env_logger_for_tests, op::Op, operation::verify_operation,
    printable::Printable, result::Result,
};
use pliron_llvm::{
    from_llvm_ir,
    llvm_sys::core::{LLVMContext, LLVMMemoryBuffer, LLVMModule},
    to_llvm_ir,
};

mod common;

fn to_llvm_ir_o1(input: &str) -> Result<String> {
    init_env_logger_for_tests!();

    let ctx = &mut Context::new();
    let module_op = common::parse_op_verify(ctx, input)?;

    common::run_o1_passes_verify(ctx, module_op)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = to_llvm_ir::convert_module(ctx, &llvm_ctx, module_op)?;
    Ok(llvm_mod.to_string())
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

    let after = to_llvm_ir_o1(input)?;

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

    let err = to_llvm_ir_o1(input).expect_err("verifier must reject the flags");
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

/// A `select` without fast-math flags imported from LLVM IR must not carry a
/// spurious empty `llvm_select_fast_math_flags` attribute
#[test]
fn llvm_ir_select_without_fastmath_flags_omits_attr() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
            define float @choose(i1 %c, float %a, float %b) {
            entry:
              %r = select i1 %c, float %a, float %b
              ret float %r
            }
        "#;

    let llvm_ctx = LLVMContext::default();
    let buf = LLVMMemoryBuffer::from_str(input, "select_no_fmf");
    let llvm_mod =
        LLVMModule::from_ir_in_memory_buffer(&llvm_ctx, buf).expect("LLVM IR input should parse");

    let ctx = &mut Context::new();
    let module_op = from_llvm_ir::convert_module(ctx, &llvm_mod)?;
    verify_operation(module_op.get_operation(), ctx)?;

    let pliron_text = module_op.get_operation().disp(ctx).to_string();
    assert!(
        pliron_text.contains("llvm.select"),
        "expected a select op in the imported IR:\n{pliron_text}"
    );
    assert!(
        !pliron_text.contains("<>"),
        "select without fast-math flags should not print an empty attribute:\n{pliron_text}"
    );

    Ok(())
}
