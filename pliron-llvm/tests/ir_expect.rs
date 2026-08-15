// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! General tests that expect a known (golden) IR.

#![cfg(feature = "llvm-sys")]

use expect_test::expect;
use pliron::{context::Context, init_env_logger_for_tests, printable::Printable, result::Result};
use pliron_llvm::{llvm_sys::core::LLVMContext, to_llvm_ir};

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
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "select_fmf")?;

    let pliron_text = module_op.disp(ctx).to_string();
    expect![[r#"
        builtin.module @select_fmf 
        {
          ^block1v1():
            llvm.func @choose: llvm.func <builtin.fp32 (builtin.integer i1, builtin.fp32 , builtin.fp32 ) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: builtin.integer i1, v1: builtin.fp32 , v2: builtin.fp32 ):
                r_v3 = llvm.select <NNAN | NSZ> v0 ? v1 : v2 : builtin.fp32  !0;
                llvm.return r_v3
            }
        }"#]].assert_eq(&pliron_text);

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = to_llvm_ir::convert_module(ctx, &out_llvm_ctx, module_op)?;
    let out = out_mod.to_string();
    expect![[r#"
        ; ModuleID = 'select_fmf'
        source_filename = "select_fmf"

        define float @choose(i1 %0, float %1, float %2) {
        entry_block2v1:
          %r_v3 = select nnan nsz i1 %0, float %1, float %2
          ret float %r_v3
        }
    "#]]
    .assert_eq(&out);
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
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "select_no_fmf")?;

    let pliron_text = module_op.disp(ctx).to_string();
    expect![[r#"
        builtin.module @select_no_fmf 
        {
          ^block1v1():
            llvm.func @choose: llvm.func <builtin.fp32 (builtin.integer i1, builtin.fp32 , builtin.fp32 ) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: builtin.integer i1, v1: builtin.fp32 , v2: builtin.fp32 ):
                r_v3 = llvm.select  v0 ? v1 : v2 : builtin.fp32  !0;
                llvm.return r_v3
            }
        }"#]].assert_eq(&pliron_text);

    Ok(())
}

/// Combinations of `struct` types
#[test]
fn llvm_ir_struct_combinations_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
        %Packed = type <{ i8, i32 }>
        %Unpacked = type { i8, i32 }
        %Nested = type { %Packed, %Unpacked, i16 }
        %List = type { i32, %List* }
        %Opaque = type opaque

        define void @use_structs(%Opaque* %op) {
        entry:
          %packed = alloca %Packed
          %p0 = insertvalue %Packed undef, i8 1, 0
          %p1 = insertvalue %Packed %p0, i32 2, 1
          store %Packed %p1, %Packed* %packed

          %unpacked = alloca %Unpacked
          %u0 = insertvalue %Unpacked undef, i8 3, 0
          %u1 = insertvalue %Unpacked %u0, i32 4, 1
          store %Unpacked %u1, %Unpacked* %unpacked

          %anonu = alloca { i8, i32 }
          %au0 = insertvalue { i8, i32 } undef, i8 5, 0
          %au1 = insertvalue { i8, i32 } %au0, i32 6, 1
          store { i8, i32 } %au1, { i8, i32 }* %anonu

          %anonp = alloca <{ i8, i32 }>
          %ap0 = insertvalue <{ i8, i32 }> undef, i8 7, 0
          %ap1 = insertvalue <{ i8, i32 }> %ap0, i32 8, 1
          store <{ i8, i32 }> %ap1, <{ i8, i32 }>* %anonp

          %nested = alloca %Nested
          %n0 = insertvalue %Nested undef, %Packed %p1, 0
          %n1 = insertvalue %Nested %n0, %Unpacked %u1, 1
          %n2 = insertvalue %Nested %n1, i16 9, 2
          store %Nested %n2, %Nested* %nested

          %list = alloca %List
          %l0 = insertvalue %List undef, i32 10, 0
          %l1 = insertvalue %List %l0, %List* null, 1
          store %List %l1, %List* %list

          ret void
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "struct_combos")?;

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = to_llvm_ir::convert_module(ctx, &out_llvm_ctx, module_op)?;
    let out = out_mod.to_string();
    expect![[r#"
        ; ModuleID = 'struct_combos'
        source_filename = "struct_combos"

        %Packed = type <{ i8, i32 }>
        %Unpacked = type { i8, i32 }
        %Nested = type { %Packed, %Unpacked, i16 }
        %List = type { i32, ptr }

        define void @use_structs(ptr %0) {
        entry_block2v1:
          %packed_v2 = alloca %Packed, align 8
          store %Packed <{ i8 1, i32 2 }>, ptr %packed_v2, align 1
          %unpacked_v8 = alloca %Unpacked, align 8
          store %Unpacked { i8 3, i32 4 }, ptr %unpacked_v8, align 4
          %anonu_v14 = alloca { i8, i32 }, align 8
          store { i8, i32 } { i8 5, i32 6 }, ptr %anonu_v14, align 4
          %anonp_v20 = alloca <{ i8, i32 }>, align 8
          store <{ i8, i32 }> <{ i8 7, i32 8 }>, ptr %anonp_v20, align 1
          %nested_v26 = alloca %Nested, align 8
          store %Nested { %Packed <{ i8 1, i32 2 }>, %Unpacked { i8 3, i32 4 }, i16 9 }, ptr %nested_v26, align 4
          %list_v32 = alloca %List, align 8
          store %List { i32 10, ptr null }, ptr %list_v32, align 8
          ret void
        }
    "#]]
    .assert_eq(&out);
    Ok(())
}
