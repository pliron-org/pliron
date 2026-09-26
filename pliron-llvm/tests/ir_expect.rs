// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! General tests that expect a known (golden) IR.

#![cfg(feature = "llvm-sys")]

use expect_test::expect;
use pliron::{
    builtin::ops::ModuleOp,
    context::Context,
    init_env_logger_for_tests,
    result::{Error, Result},
};
use pliron_llvm::{
    attributes::ConstAggregateVerifyErr,
    from_llvm_ir,
    llvm_sys::core::LLVMContext,
    ops::{ConstantOpVerifyErr, SelectOpVerifyErr},
};
mod common;

/// Parse `input` as pliron IR, run the O1 pipeline over it, round-trip the result
/// through the printer and parser, and return the LLVM-IR it lowers to.
fn to_llvm_ir_o1(input: &str) -> Result<String> {
    init_env_logger_for_tests!();

    let ctx = &mut Context::new();
    let module_op = common::parse_op_verify(ctx, input)?;

    common::run_o1_passes_verify(ctx, module_op)?;
    let (_, reparsed) = common::print_parse_verify(ctx, module_op)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, reparsed)?;
    Ok(llvm_mod.to_string())
}

#[test]
fn data_layout_target_triple() -> Result<()> {
    // `-A5` makes LLVM's C-API allocate in address space 5.
    // `@foo`'s `llvm.ptr (5)` alloca lowers directly, while
    // `@bar`'s `llvm.ptr (0)` needs an address space cast.
    let input = r#"
        builtin.module @m
          [llvm_data_layout: builtin.string "e-p:64:64-A5",
           llvm_target_triple: builtin.string "amdgcn-amd-amdhsa"] {
        ^block_0_0():
          llvm.func @foo: llvm.func <llvm.void(builtin.integer i32) variadic = false> [] {
          ^entry_block_1_0(n: builtin.integer i32):
            ptr = llvm.alloca [builtin.integer i32 x n] : llvm.ptr (5);
            llvm.return
          };
          llvm.func @bar: llvm.func <llvm.void(builtin.integer i32) variadic = false> [] {
          ^entry_block_2_0(n: builtin.integer i32):
            ptr = llvm.alloca [builtin.integer i32 x n] : llvm.ptr (0);
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

        define void @bar(i32 %0) {
        entry_block_2_0_block3v1:
          %ptr_v3 = alloca i32, i32 %0, align 4, addrspace(5)
          %ptr_v3.ascast = addrspacecast ptr addrspace(5) %ptr_v3 to ptr
          ret void
        }
    "#]]
    .assert_eq(&llvm_mod.to_string());
    assert_eq!(llvm_mod.data_layout(), "e-p:64:64-A5");
    assert_eq!(llvm_mod.target_triple(), "amdgcn-amd-amdhsa");

    let ctx2 = &mut Context::new();
    let module_op2 = from_llvm_ir::convert_module(ctx2, &llvm_mod)?;
    let (printed, reparsed) = common::print_parse_verify(ctx2, module_op2)?;
    expect![[r#"
        builtin.module @m 
          [llvm_data_layout: builtin.string "e-p:64:64-A5", llvm_target_triple: builtin.string "amdgcn-amd-amdhsa"]
        {
          ^block1v1():
            llvm.func @foo: llvm.func <llvm.void (builtin.integer i32) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: builtin.integer i32):
                ptr_v1_v1 = llvm.alloca [builtin.integer i32 x v0] [align : 4] : llvm.ptr (5) !0;
                llvm.return 
            };
            llvm.func @bar: llvm.func <llvm.void (builtin.integer i32) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block3v1(v2: builtin.integer i32):
                ptr_v3_v3 = llvm.alloca [builtin.integer i32 x v2] [align : 4] : llvm.ptr (5) !1;
                ptr_v3_ascast_v4 = llvm.addrspacecast ptr_v3_v3 to llvm.ptr (0) !2;
                llvm.return 
            }
        }

        outlined_attributes:
        !0 = [builtin_given_names = builtin.given_names [ptr_v1]]
        !1 = [builtin_given_names = builtin.given_names [ptr_v3]]
        !2 = [builtin_given_names = builtin.given_names [ptr_v3_ascast]]
    "#]].assert_eq(&printed);
    assert_eq!(
        pliron_llvm::attributes::get_data_layout(ctx2, reparsed).as_deref(),
        Some("e-p:64:64-A5")
    );
    assert_eq!(
        pliron_llvm::attributes::get_target_triple(ctx2, reparsed).as_deref(),
        Some("amdgcn-amd-amdhsa")
    );

    Ok(())
}

/// Constructs authored directly in pliron's IR and checked for lowering to LLVM-IR.
/// Typically tests come here when they don't have a `from_llvm_ir` conversion but can
/// be represented in the dialect.
#[test]
fn pliron_ir_lowers_to_llvm_ir() -> Result<()> {
    let scalable_i32 = "llvm.vector <Scalable x 4 x builtin.integer i32>";
    let input = format!(
        r#"
        builtin.module @m {{
        ^block_0_0():
          llvm.func @splat: llvm.func <{scalable_i32}() variadic = false> [] {{
          ^entry_block_1_0():
            c = llvm.constant <llvm.splat <builtin.integer <7: i32> : {scalable_i32}>> : {scalable_i32};
            llvm.return c
          }}
        }}
    "#
    );

    let after = to_llvm_ir_o1(&input)?;

    expect![[r#"
        ; ModuleID = 'm'
        source_filename = "m"

        define <vscale x 4 x i32> @splat() {
        entry_block_1_0_block2v1_block4v1:
          ret <vscale x 4 x i32> splat (i32 7)
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

/// LLVM-IR -> pliron -> pliron text -> pliron -> LLVM-IR:
///
/// Details such as flags and attributes that `compile_run.rs` can't test easily.
#[test]
fn llvm_ir_instruction_flags_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
        define float @choose(i1 %c, float %a, float %b) {
        entry:
          %r = select nnan nsz i1 %c, float %a, float %b
          ret float %r
        }

        define float @choose_plain(i1 %c, float %a, float %b) {
        entry:
          %r = select i1 %c, float %a, float %b
          ret float %r
        }

        define ptr @gep_flags(ptr %p, i64 %i) {
        entry:
          %plain = getelementptr i8, ptr %p, i64 %i
          %nusw = getelementptr nusw i8, ptr %p, i64 %i
          %nuw = getelementptr nuw i8, ptr %p, i64 %i
          %inbounds = getelementptr inbounds i8, ptr %p, i64 %i
          %both = getelementptr inbounds nuw i8, ptr %p, i64 %i
          ret ptr %both
        }

        define void @syncscopes(ptr %p) {
        entry:
          fence syncscope("device") seq_cst
          fence syncscope("singlethread") seq_cst
          fence seq_cst
          %v = load atomic i32, ptr %p syncscope("agent") seq_cst, align 4
          store atomic i32 %v, ptr %p syncscope("block") seq_cst, align 4
          ret void
        }

        define i32 @volatile_rw(ptr %p, i32 %x) {
        entry:
          %v = load volatile i32, ptr %p, align 4
          store volatile i32 %x, ptr %p, align 4
          %plain = load i32, ptr %p, align 4
          store i32 %plain, ptr %p, align 4
          ret i32 %v
        }

        declare void @die() noreturn nounwind
        declare void @llvm.trap()

        define void @attributes(i32 %x) convergent alignstack(16) "no-jump-tables" "target-cpu"="x86-64" {
        entry:
          call void @die() noreturn nounwind
          call void @llvm.trap() cold nounwind
          ret void
        }

        ; `memory(none)` is an integer attribute whose value is zero. 
        ; The value-kind table must tell it apart from an attribute that takes no value.
        define void @zero_valued_int_attr() memory(none) {
        entry:
          ret void
        }

        define i32 @inline_asm(i32 %a, i32 %b, ptr %p) {
        entry:
          call void asm sideeffect "nop", ""() convergent nounwind
          call void asm "nop", ""()
          %r = call i32 asm "add $1, $2, $0", "=r,r,r"(i32 %a, i32 %b)
          %pair = call { i32, i32 } asm "nop", "=r,=r,r"(i32 %r)
          call void asm sideeffect "str $0, [$1]", "r,r"(i32 %r, ptr %p)
          ret i32 %r
        }
    "#;

    let llvm_ctx = LLVMContext::default();
    let ctx = &mut Context::new();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "instruction_flags")?;

    let (printed, reparsed) = common::print_parse_verify(ctx, module_op)?;
    expect![[r#"
        builtin.module @instruction_flags 
        {
          ^block1v1():
            llvm.func @choose: llvm.func <builtin.fp32 (builtin.integer i1, builtin.fp32 , builtin.fp32 ) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: builtin.integer i1, v1: builtin.fp32 , v2: builtin.fp32 ):
                r_v3 = llvm.select <NNAN | NSZ> v0 ? v1 : v2 : builtin.fp32  !0;
                llvm.return r_v3
            };
            llvm.func @choose_plain: llvm.func <builtin.fp32 (builtin.integer i1, builtin.fp32 , builtin.fp32 ) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block3v1(v4: builtin.integer i1, v5: builtin.fp32 , v6: builtin.fp32 ):
                r_v7 = llvm.select  v4 ? v5 : v6 : builtin.fp32  !1;
                llvm.return r_v7
            };
            llvm.func @gep_flags: llvm.func <llvm.ptr (0)(llvm.ptr (0), builtin.integer i64) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block4v1(v8: llvm.ptr (0), v9: builtin.integer i64):
                plain_v10 = llvm.gep <builtin.integer i8> (v8, v9)[OperandIdx(1)] : llvm.ptr (0) !2;
                nusw_v11 = llvm.gep <builtin.integer i8> (v8, v9)<NUSW>[OperandIdx(1)] : llvm.ptr (0) !3;
                nuw_v12 = llvm.gep <builtin.integer i8> (v8, v9)<NUW>[OperandIdx(1)] : llvm.ptr (0) !4;
                inbounds_v13 = llvm.gep <builtin.integer i8> (v8, v9)<INBOUNDS>[OperandIdx(1)] : llvm.ptr (0) !5;
                both_v14 = llvm.gep <builtin.integer i8> (v8, v9)<INBOUNDS | NUW>[OperandIdx(1)] : llvm.ptr (0) !6;
                llvm.return both_v14
            };
            llvm.func @syncscopes: llvm.func <llvm.void (llvm.ptr (0)) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block5v1(v15: llvm.ptr (0)):
                llvm.fence syncscope : NamedScope("device") SeqCst;
                llvm.fence syncscope : SingleThread SeqCst;
                llvm.fence syncscope : System SeqCst;
                v_v16 = llvm.atomic_load v15 [align : 4] syncscope : NamedScope("agent") SeqCst : builtin.integer i32 !7;
                llvm.atomic_store *v15 <- v_v16 [align : 4] syncscope : NamedScope("block") SeqCst;
                llvm.return 
            };
            llvm.func @volatile_rw: llvm.func <builtin.integer i32(llvm.ptr (0), builtin.integer i32) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block6v1(v17: llvm.ptr (0), v18: builtin.integer i32):
                v_v19 = llvm.load v17 [volatile : true][align : 4] : builtin.integer i32 !8;
                llvm.store *v17 <- v18 [volatile : true][align : 4];
                plain_v20 = llvm.load v17 [align : 4] : builtin.integer i32 !9;
                llvm.store *v17 <- plain_v20 [align : 4];
                llvm.return v_v19
            };
            llvm.func @die: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] !10;
            llvm.func @attributes: llvm.func <llvm.void (builtin.integer i32) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block7v1(v21: builtin.integer i32):
                v22 = llvm.call @die () : llvm.func <llvm.void () variadic = false> !11;
                v23 = llvm.call_intrinsic @"llvm.trap" () : llvm.func <llvm.void () variadic = false> !12;
                llvm.return 
            } !13;
            llvm.func @zero_valued_int_attr: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block8v1():
                llvm.return 
            } !14;
            llvm.func @inline_asm: llvm.func <builtin.integer i32(builtin.integer i32, builtin.integer i32, llvm.ptr (0)) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block9v1(v24: builtin.integer i32, v25: builtin.integer i32, v26: llvm.ptr (0)):
                v27 = llvm.inline_asm "nop", "" side_effects = true attrs : !outlined () : llvm.void  !15;
                v28 = llvm.inline_asm "nop", "" side_effects = false  () : llvm.void ;
                r_v29 = llvm.inline_asm "add $1, $2, $0", "=r,r,r" side_effects = false  (v24, v25) : builtin.integer i32 !16;
                pair_v30 = llvm.inline_asm "nop", "=r,=r,r" side_effects = false  (r_v29) : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> !17;
                v31 = llvm.inline_asm "str $0, [$1]", "r,r" side_effects = true  (r_v29, v26) : llvm.void ;
                llvm.return r_v29
            }
        }

        outlined_attributes:
        !0 = [builtin_given_names = builtin.given_names [r]]
        !1 = [builtin_given_names = builtin.given_names [r]]
        !2 = [builtin_given_names = builtin.given_names [plain]]
        !3 = [builtin_given_names = builtin.given_names [nusw]]
        !4 = [builtin_given_names = builtin.given_names [nuw]]
        !5 = [builtin_given_names = builtin.given_names [inbounds]]
        !6 = [builtin_given_names = builtin.given_names [both]]
        !7 = [builtin_given_names = builtin.given_names [v]]
        !8 = [builtin_given_names = builtin.given_names [v]]
        !9 = [builtin_given_names = builtin.given_names [plain]]
        !10 = [llvm_func_attrs = llvm.attributes ["noreturn", "nounwind"]]
        !11 = [llvm_call_attrs = llvm.attributes ["noreturn", "nounwind"]]
        !12 = [llvm_intrinsic_attrs = llvm.attributes ["cold", "nounwind"]]
        !13 = [llvm_func_attrs = llvm.attributes ["convergent", "alignstack" = 16, "no-jump-tables" = "", "target-cpu" = "x86-64"]]
        !14 = [llvm_func_attrs = llvm.attributes ["memory" = 0]]
        !15 = [llvm_inline_asm_attrs = llvm.attributes ["convergent", "nounwind"]]
        !16 = [builtin_given_names = builtin.given_names [r]]
        !17 = [builtin_given_names = builtin.given_names [pair]]
    "#]].assert_eq(&printed);

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, reparsed)?;
    expect![[r#"
        ; ModuleID = 'instruction_flags'
        source_filename = "instruction_flags"

        define float @choose(i1 %0, float %1, float %2) {
        entry_block2v1_block11v1:
          %r_v35 = select nnan nsz i1 %0, float %1, float %2
          ret float %r_v35
        }

        define float @choose_plain(i1 %0, float %1, float %2) {
        entry_block3v1_block12v1:
          %r_v39 = select i1 %0, float %1, float %2
          ret float %r_v39
        }

        define ptr @gep_flags(ptr %0, i64 %1) {
        entry_block4v1_block13v1:
          %plain_v42 = getelementptr i8, ptr %0, i64 %1
          %nusw_v43 = getelementptr nusw i8, ptr %0, i64 %1
          %nuw_v44 = getelementptr nuw i8, ptr %0, i64 %1
          %inbounds_v45 = getelementptr inbounds i8, ptr %0, i64 %1
          %both_v46 = getelementptr inbounds nuw i8, ptr %0, i64 %1
          ret ptr %both_v46
        }

        define void @syncscopes(ptr %0) {
        entry_block5v1_block14v1:
          fence syncscope("device") seq_cst
          fence syncscope("singlethread") seq_cst
          fence seq_cst
          %v_v48 = load atomic i32, ptr %0 syncscope("agent") seq_cst, align 4
          store atomic i32 %v_v48, ptr %0 syncscope("block") seq_cst, align 4
          ret void
        }

        define i32 @volatile_rw(ptr %0, i32 %1) {
        entry_block6v1_block15v1:
          %v_v51 = load volatile i32, ptr %0, align 4
          store volatile i32 %1, ptr %0, align 4
          %plain_v52 = load i32, ptr %0, align 4
          store i32 %plain_v52, ptr %0, align 4
          ret i32 %v_v51
        }

        ; Function Attrs: noreturn nounwind
        declare void @die() #0

        ; Function Attrs: convergent alignstack(16)
        define void @attributes(i32 %0) #1 {
        entry_block7v1_block16v1:
          call void @die() #0
          call void @llvm.trap() #4
          ret void
        }

        ; Function Attrs: memory(none)
        define void @zero_valued_int_attr() #2 {
        entry_block8v1_block17v1:
          ret void
        }

        define i32 @inline_asm(i32 %0, i32 %1, ptr %2) {
        entry_block9v1_block18v1:
          call void asm sideeffect "nop", ""() #5
          call void asm "nop", ""()
          %r_v61 = call i32 asm "add $1, $2, $0", "=r,r,r"(i32 %0, i32 %1)
          %pair_v62 = call { i32, i32 } asm "nop", "=r,=r,r"(i32 %r_v61)
          call void asm sideeffect "str $0, [$1]", "r,r"(i32 %r_v61, ptr %2)
          ret i32 %r_v61
        }

        ; Function Attrs: cold noreturn nounwind memory(inaccessiblemem: write)
        declare void @llvm.trap() #3

        attributes #0 = { noreturn nounwind }
        attributes #1 = { convergent alignstack=16 "no-jump-tables" "target-cpu"="x86-64" }
        attributes #2 = { memory(none) }
        attributes #3 = { cold noreturn nounwind memory(inaccessiblemem: write) }
        attributes #4 = { cold nounwind }
        attributes #5 = { convergent nounwind }
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

/// Constant aggregates, vector splats, and symbol addresses as global initializers.
#[test]
fn constant_aggregates_and_splats_roundtrip() -> Result<()> {
    init_env_logger_for_tests!();
    let input = r#"
        @g = global i32 0
        @g_const = constant i32 1
        @array = global [4 x i32] [i32 1, i32 2, i32 3, i32 4]
        @struct = global { i32, float, ptr } { i32 1, float 2.0, ptr @g }
        @nested = global [2 x { i32, i32 }] [{ i32, i32 } { i32 1, i32 2 },
                                             { i32, i32 } { i32 3, i32 4 }]
        @vtable = global [2 x ptr] [ptr @g, ptr @f]
        @addr = global ptr @g_const
        @string = global [9 x i8] c"longtext\00"
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

    // The printed constants must parse back and verify.
    let (printed, reparsed) = common::print_parse_verify(ctx, module_op)?;
    expect![[r#"
        builtin.module @constants 
        {
          ^block1v1():
            llvm.global @g : builtin.integer i32
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = builtin.integer <0: i32>;
            llvm.global @g_const : builtin.integer i32
              [llvm_global_linkage: llvm.linkage ExternalLinkage, llvm_global_constant: builtin.bool true] = builtin.integer <1: i32>;
            llvm.global @array : llvm.array [4 x builtin.integer i32]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = !outlined !0;
            llvm.global @struct : llvm.struct <{ builtin.integer i32, builtin.fp32 , llvm.ptr (0) } : Unpacked>
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[builtin.integer <1: i32>, builtin.single 2, llvm.symbol_addr <@g : llvm.ptr (0)>] : llvm.struct <{ builtin.integer i32, builtin.fp32 , llvm.ptr (0) } : Unpacked>>;
            llvm.global @nested : llvm.array [2 x llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>, llvm.aggregate <[builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>] : llvm.array [2 x llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>]>;
            llvm.global @vtable : llvm.array [2 x llvm.ptr (0)]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.aggregate <[llvm.symbol_addr <@g : llvm.ptr (0)>, llvm.symbol_addr <@f : llvm.ptr (0)>] : llvm.array [2 x llvm.ptr (0)]>;
            llvm.global @addr : llvm.ptr (0)
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.symbol_addr <@g_const : llvm.ptr (0)>;
            llvm.global @string : llvm.array [9 x builtin.integer i8]
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = !outlined !1;
            llvm.global @vector : llvm.vector <Fixed x 4 x builtin.integer i32>
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = !outlined !2;
            llvm.global @splat : llvm.vector <Fixed x 4 x builtin.integer i32>
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 4 x builtin.integer i32>>;
            llvm.func @f: llvm.func <llvm.void (llvm.ptr (0)) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: llvm.ptr (0)):
                v1 = llvm.constant <!outlined> : llvm.array [4 x builtin.integer i32] !3;
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
        }

        outlined_attributes:
        !0 = [llvm_global_initializer = !4]
        !1 = [llvm_global_initializer = !5]
        !2 = [llvm_global_initializer = !6]
        !3 = [llvm_constant_value = !4]
        !4 = llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.array [4 x builtin.integer i32]>
        !5 = llvm.bytes [108, 111, 110, 103, 116, 101, 120, 116, 0]
        !6 = llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>, builtin.integer <4: i32>] : llvm.vector <Fixed x 4 x builtin.integer i32>>
    "#]].assert_eq(&printed);

    let out_llvm_ctx = LLVMContext::default();
    let out_mod = common::to_llvm_ir_verify(ctx, &out_llvm_ctx, reparsed)?;
    expect![[r#"
        ; ModuleID = 'constants'
        source_filename = "constants"

        @g = global i32 0
        @g_const = constant i32 1
        @array = global [4 x i32] [i32 1, i32 2, i32 3, i32 4]
        @struct = global { i32, float, ptr } { i32 1, float 2.000000e+00, ptr @g }
        @nested = global [2 x { i32, i32 }] [{ i32, i32 } { i32 1, i32 2 }, { i32, i32 } { i32 3, i32 4 }]
        @vtable = global [2 x ptr] [ptr @g, ptr @f]
        @addr = global ptr @g_const
        @string = global [9 x i8] c"longtext\00"
        @vector = global <4 x i32> <i32 1, i32 2, i32 3, i32 4>
        @splat = global <4 x i32> splat (i32 7)

        define void @f(ptr %0) {
        entry_block2v1_block6v1:
          store [4 x i32] [i32 1, i32 2, i32 3, i32 4], ptr %0, align 4
          store { i32, ptr } { i32 5, ptr @g }, ptr %0, align 8
          store [6 x i8] c"hello\00", ptr %0, align 1
          ret void
        }

        define <4 x i32> @fixed() {
        entry_block3v1_block7v1:
          ret <4 x i32> splat (i32 7)
        }

        define <4 x float> @fp() {
        entry_block4v1_block8v1:
          ret <4 x float> splat (float 2.500000e+00)
        }
    "#]].assert_eq(&out_mod.to_string());

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
        TypedHandle mismatch: expected llvm.vector but provided llvm.array
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
