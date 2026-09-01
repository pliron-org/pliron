// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! General tests that expect a known (golden) IR.

#![cfg(feature = "llvm-sys")]

use expect_test::expect;
use pliron::{
    builtin::ops::ModuleOp,
    context::Context,
    init_env_logger_for_tests,
    op::Op,
    printable::Printable,
    result::{Error, Result},
};
use pliron_llvm::{
    attributes::ConstAggregateVerifyErr,
    from_llvm_ir,
    llvm_sys::core::LLVMContext,
    ops::{ConstantOpVerifyErr, SelectOpVerifyErr},
    to_llvm_ir,
};
mod common;

fn to_llvm_ir_o1(input: &str) -> Result<String> {
    init_env_logger_for_tests!();

    let ctx = &mut Context::new();
    let module_op = common::parse_op_verify(ctx, input)?;

    common::run_o1_passes_verify(ctx, module_op)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    Ok(llvm_mod.to_string())
}

#[test]
fn data_layout_target_triple() -> Result<()> {
    let input = r#"
        builtin.module @m
          [llvm_data_layout: builtin.string "e-p:64:64-A5",
           llvm_target_triple: builtin.string "amdgcn-amd-amdhsa"] {
        ^block_0_0():
          llvm.func @foo: llvm.func <llvm.void(builtin.integer i32) variadic = false> [] {
          ^entry_block_1_0(n: builtin.integer i32):
            ptr = llvm.alloca [builtin.integer i32 x n] : llvm.ptr (5);
            llvm.return
          }
        }
    "#;

    let ctx = &mut Context::new();
    let module_op = common::parse_op_verify(ctx, input)?;
    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;

    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"
        target datalayout = "e-p:64:64-A5"
        target triple = "amdgcn-amd-amdhsa"

        define void @foo(i32 %0) {
        entry_block_1_0_block2v1:
          %ptr_v1 = alloca i32, i32 %0, align 4, addrspace(5)
          ret void
        }
    "#]]
    .assert_eq(&llvm_mod.to_string());
    assert_eq!(llvm_mod.data_layout(), "e-p:64:64-A5");
    assert_eq!(llvm_mod.target_triple(), "amdgcn-amd-amdhsa");

    // The data layout and target triple must survive a round trip back to pliron.
    let ctx2 = &mut Context::new();
    let module_op2 = from_llvm_ir::convert_module(ctx2, &llvm_mod)?;
    expect![[r#"
        builtin.module @m 
          [llvm_data_layout: builtin.string "e-p:64:64-A5", llvm_target_triple: builtin.string "amdgcn-amd-amdhsa"]
        {
          ^block_0_0_block1v1() !0:
            llvm.func @foo: llvm.func <llvm.void (builtin.integer i32) variadic = false>
              [] 
            {
              ^entry_block_1_0_block2v1(n_v0: builtin.integer i32) !1:
                ptr_v1 = llvm.alloca [builtin.integer i32 x n_v0]  : llvm.ptr (5) !2;
                llvm.return  !3
            } !4
        }"#]].assert_eq(&module_op2.disp(ctx).to_string());
    assert_eq!(
        pliron_llvm::attributes::get_data_layout(ctx2, module_op2).as_deref(),
        Some("e-p:64:64-A5")
    );
    assert_eq!(
        pliron_llvm::attributes::get_target_triple(ctx2, module_op2).as_deref(),
        Some("amdgcn-amd-amdhsa")
    );

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
    let err = err.err.downcast_ref::<SelectOpVerifyErr>().unwrap();
    assert!(matches!(err, SelectOpVerifyErr::FastMathFlagsOnNonFloatErr));
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
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, module_op)?;
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

/// GEP no-wrap flags must survive LLVM -> pliron -> LLVM conversion.
#[test]
fn llvm_ir_gep_no_wrap_flags_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
        define ptr @gep_flags(ptr %p, i64 %i) {
        entry:
          %plain = getelementptr i8, ptr %p, i64 %i
          %nusw = getelementptr nusw i8, ptr %p, i64 %i
          %nuw = getelementptr nuw i8, ptr %p, i64 %i
          %inbounds = getelementptr inbounds i8, ptr %p, i64 %i
          %both = getelementptr inbounds nuw i8, ptr %p, i64 %i
          ret ptr %both
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "gep_flags")?;

    expect![[r#"
        builtin.module @gep_flags 
        {
          ^block1v1():
            llvm.func @gep_flags: llvm.func <llvm.ptr (0)(llvm.ptr (0), builtin.integer i64) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: llvm.ptr (0), v1: builtin.integer i64):
                plain_v2 = llvm.gep <builtin.integer i8> (v0, v1)[OperandIdx(1)] : llvm.ptr (0) !0;
                nusw_v3 = llvm.gep <builtin.integer i8> (v0, v1)<NUSW>[OperandIdx(1)] : llvm.ptr (0) !1;
                nuw_v4 = llvm.gep <builtin.integer i8> (v0, v1)<NUW>[OperandIdx(1)] : llvm.ptr (0) !2;
                inbounds_v5 = llvm.gep <builtin.integer i8> (v0, v1)<INBOUNDS>[OperandIdx(1)] : llvm.ptr (0) !3;
                both_v6 = llvm.gep <builtin.integer i8> (v0, v1)<INBOUNDS | NUW>[OperandIdx(1)] : llvm.ptr (0) !4;
                llvm.return both_v6
            }
        }"#]].assert_eq(&module_op.disp(ctx).to_string());

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'gep_flags'
        source_filename = "gep_flags"

        define ptr @gep_flags(ptr %0, i64 %1) {
        entry_block2v1:
          %plain_v2 = getelementptr i8, ptr %0, i64 %1
          %nusw_v3 = getelementptr nusw i8, ptr %0, i64 %1
          %nuw_v4 = getelementptr nuw i8, ptr %0, i64 %1
          %inbounds_v5 = getelementptr inbounds i8, ptr %0, i64 %1
          %both_v6 = getelementptr inbounds nuw i8, ptr %0, i64 %1
          ret ptr %both_v6
        }
    "#]]
    .assert_eq(&out_mod.to_string());
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
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, module_op)?;
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

/// Constant aggregates, vector splats,the symbol addresses as global initializers.
#[test]
fn constant_aggregates_and_splats_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
        @g = global i32 0
        @array = global [4 x i32] [i32 1, i32 2, i32 3, i32 4]
        @struct = global { i32, float, ptr } { i32 1, float 2.0, ptr @g }
        @nested = global [2 x { i32, i32 }] [{ i32, i32 } { i32 1, i32 2 },
                                             { i32, i32 } { i32 3, i32 4 }]
        @vtable = global [2 x ptr] [ptr @g, ptr @f]
        @addr = global ptr @g
        @string = global [6 x i8] c"hello\00"
        @vector = global <4 x i32> <i32 1, i32 2, i32 3, i32 4>
        @splat = global <4 x i32> splat (i32 7)

        define void @f(ptr %p) {
        entry:
          store [4 x i32] [i32 1, i32 2, i32 3, i32 4], ptr %p
          store { i32, ptr } { i32 5, ptr @g }, ptr %p
          store [6 x i8] c"hello\00", ptr %p
          ret void
        }

        define <4 x i32> @fixed() {
          ret <4 x i32> splat (i32 7)
        }

        define <4 x float> @fp() {
          ret <4 x float> splat (float 2.5)
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "constants")?;
    let printed = module_op.disp(ctx).to_string();
    expect![[r#"
        builtin.module @constants 
        {
          ^block1v1():
            llvm.global @g : builtin.integer i32
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = builtin.integer <0: i32>;
            llvm.global @array : llvm.array [4 x builtin.integer i32]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.array [4 x builtin.integer i32]>;
            llvm.global @struct : llvm.struct <{ builtin.integer i32, builtin.fp32 , llvm.ptr (0) } : Unpacked>
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[builtin.integer <1: i32>, builtin.single 2, llvm.symbol_addr <@g : llvm.ptr (0)>] : llvm.struct <{ builtin.integer i32, builtin.fp32 , llvm.ptr (0) } : Unpacked>>;
            llvm.global @nested : llvm.array [2 x llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>, llvm.aggregate <[builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>] : llvm.array [2 x llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>]>;
            llvm.global @vtable : llvm.array [2 x llvm.ptr (0)]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[llvm.symbol_addr <@g : llvm.ptr (0)>, llvm.symbol_addr <@f : llvm.ptr (0)>] : llvm.array [2 x llvm.ptr (0)]>;
            llvm.global @addr : llvm.ptr (0)
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.symbol_addr <@g : llvm.ptr (0)>;
            llvm.global @string : llvm.array [6 x builtin.integer i8]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.bytes [104, 101, 108, 108, 111, 0];
            llvm.global @vector : llvm.vector <Fixed x 4 x builtin.integer i32>
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.vector <Fixed x 4 x builtin.integer i32>>;
            llvm.global @splat : llvm.vector <Fixed x 4 x builtin.integer i32>
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 4 x builtin.integer i32>>;
            llvm.func @f: llvm.func <llvm.void (llvm.ptr (0)) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: llvm.ptr (0)):
                v1 = llvm.constant <llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.array [4 x builtin.integer i32]>> : llvm.array [4 x builtin.integer i32];
                v2 = llvm.constant <llvm.aggregate <[builtin.integer <5: i32>, llvm.symbol_addr <@g : llvm.ptr (0)>] : llvm.struct <{ builtin.integer i32, llvm.ptr (0) } : Unpacked>>> : llvm.struct <{ builtin.integer i32, llvm.ptr (0) } : Unpacked>;
                v3 = llvm.constant <llvm.bytes [104, 101, 108, 108, 111, 0]> : llvm.array [6 x builtin.integer i8];
                llvm.store *v0 <- v1 [align : 4];
                llvm.store *v0 <- v2 [align : 8];
                llvm.store *v0 <- v3 [align : 1];
                llvm.return 
            };
            llvm.func @fixed: llvm.func <llvm.vector <Fixed x 4 x builtin.integer i32>() variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block3v1():
                v4 = llvm.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 4 x builtin.integer i32>>> : llvm.vector <Fixed x 4 x builtin.integer i32>;
                llvm.return v4
            };
            llvm.func @fp: llvm.func <llvm.vector <Fixed x 4 x builtin.fp32 >() variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block4v1():
                v5 = llvm.constant <llvm.splat <builtin.single 2.5 : llvm.vector <Fixed x 4 x builtin.fp32 >>> : llvm.vector <Fixed x 4 x builtin.fp32 >;
                llvm.return v5
            }
        }"#]]
    .assert_eq(&printed);

    // The printed constants must parse back and verify.
    let parse_ctx = &mut Context::new();
    common::parse_op_verify::<ModuleOp>(parse_ctx, &printed)?;

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'constants'
        source_filename = "constants"

        @g = global i32 0
        @array = global [4 x i32] [i32 1, i32 2, i32 3, i32 4]
        @struct = global { i32, float, ptr } { i32 1, float 2.000000e+00, ptr @g }
        @nested = global [2 x { i32, i32 }] [{ i32, i32 } { i32 1, i32 2 }, { i32, i32 } { i32 3, i32 4 }]
        @vtable = global [2 x ptr] [ptr @g, ptr @f]
        @addr = global ptr @g
        @string = global [6 x i8] c"hello\00"
        @vector = global <4 x i32> <i32 1, i32 2, i32 3, i32 4>
        @splat = global <4 x i32> splat (i32 7)

        define void @f(ptr %0) {
        entry_block2v1:
          store [4 x i32] [i32 1, i32 2, i32 3, i32 4], ptr %0, align 4
          store { i32, ptr } { i32 5, ptr @g }, ptr %0, align 8
          store [6 x i8] c"hello\00", ptr %0, align 1
          ret void
        }

        define <4 x i32> @fixed() {
        entry_block3v1:
          ret <4 x i32> splat (i32 7)
        }

        define <4 x float> @fp() {
        entry_block4v1:
          ret <4 x float> splat (float 2.500000e+00)
        }
    "#]]
    .assert_eq(&out_mod.to_string());

    Ok(())
}

#[test]
fn scalable_vector_splat() -> Result<()> {
    init_env_logger_for_tests!();
    let scalable_i32 = "llvm.vector <Scalable x 4 x builtin.integer i32>";
    // We don't start from LLVM-IR as we can't import this
    let input = format!(
        r#"
        builtin.module @m {{
        ^block_0_0():
          llvm.func @f: llvm.func <{scalable_i32}() variadic = false> [] {{
          ^entry_block_1_0():
            c = llvm.constant <llvm.splat <builtin.integer <7: i32> : {scalable_i32}>> : {scalable_i32};
            llvm.return c
          }}
        }}
    "#
    );

    let ctx = &mut Context::new();
    let module_op = common::parse_op_verify::<ModuleOp>(ctx, &input)?;
    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"

        define <vscale x 4 x i32> @f() {
        entry_block_1_0_block2v1:
          ret <vscale x 4 x i32> splat (i32 7)
        }
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}

#[test]
fn ill_typed_constants_are_rejected() {
    let module = |value: &str, ty: &str| {
        format!(
            r#"
            builtin.module @m {{
            ^block_0_0():
              llvm.func @f: llvm.func <llvm.void() variadic = false> [] {{
              ^entry_block_1_0():
                c = llvm.constant <{value}> : {ty};
                llvm.return
              }}
            }}
        "#
        )
    };
    // Utility to parse and verify a module, checking that it fails.
    let rejected = |value: &str, ty: &str, expectation: &str| -> Error {
        let ctx = &mut Context::new();
        let Err(err) = common::parse_op_verify::<ModuleOp>(ctx, &module(value, ty)) else {
            panic!("{expectation}");
        };
        err
    };

    let err = rejected(
        "llvm.aggregate <[builtin.integer <1: i32>] : llvm.array [2 x builtin.integer i32]>",
        "llvm.array [2 x builtin.integer i32]",
        "an aggregate with too few elements must be rejected",
    );
    assert!(matches!(
        err.err.downcast_ref::<ConstAggregateVerifyErr>().unwrap(),
        ConstAggregateVerifyErr::NumElements(..)
    ));

    let err = rejected(
        "llvm.aggregate <[builtin.integer <1: i64>, builtin.integer <2: i64>] \
         : llvm.array [2 x builtin.integer i32]>",
        "llvm.array [2 x builtin.integer i32]",
        "an aggregate whose elements are ill typed must be rejected",
    );
    assert!(matches!(
        err.err.downcast_ref::<ConstAggregateVerifyErr>().unwrap(),
        ConstAggregateVerifyErr::ElementType(..)
    ));

    // A scalable vector's element count isn't known statically, so its elements cannot
    // be listed out; such a constant has to be a splat.
    let scalable = "llvm.vector <Scalable x 4 x builtin.integer i32>";
    let err = rejected(
        &format!("llvm.aggregate <[builtin.integer <1: i32>] : {scalable}>"),
        scalable,
        "a scalable vector with its elements listed must be rejected",
    );
    expect![[r#"
        Compilation error: verification failed.
        A constant of the scalable vector type llvm.vector <Scalable x 4 x builtin.integer i32> must use llvm.splat"#]].assert_eq(&err.to_string());

    // A splat's type is a `VectorType`, so a non-vector one doesn't parse.
    let err = rejected(
        "llvm.splat <builtin.integer <1: i32> : llvm.array [2 x builtin.integer i32]>",
        "llvm.array [2 x builtin.integer i32]",
        "a splat of a non-vector type must be rejected",
    );
    expect![[r#"
        Compilation error: invalid input program.
        Parse error at line: 6, column: 75
        Expected type llvm.vector, but found llvm.array
    "#]]
    .assert_eq(&err.to_string());

    // A byte string's type is an array of exactly as many `i8`s as it has bytes.
    let err = rejected(
        "llvm.bytes [104, 105]",
        "llvm.array [3 x builtin.integer i8]",
        "a byte string whose array is of the wrong length must be rejected",
    );
    assert!(matches!(
        err.err.downcast_ref::<ConstantOpVerifyErr>().unwrap(),
        ConstantOpVerifyErr::ResultTypeMismatch(..)
    ));

    let err = rejected(
        "llvm.bytes [104, 105]",
        "llvm.array [2 x builtin.integer i32]",
        "a byte string that isn't an array of i8 must be rejected",
    );
    assert!(matches!(
        err.err.downcast_ref::<ConstantOpVerifyErr>().unwrap(),
        ConstantOpVerifyErr::ResultTypeMismatch(..)
    ));

    let err = rejected(
        "builtin.integer <1: i32>",
        "builtin.integer i64",
        "a value whose type isn't the constant's must be rejected",
    );
    assert!(matches!(
        err.err.downcast_ref::<ConstantOpVerifyErr>().unwrap(),
        ConstantOpVerifyErr::ResultTypeMismatch(..)
    ));
}

#[test]
fn named_syncscopes_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();

    let input = r#"
        define void @f(ptr %p) {
        entry:
          fence syncscope("device") seq_cst
          fence syncscope("singlethread") seq_cst
          fence seq_cst
          %v = load atomic i32, ptr %p syncscope("agent") seq_cst, align 4
          store atomic i32 %v, ptr %p syncscope("block") seq_cst, align 4
          ret void
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "syncscopes")?;

    expect![[r#"
        builtin.module @syncscopes 
        {
          ^block1v1():
            llvm.func @f: llvm.func <llvm.void (llvm.ptr (0)) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: llvm.ptr (0)):
                llvm.fence syncscope : NamedScope("device") SeqCst;
                llvm.fence syncscope : SingleThread SeqCst;
                llvm.fence syncscope : System SeqCst;
                v_v1 = llvm.atomic_load v0 [align : 4] syncscope : NamedScope("agent") SeqCst : builtin.integer i32 !0;
                llvm.atomic_store *v0 <- v_v1 [align : 4] syncscope : NamedScope("block") SeqCst;
                llvm.return 
            }
        }"#]].assert_eq(&module_op.disp(ctx).to_string());

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, module_op)?;
    let out = out_mod.to_string();
    expect![[r#"
        ; ModuleID = 'syncscopes'
        source_filename = "syncscopes"

        define void @f(ptr %0) {
        entry_block2v1:
          fence syncscope("device") seq_cst
          fence syncscope("singlethread") seq_cst
          fence seq_cst
          %v_v1 = load atomic i32, ptr %0 syncscope("agent") seq_cst, align 4
          store atomic i32 %v_v1, ptr %0 syncscope("block") seq_cst, align 4
          ret void
        }
    "#]]
    .assert_eq(&out);
    Ok(())
}

#[test]
fn llvm_ir_volatile_load_store_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
        define i32 @volatile_rw(ptr %p, i32 %x) {
        entry:
          %v = load volatile i32, ptr %p, align 4
          store volatile i32 %x, ptr %p, align 4
          %plain = load i32, ptr %p, align 4
          store i32 %plain, ptr %p, align 4
          ret i32 %v
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "volatile_load_store")?;

    let pliron_text = module_op.get_operation().disp(ctx).to_string();
    expect!["2"].assert_eq(&pliron_text.matches("[volatile : true]").count().to_string());

    // The printed Pliron form must itself round-trip through the parser. This also
    // verifies that non-volatile load/store syntax remains unambiguous next to alignment.
    let reparsed_ctx = &mut Context::new();
    let reparsed_module = common::parse_op_verify(reparsed_ctx, &pliron_text)?;

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = to_llvm_ir::convert_module(reparsed_ctx, &out_llvm_ctx, reparsed_module)?;
    let out = out_mod.to_string();

    let memory_op_summary = format!(
        "load volatile: {}\nstore volatile: {}\nload: {}\nstore: {}",
        out.matches("load volatile").count(),
        out.matches("store volatile").count(),
        out.matches("load i32").count(),
        out.matches("store i32").count(),
    );
    expect![[r#"
        load volatile: 1
        store volatile: 1
        load: 1
        store: 1"#]]
    .assert_eq(&memory_op_summary);
    Ok(())
}
