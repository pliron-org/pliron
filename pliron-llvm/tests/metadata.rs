// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for LLVM metadata: conversion from LLVM-IR, printing / parsing, and
//! conversion back to LLVM-IR.

#![cfg(feature = "llvm-sys")]

use core::num::NonZero;

use expect_test::expect;
use pliron::{
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::SingleBlockRegionInterface,
        ops::ModuleOp,
        types::{IntegerType, Signedness},
    },
    context::{Context, Ptr},
    graph::walkers::{self, IRNode, WALKCONFIG_PREORDER_FORWARD},
    init_env_logger_for_tests,
    linked_list::ContainsLinkedList,
    op::Op,
    operation::Operation,
    printable::Printable,
    result::{Error, Result},
    utils::apint::APInt,
};
use pliron_llvm::{
    llvm_sys::core::LLVMContext,
    metadata::{
        MdNodeAttr, MdOperandAttr, MetadataVerifyErr, attach_new_metadata, get_attachments,
        get_metadata_table, get_named_metadata, verify_metadata,
    },
    metadata_conversions::to_llvm_ir::MdToLLVMErr,
    ops::{FuncOp, StoreOp},
    to_llvm_ir,
};

mod common;

/// A loop with TBAA, alias scopes and loop metadata, as `clang -O2` produces it,
/// plus a custom metadata kind, named metadata and a metadata node referring to a
/// global.
const METADATA_LL: &str = r#"
  @g = global i32 0

  define void @run(ptr %d, ptr %s, i32 %n) {
  entry:
    br label %loop

  loop:
    %i = phi i64 [ 0, %entry ], [ %i.next, %loop ]
    %sp = getelementptr inbounds i32, ptr %s, i64 %i
    %v = load i32, ptr %sp, align 4, !tbaa !5, !alias.scope !12, !noalias !9
    %inc = add nsw i32 %v, 1
    %dp = getelementptr inbounds i32, ptr %d, i64 %i
    store i32 %inc, ptr %dp, align 4, !tbaa !5, !alias.scope !9, !noalias !12, !my.custom.kind !17
    %i.next = add nuw nsw i64 %i, 1
    %done = icmp eq i64 %i.next, 16
    br i1 %done, label %exit, label %loop, !llvm.loop !14

  exit:
    ret void
  }

  !llvm.module.flags = !{!0, !1}
  !llvm.ident = !{!4}

  !0 = !{i32 1, !"wchar_size", i32 4}
  !1 = !{i32 7, !"uwtable", i32 2}
  !4 = !{!"pliron test"}
  !5 = !{!6, !6, i64 0}
  !6 = !{!"int", !7, i64 0}
  !7 = !{!"omnipotent char", !8, i64 0}
  !8 = !{!"Simple C/C++ TBAA"}
  !9 = !{!10}
  !10 = distinct !{!10, !11, !"copy: argument 0"}
  !11 = distinct !{!11, !"copy"}
  !12 = !{!13}
  !13 = distinct !{!13, !11, !"copy: argument 1"}
  !14 = distinct !{!14, !15, !16}
  !15 = !{!"llvm.loop.mustprogress"}
  !16 = !{!"llvm.loop.unroll.disable"}
  !17 = !{!"custom", ptr @g, null, i64 42}
"#;

/// Convert LLVM-IR text into a pliron [ModuleOp] and print it.
fn from_llvm_ir(ctx: &mut Context, input: &str) -> Result<(ModuleOp, String)> {
    init_env_logger_for_tests!();
    let llvm_ctx = LLVMContext::default();
    let module_op = common::parse_llvm_ir_verify(ctx, &llvm_ctx, input, "metadata_test")?;
    verify_metadata(ctx, module_op)?;
    let printed = module_op.get_operation().disp(ctx).to_string();
    Ok((module_op, printed))
}

#[test]
fn metadata_from_llvm_ir() -> Result<()> {
    let ctx = &mut Context::new();
    let (_, printed) = from_llvm_ir(ctx, METADATA_LL)?;
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.global @g : builtin.integer i32
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = builtin.integer <0: i32>;
            llvm.func @run: llvm.func <llvm.void (llvm.ptr (0), llvm.ptr (0), builtin.integer i32) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: llvm.ptr (0), v1: llvm.ptr (0), v2: builtin.integer i32):
                v3 = llvm.constant <builtin.integer <0: i64>> : builtin.integer i64;
                v7 = llvm.constant <builtin.integer <1: i32>> : builtin.integer i32;
                v10 = llvm.constant <builtin.integer <1: i64>> : builtin.integer i64;
                v12 = llvm.constant <builtin.integer <16: i64>> : builtin.integer i64;
                llvm.br ^loop_block3v1(v3)

              ^loop_block3v1(v4: builtin.integer i64):
                sp_v5 = llvm.gep <builtin.integer i32> (v1, v4)<INBOUNDS>[OperandIdx(1)] : llvm.ptr (0) !0;
                v_v6 = llvm.load sp_v5 [align : 4] : builtin.integer i32 !1;
                inc_v8 = llvm.add v_v6, v7 <{nsw=true,nuw=false}>: builtin.integer i32 !2;
                dp_v9 = llvm.gep <builtin.integer i32> (v0, v4)<INBOUNDS>[OperandIdx(1)] : llvm.ptr (0) !3;
                llvm.store *dp_v9 <- inc_v8 [align : 4] !4;
                i_next_v11 = llvm.add v4, v10 <{nsw=true,nuw=true}>: builtin.integer i64 !5;
                done_v13 = llvm.icmp i_next_v11 <EQ> v12 : builtin.integer i1 !6;
                llvm.cond_br if done_v13 ^exit_block4v1() else ^loop_block3v1(i_next_v11) !7

              ^exit_block4v1():
                llvm.return 
            }
        } !8

        outlined_attributes:
        !0 = [builtin_given_names = builtin.given_names [sp]]
        !1 = [llvm_metadata = llvm.md_attachments ["tbaa" = #0, "alias.scope" = #4, "noalias" = #7], builtin_given_names = builtin.given_names [v]]
        !2 = [builtin_given_names = builtin.given_names [inc]]
        !3 = [builtin_given_names = builtin.given_names [dp]]
        !4 = [llvm_metadata = llvm.md_attachments ["tbaa" = #0, "alias.scope" = #7, "noalias" = #4, "my.custom.kind" = #9]]
        !5 = [builtin_given_names = builtin.given_names [i_next]]
        !6 = [builtin_given_names = builtin.given_names [done]]
        !7 = [llvm_metadata = llvm.md_attachments ["llvm.loop" = #10]]
        !8 = [llvm_named_metadata = llvm.named_md ["llvm.module.flags" = [#13, #14], "llvm.ident" = [#15]], llvm_metadata_defs = llvm.md_table [
          #0 = !{#1, #1, builtin.integer <0: i64>},
          #1 = !{!"int", #2, builtin.integer <0: i64>},
          #2 = !{!"omnipotent char", #3, builtin.integer <0: i64>},
          #3 = !{!"Simple C/C++ TBAA"},
          #4 = !{#5},
          #5 = distinct !{#5, #6, !"copy: argument 1"},
          #6 = distinct !{#6, !"copy"},
          #7 = !{#8},
          #8 = distinct !{#8, #6, !"copy: argument 0"},
          #9 = !{!"custom", @g, null, builtin.integer <42: i64>},
          #10 = distinct !{#10, #11, #12},
          #11 = !{!"llvm.loop.mustprogress"},
          #12 = !{!"llvm.loop.unroll.disable"},
          #13 = !{builtin.integer <1: i32>, !"wchar_size", builtin.integer <4: i32>},
          #14 = !{builtin.integer <7: i32>, !"uwtable", builtin.integer <2: i32>},
          #15 = !{!"pliron test"}
        ]]
    "#]].assert_eq(&printed);
    Ok(())
}

/// Every metadata attachment in the module, in walk order, as printed.
fn collect_attachments(ctx: &Context, module: Ptr<Operation>) -> Vec<String> {
    let mut attachments = vec![];
    walkers::uninterruptible::immutable::walk_op(
        ctx,
        &mut attachments,
        &WALKCONFIG_PREORDER_FORWARD,
        module,
        |ctx: &Context, attachments: &mut Vec<String>, node: IRNode| {
            if let IRNode::Operation(op) = node
                && let Some(attached) = get_attachments(ctx, op)
            {
                attachments.push(attached.disp(ctx).to_string());
            }
        },
    );
    attachments
}

#[test]
fn metadata_pliron_ir_roundtrips() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, printed) = from_llvm_ir(ctx, METADATA_LL)?;

    // Parse back what we printed. pliron's printer adds source locations and renames
    // values on a re-parse, so compare the metadata itself rather than the full text.
    let ctx2 = &mut Context::new();
    let reparsed = common::parse_op_verify::<ModuleOp>(ctx2, &printed)?;
    verify_metadata(ctx2, reparsed)?;

    assert_eq!(
        get_metadata_table(ctx, module_op),
        get_metadata_table(ctx2, reparsed)
    );
    assert_eq!(
        get_named_metadata(ctx, module_op),
        get_named_metadata(ctx2, reparsed)
    );
    assert_eq!(
        collect_attachments(ctx, module_op.get_operation()),
        collect_attachments(ctx2, reparsed.get_operation())
    );

    Ok(())
}

#[test]
fn metadata_to_llvm_ir() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _) = from_llvm_ir(ctx, METADATA_LL)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        @g = global i32 0

        define void @run(ptr %0, ptr %1, i32 %2) {
        entry_block2v1:
          br label %loop_block3v1

        loop_block3v1:                                    ; preds = %loop_block3v1, %entry_block2v1
          %v4 = phi i64 [ 0, %entry_block2v1 ], [ %i_next_v11, %loop_block3v1 ]
          %sp_v5 = getelementptr inbounds i32, ptr %1, i64 %v4
          %v_v6 = load i32, ptr %sp_v5, align 4, !tbaa !3, !alias.scope !7, !noalias !10
          %inc_v8 = add i32 %v_v6, 1
          %dp_v9 = getelementptr inbounds i32, ptr %0, i64 %v4
          store i32 %inc_v8, ptr %dp_v9, align 4, !tbaa !3, !alias.scope !10, !noalias !7, !my.custom.kind !12
          %i_next_v11 = add i64 %v4, 1
          %done_v13 = icmp eq i64 %i_next_v11, 16
          br i1 %done_v13, label %exit_block4v1, label %loop_block3v1, !llvm.loop !13

        exit_block4v1:                                    ; preds = %loop_block3v1
          ret void
        }

        !llvm.module.flags = !{!0, !1}
        !llvm.ident = !{!2}

        !0 = !{i32 1, !"wchar_size", i32 4}
        !1 = !{i32 7, !"uwtable", i32 2}
        !2 = !{!"pliron test"}
        !3 = !{!4, !4, i64 0}
        !4 = !{!"int", !5, i64 0}
        !5 = !{!"omnipotent char", !6, i64 0}
        !6 = !{!"Simple C/C++ TBAA"}
        !7 = !{!8}
        !8 = distinct !{!8, !9, !"copy: argument 1"}
        !9 = distinct !{!9, !"copy"}
        !10 = !{!11}
        !11 = distinct !{!11, !9, !"copy: argument 0"}
        !12 = !{!"custom", ptr @g, null, i64 42}
        !13 = distinct !{!13, !14, !15}
        !14 = !{!"llvm.loop.mustprogress"}
        !15 = !{!"llvm.loop.unroll.disable"}
    "#]].assert_eq(&llvm_mod.to_string());
    Ok(())
}

/// Debug info metadata cannot be represented, so it is dropped (with a warning) and
/// everything else about the module still converts.
#[test]
fn debug_info_metadata_is_dropped() -> Result<()> {
    let input = r#"
      @g = global i32 0

      define void @f(i32 %n) !dbg !4 {
        %a = add i32 %n, 1, !my.kind !7
        ret void
      }

      !llvm.dbg.cu = !{!0}
      !llvm.module.flags = !{!3}

      !0 = distinct !DICompileUnit(language: DW_LANG_C11, file: !1, producer: "pliron test", emissionKind: FullDebug)
      !1 = !DIFile(filename: "t.c", directory: "/")
      !3 = !{i32 2, !"Debug Info Version", i32 3}
      !4 = distinct !DISubprogram(name: "f", scope: !1, file: !1, line: 1, type: !5, spFlags: DISPFlagDefinition, unit: !0)
      !5 = !DISubroutineType(types: !6)
      !6 = !{null}
      !7 = !{!"a tuple that survives", !1}
    "#;

    let ctx = &mut Context::new();
    let (module_op, printed) = from_llvm_ir(ctx, input)?;

    // The generic node referring to a `DIFile` goes with it.
    // `!dbg` and `!llvm.dbg.cu` are gone entirely too.
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.global @g : builtin.integer i32
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = builtin.integer <0: i32>;
            llvm.func @f: llvm.func <llvm.void (builtin.integer i32) variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1(v0: builtin.integer i32):
                v1 = llvm.constant <builtin.integer <1: i32>> : builtin.integer i32;
                a_v2 = llvm.add v0, v1 <{nsw=false,nuw=false}>: builtin.integer i32 !0;
                llvm.return 
            }
        } !1

        outlined_attributes:
        !0 = [builtin_given_names = builtin.given_names [a]]
        !1 = [llvm_named_metadata = llvm.named_md ["llvm.module.flags" = [#1]], llvm_metadata_defs = llvm.md_table [
          #0 = !{},
          #1 = !{builtin.integer <2: i32>, !"Debug Info Version", builtin.integer <3: i32>}
        ]]
    "#]].assert_eq(&printed);
    assert_eq!(
        get_named_metadata(ctx, module_op)
            .and_then(|named| named.get("llvm.dbg.cu").map(<[u32]>::to_vec)),
        None
    );

    // What is left must still convert back to valid LLVM-IR.
    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        @g = global i32 0

        define void @f(i32 %0) {
        entry_block2v1:
          %a_v2 = add i32 %0, 1
          ret void
        }

        !llvm.module.flags = !{!0}

        !0 = !{i32 2, !"Debug Info Version", i32 3}
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}

/// Metadata attached to a global variable and to a function (LLVM's `GlobalObject`
/// metadata) rather than to an instruction.
#[test]
fn global_object_metadata() -> Result<()> {
    let input = r#"
      @g = global i32 0, !absolute_symbol !0

      define void @f() !prof !1 {
        ret void
      }

      !0 = !{i64 0, i64 256}
      !1 = !{!"function_entry_count", i64 100}
    "#;

    let ctx = &mut Context::new();
    let (module_op, _) = from_llvm_ir(ctx, input)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        @g = global i32 0, !absolute_symbol !0

        define void @f() !prof !1 {
        entry_block2v1:
          ret void
        }

        !0 = !{i64 0, i64 256}
        !1 = !{!"function_entry_count", i64 100}
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}

/// A `distinct` node that doesn't refer to itself round-trips through pliron, but
/// cannot be re-created through the LLVM C-API, which has no way to ask for a distinct
/// node. Rather than silently handing LLVM a uniqued node with the wrong identity, the
/// conversion fails.
#[test]
fn non_self_referential_distinct_metadata_is_rejected() -> Result<()> {
    let input = r#"
      define void @f() {
        ret void, !my.kind !0
      }

      !0 = distinct !{i32 1}
    "#;

    let ctx = &mut Context::new();
    let (module_op, printed) = from_llvm_ir(ctx, input)?;
    // The node's distinctness must survive the import.
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.func @f: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1():
                llvm.return  !0
            }
        } !1

        outlined_attributes:
        !0 = [llvm_metadata = llvm.md_attachments ["my.kind" = #0]]
        !1 = [llvm_metadata_defs = llvm.md_table [
          #0 = distinct !{builtin.integer <1: i32>}
        ]]
    "#]]
    .assert_eq(&printed);

    let llvm_ctx = LLVMContext::default();
    let Err(Error { err, .. }) = to_llvm_ir::convert_module(ctx, &llvm_ctx, module_op) else {
        panic!("a distinct node that isn't self referential must be rejected");
    };
    let err = err.downcast_ref::<MdToLLVMErr>().unwrap();
    assert!(matches!(err, MdToLLVMErr::UnrepresentableDistinct(0)));
    Ok(())
}

/// A reference to a metadata node that isn't in the module's table is caught by
/// [verify_metadata].
#[test]
fn dangling_metadata_reference_is_caught() -> Result<()> {
    let input = r#"
        builtin.module @m {
        ^block_0_0():
          llvm.func @f: llvm.func <llvm.void () variadic = false> [] {
          ^entry_block_1_0():
            llvm.return  !0
          }
        } !1

        outlined_attributes:
        !0 = [llvm_metadata = llvm.md_attachments ["my.kind" = #7]]
        !1 = [llvm_metadata_defs = llvm.md_table [
          #0 = !{!"only node"}
        ]]
    "#;

    let ctx = &mut Context::new();
    let module_op = common::parse_op_verify::<ModuleOp>(ctx, input)?;
    let Err(Error { err, .. }) = verify_metadata(ctx, module_op) else {
        panic!("a dangling metadata reference must be caught");
    };
    let err = err.downcast_ref::<MetadataVerifyErr>().unwrap();
    assert!(matches!(err, MetadataVerifyErr::DanglingNodeRef(7)));
    Ok(())
}

/// An `MDString` may hold arbitrary text; quoting must round-trip it.
#[test]
fn metadata_string_escaping_roundtrips() -> Result<()> {
    let input = r#"
      define void @f() {
        ret void, !my.kind !0
      }

      !0 = !{!"a \22quoted\22 and \5Cbackslash\5C and a\0Anewline"}
    "#;

    let ctx = &mut Context::new();
    let (module_op, printed) = from_llvm_ir(ctx, input)?;

    let ctx2 = &mut Context::new();
    let reparsed = common::parse_op_verify::<ModuleOp>(ctx2, &printed)?;
    assert_eq!(
        get_metadata_table(ctx, module_op),
        get_metadata_table(ctx2, reparsed)
    );

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx2, &llvm_ctx, reparsed)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        define void @f() {
        entry_block2v1_block2v1:
          ret void, !my.kind !0
        }

        !0 = !{!"a \22quoted\22 and \\backslash\\ and a\0Anewline"}
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}

/// An `MDString`'s contents are arbitrary bytes.
/// One that isn't UTF-8 has no pliron counterpart, so the operand is dropped (with a warning)
#[test]
fn non_utf8_metadata_string_is_dropped() -> Result<()> {
    let input = r#"
      define void @f() {
        ret void, !my.kind !0
      }

      !0 = !{!"ok", !"\FF\FE", !"also ok"}
    "#;

    let ctx = &mut Context::new();
    let (_, printed) = from_llvm_ir(ctx, input)?;
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.func @f: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1():
                llvm.return 
            }
        } !0

        outlined_attributes:
        !0 = [llvm_metadata_defs = llvm.md_table [
          #0 = !{}
        ]]
    "#]]
    .assert_eq(&printed);
    Ok(())
}

/// LLVM intrinsics have no counterpart in the pliron module,
/// so a metadata node naming one is dropped (with a warning).
#[test]
fn metadata_referring_to_an_intrinsic_is_dropped() -> Result<()> {
    let input = r#"
      declare void @llvm.donothing()

      define void @f() {
        ret void, !my.kind !0
      }

      !0 = !{!"callee", ptr @llvm.donothing}
    "#;

    let ctx = &mut Context::new();
    let (_, printed) = from_llvm_ir(ctx, input)?;
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.func @f: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1():
                llvm.return 
            }
        } !0

        outlined_attributes:
        !0 = [llvm_metadata_defs = llvm.md_table [
          #0 = !{}
        ]]
    "#]]
    .assert_eq(&printed);
    Ok(())
}

/// An alias has no counterpart in the pliron module, so a metadata node
/// with an alias as an operand is dropped (with a warning).
#[test]
fn unsupported_constant_metadata_operand_is_dropped() -> Result<()> {
    let input = r#"
      @g = global i32 0
      @a = alias i32, ptr @g

      define void @f() {
        ret void, !my.kind !0
      }

      !0 = !{!"aliased", ptr @a}
    "#;

    let ctx = &mut Context::new();
    let (_, printed) = from_llvm_ir(ctx, input)?;
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.global @g : builtin.integer i32
              [llvm_global_linkage: llvm.linkage ExternalLinkage] = builtin.integer <0: i32>;
            llvm.func @f: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1():
                llvm.return 
            }
        } !0

        outlined_attributes:
        !0 = [llvm_metadata_defs = llvm.md_table [
          #0 = !{}
        ]]
    "#]]
    .assert_eq(&printed);
    Ok(())
}

/// macOS clang puts an aggregate constant in the `SDK Version` module flag. A module
/// flag missing its value operand is invalid LLVM-IR, so this has to survive whole.
#[test]
fn module_flag_with_aggregate_constant_roundtrips() -> Result<()> {
    let input = r#"
      define void @f() {
        ret void
      }

      !llvm.module.flags = !{!0, !1}

      !0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 1]}
      !1 = !{i32 1, !"wchar_size", i32 4}
    "#;

    let ctx = &mut Context::new();
    let (module_op, printed) = from_llvm_ir(ctx, input)?;
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.func @f: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1():
                llvm.return 
            }
        } !0

        outlined_attributes:
        !0 = [llvm_named_metadata = llvm.named_md ["llvm.module.flags" = [#0, #1]], llvm_metadata_defs = llvm.md_table [
          #0 = !{builtin.integer <2: i32>, !"SDK Version", llvm.aggregate <[builtin.integer <15: i32>, builtin.integer <1: i32>] : llvm.array [2 x builtin.integer i32]>},
          #1 = !{builtin.integer <1: i32>, !"wchar_size", builtin.integer <4: i32>}
        ]]
    "#]]
    .assert_eq(&printed);

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        define void @f() {
        entry_block2v1:
          ret void
        }

        !llvm.module.flags = !{!0, !1}

        !0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 1]}
        !1 = !{i32 1, !"wchar_size", i32 4}
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}

/// A metadata kind name that LLVM doesn't pre-register is recovered from the module's
/// printed form, where LLVM escapes anything that isn't a name character.
#[test]
fn escaped_metadata_kind_name_roundtrips() -> Result<()> {
    let input = r#"
      define void @f() {
        ret void, !my\20kind !0
      }

      !0 = !{!"contents"}
    "#;

    let ctx = &mut Context::new();
    let (_, printed) = from_llvm_ir(ctx, input)?;
    expect![[r#"
        builtin.module @metadata_test 
        {
          ^block1v1():
            llvm.func @f: llvm.func <llvm.void () variadic = false>
              [llvm_function_linkage: llvm.linkage ExternalLinkage] 
            {
              ^entry_block2v1():
                llvm.return  !0
            }
        } !1

        outlined_attributes:
        !0 = [llvm_metadata = llvm.md_attachments ["my kind" = #0]]
        !1 = [llvm_metadata_defs = llvm.md_table [
          #0 = !{!"contents"}
        ]]
    "#]]
    .assert_eq(&printed);

    let ctx2 = &mut Context::new();
    let reparsed = common::parse_op_verify::<ModuleOp>(ctx2, &printed)?;
    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx2, &llvm_ctx, reparsed)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        define void @f() {
        entry_block2v1_block2v1:
          ret void, !my\20kind !0
        }

        !0 = !{!"contents"}
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}

// A simple test that attaches a `nontemporal` md node to a store.
#[test]
fn non_temporal_store_emits_metadata() -> Result<()> {
    let input = r#"
      define void @stream(ptr %out, <8 x float> %val) {
        store <8 x float> %val, ptr %out, align 4
        ret void
      }
    "#;

    let ctx = &mut Context::new();
    let (module_op, _) = from_llvm_ir(ctx, input)?;
    let func = Operation::get_op::<FuncOp>(
        module_op
            .get_body(ctx, 0)
            .deref(ctx)
            .iter(ctx)
            .next()
            .unwrap(),
        ctx,
    )
    .unwrap();
    let store = func
        .get_entry_block(ctx)
        .unwrap()
        .deref(ctx)
        .iter(ctx)
        .find(|op| Operation::get_op::<StoreOp>(*op, ctx).is_some())
        .expect("@stream has a store");

    let i32_ty = IntegerType::get(ctx, 32, Signedness::Signless);
    let one = IntegerAttr::new(i32_ty, APInt::from_u64(1, NonZero::new(32).unwrap()));
    // `!{i32 1}`, the node `!nontemporal` expects.
    let node = MdNodeAttr::new_tuple(vec![MdOperandAttr::Constant(Box::new(one))]);
    attach_new_metadata(ctx, store, "nontemporal", node)?;
    verify_metadata(ctx, module_op)?;

    let llvm_ctx = LLVMContext::default();
    let llvm_mod = common::to_llvm_ir_verify(ctx, &llvm_ctx, module_op)?;
    expect![[r#"
        ; ModuleID = 'metadata_test'
        source_filename = "metadata_test"

        define void @stream(ptr %0, <8 x float> %1) {
        entry_block2v1:
          store <8 x float> %1, ptr %0, align 4, !nontemporal !0
          ret void
        }

        !0 = !{i32 1}
    "#]]
    .assert_eq(&llvm_mod.to_string());
    Ok(())
}
