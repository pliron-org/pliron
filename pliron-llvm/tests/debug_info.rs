// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for the conversion of op locations to LLVM debug data.

use expect_test::expect;
use pliron::{
    builtin::{
        op_interfaces::{AtMostOneRegionInterface, SingleBlockRegionInterface, SymbolOpInterface},
        ops::ModuleOp,
    },
    combine::stream::position::SourcePosition,
    context::{Context, Ptr},
    init_env_logger_for_tests,
    linked_list::ContainsLinkedList,
    location::{Located, Location, Source},
    op::Op,
    operation::Operation,
    result::Result,
};
use pliron_llvm::{
    debug_info_conversions::to_llvm_ir::{DebugInfoOptions, EmissionKind, SourceText},
    llvm_sys::core::LLVMContext,
    ops::{ConstantOp, FuncOp},
    to_llvm_ir,
};

mod common;

/// `kernel` calls `helper`. `helper` has no locations.
const INPUT_LL: &str = r"
  define void @kernel_sym(ptr %p) {
  entry:
    %v = load i32, ptr %p
    %w = add i32 %v, 1
    store i32 %w, ptr %p
    call void @helper(ptr %p)
    ret void
  }

  define void @helper(ptr %p) {
  entry:
    ret void
  }
";

/// The function `name` in `module`.
fn function(ctx: &Context, module: ModuleOp, name: &str) -> FuncOp {
    module
        .get_body(ctx, 0)
        .deref(ctx)
        .iter(ctx)
        .filter_map(|op| Operation::get_op::<FuncOp>(op, ctx))
        .find(|func| func.get_symbol_name(ctx).to_string() == name)
        .expect("function not found")
}

/// The ops of the function `name`, in order, without constants.
fn function_ops(ctx: &Context, module: ModuleOp, name: &str) -> Vec<Ptr<Operation>> {
    let region = function(ctx, module, name)
        .get_region(ctx)
        .expect("function has no body");
    region
        .deref(ctx)
        .iter(ctx)
        .flat_map(|block| block.deref(ctx).iter(ctx).collect::<Vec<_>>())
        .filter(|op| Operation::get_op::<ConstantOp>(*op, ctx).is_none())
        .collect()
}

fn src_pos(ctx: &mut Context, file: &str, line: i32, column: i32) -> Location {
    Location::SrcPos {
        src: Source::new_from_file(ctx, file),
        pos: SourcePosition { line, column },
    }
}

fn named(name: &str, child_loc: Location) -> Location {
    Location::Named {
        name: name.to_string(),
        child_loc: Box::new(child_loc),
    }
}

/// Parse [`INPUT_LL`], and give the ops of `kernel_sym` these locations:
/// - load: a position in the kernel.
/// - add: a position in `inner`, inlined at a position in the kernel.
/// - store: a position in the kernel, in a different file.
/// - call: unknown.
/// - return: a position in the kernel.
fn located_module(ctx: &mut Context, llvm_ctx: &LLVMContext) -> Result<ModuleOp> {
    init_env_logger_for_tests!();
    let module = common::parse_llvm_ir_verify(ctx, llvm_ctx, INPUT_LL, "debug_info_test")?;
    let ops = function_ops(ctx, module, "kernel_sym");
    let [load, add, store, _call, ret] = ops.as_slice() else {
        panic!("unexpected ops: {}", ops.len());
    };
    let locations = [
        (*load, named("kernel", src_pos(ctx, "k.rs", 3, 5))),
        (
            *add,
            Location::CallSite {
                callee: Box::new(named("inner", src_pos(ctx, "inner.rs", 7, 9))),
                caller: Box::new(named("kernel", src_pos(ctx, "k.rs", 4, 5))),
            },
        ),
        (*store, named("kernel", src_pos(ctx, "other.rs", 10, 1))),
        (*ret, named("kernel", src_pos(ctx, "k.rs", 6, 1))),
    ];
    for (op, loc) in locations {
        op.deref_mut(ctx).set_loc(loc);
    }
    Ok(module)
}

#[test]
fn locations_to_debug_info() -> Result<()> {
    let ctx = &mut Context::new();
    let llvm_ctx = LLVMContext::default();
    let module = located_module(ctx, &llvm_ctx)?;

    let mut options = DebugInfoOptions::default();
    options.producer = "test".to_string();
    options.directory = "/src".to_string();
    let llvm_module = to_llvm_ir::convert_module_with_debug_info(ctx, &llvm_ctx, module, options)?;
    llvm_module.verify().expect("LLVM verifier failed");

    expect![[r#"
        ; ModuleID = 'debug_info_test'
        source_filename = "debug_info_test"

        define void @kernel_sym(ptr %0) !dbg !4 {
        entry_block2v1:
          %v_v1 = load i32, ptr %0, align 4, !dbg !7
          %w_v3 = add i32 %v_v1, 1, !dbg !8
          store i32 %w_v3, ptr %0, align 4, !dbg !12
          call void @helper(ptr %0), !dbg !15
          ret void, !dbg !16
        }

        define void @helper(ptr %0) {
        entry_block3v1:
          ret void
        }

        !llvm.dbg.cu = !{!0}
        !llvm.module.flags = !{!2, !3}

        !0 = distinct !DICompileUnit(language: DW_LANG_C, file: !1, producer: "test", isOptimized: false, runtimeVersion: 0, emissionKind: LineTablesOnly, splitDebugInlining: false)
        !1 = !DIFile(filename: "k.rs", directory: "/src")
        !2 = !{i32 2, !"Debug Info Version", i32 3}
        !3 = !{i32 2, !"Dwarf Version", i32 4}
        !4 = distinct !DISubprogram(name: "kernel", linkageName: "kernel_sym", scope: !1, file: !1, line: 3, type: !5, scopeLine: 3, spFlags: DISPFlagDefinition, unit: !0)
        !5 = !DISubroutineType(types: !6)
        !6 = !{}
        !7 = !DILocation(line: 3, column: 5, scope: !4)
        !8 = !DILocation(line: 7, column: 9, scope: !9, inlinedAt: !11)
        !9 = distinct !DISubprogram(name: "inner", scope: !10, file: !10, type: !5, spFlags: DISPFlagLocalToUnit | DISPFlagDefinition, unit: !0)
        !10 = !DIFile(filename: "inner.rs", directory: "/src")
        !11 = !DILocation(line: 4, column: 5, scope: !4)
        !12 = !DILocation(line: 10, column: 1, scope: !13)
        !13 = !DILexicalBlockFile(scope: !4, file: !14, discriminator: 0)
        !14 = !DIFile(filename: "other.rs", directory: "/src")
        !15 = !DILocation(line: 0, scope: !4)
        !16 = !DILocation(line: 6, column: 1, scope: !4)
    "#]]
    .assert_eq(&llvm_module.to_string());
    Ok(())
}

#[test]
fn full_emission_kind() -> Result<()> {
    let ctx = &mut Context::new();
    let llvm_ctx = LLVMContext::default();
    let module = located_module(ctx, &llvm_ctx)?;

    let mut options = DebugInfoOptions::default();
    options.emission_kind = EmissionKind::Full;
    let llvm_module = to_llvm_ir::convert_module_with_debug_info(ctx, &llvm_ctx, module, options)?;
    llvm_module.verify().expect("LLVM verifier failed");
    let ir = llvm_module.to_string();
    assert!(ir.contains("emissionKind: FullDebug"), "{ir}");
    Ok(())
}

/// A file with a text embeds it and its checksum. The other files do not.
#[test]
fn source_text() -> Result<()> {
    let ctx = &mut Context::new();
    let llvm_ctx = LLVMContext::default();
    let module = located_module(ctx, &llvm_ctx)?;

    let mut options = DebugInfoOptions::default();
    options.dwarf_version = 5;
    options.source_text.insert(
        "k.rs".to_string(),
        SourceText {
            text: "fn kernel() {}\n",
            md5: "418f53b969f82938a944f4f2bae91f4c".to_string(),
        },
    );
    let llvm_module = to_llvm_ir::convert_module_with_debug_info(ctx, &llvm_ctx, module, options)?;
    llvm_module.verify().expect("LLVM verifier failed");
    let ir = llvm_module.to_string();
    let files: Vec<_> = ir.lines().filter(|line| line.contains("!DIFile")).collect();
    expect![[r#"
        !1 = !DIFile(filename: "k.rs", directory: "", checksumkind: CSK_MD5, checksum: "418f53b969f82938a944f4f2bae91f4c", source: "fn kernel() {}\0A")
        !10 = !DIFile(filename: "inner.rs", directory: "")
        !14 = !DIFile(filename: "other.rs", directory: "")"#]]
    .assert_eq(&files.join("\n"));
    Ok(())
}

/// A module without locations gets no debug data.
#[test]
fn no_locations_no_debug_info() -> Result<()> {
    let ctx = &mut Context::new();
    let llvm_ctx = LLVMContext::default();
    let module = common::parse_llvm_ir_verify(ctx, &llvm_ctx, INPUT_LL, "debug_info_test")?;
    let llvm_module = to_llvm_ir::convert_module_with_debug_info(
        ctx,
        &llvm_ctx,
        module,
        DebugInfoOptions::default(),
    )?;
    llvm_module.verify().expect("LLVM verifier failed");
    let ir = llvm_module.to_string();
    assert!(
        !ir.contains("!dbg") && !ir.contains("llvm.module.flags"),
        "{ir}"
    );
    Ok(())
}

/// `second` has PHIs and follows a located function. The module has a DWARF version.
const PHI_INPUT_LL: &str = r#"
  define void @first() {
  entry:
    ret void
  }

  define i32 @second(i1 %c, i32 %a) {
  entry:
    br i1 %c, label %then, label %exit
  then:
    br label %exit
  exit:
    %r = phi i32 [ %a, %entry ], [ 0, %then ]
    ret i32 %r
  }

  !llvm.module.flags = !{!0}
  !0 = !{i32 7, !"Dwarf Version", i32 5}
"#;

fn fused(locations: Vec<Location>) -> Location {
    Location::Fused {
        metadata: None,
        locations,
    }
}

/// `callee`, inlined at `at`.
fn call_site(callee: Location, at: Location) -> Location {
    Location::CallSite {
        callee: Box::new(callee),
        caller: Box::new(at),
    }
}

/// Fused locations, call sites with a missing part, a located function op,
/// PHIs after a located function, and an existing DWARF version.
#[test]
fn partial_locations() -> Result<()> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let llvm_ctx = LLVMContext::default();
    let module = common::parse_llvm_ir_verify(ctx, &llvm_ctx, PHI_INPUT_LL, "debug_info_test")?;

    let [first_ret] = function_ops(ctx, module, "first")[..] else {
        panic!("unexpected ops in first");
    };
    let [cond_br, br, ret] = function_ops(ctx, module, "second")[..] else {
        panic!("unexpected ops in second");
    };
    let second = function(ctx, module, "second").get_operation();
    let locations = [
        // A callee without a position: the caller position.
        (
            first_ret,
            call_site(named("gone", Location::Unknown), src_pos(ctx, "k.rs", 1, 1)),
        ),
        // No name: the subprogram takes the symbol name.
        (
            second,
            fused(vec![Location::Unknown, src_pos(ctx, "k.rs", 20, 1)]),
        ),
        (
            cond_br,
            fused(vec![Location::Unknown, src_pos(ctx, "k.rs", 21, 3)]),
        ),
        // A callee without a name: the caller position.
        (
            br,
            call_site(src_pos(ctx, "inner.rs", 8, 1), src_pos(ctx, "k.rs", 22, 3)),
        ),
        // A caller without a position: the callee position, not inlined.
        (
            ret,
            call_site(
                named("inner", src_pos(ctx, "inner.rs", 9, 1)),
                Location::Unknown,
            ),
        ),
    ];
    for (op, loc) in locations {
        op.deref_mut(ctx).set_loc(loc);
    }

    let llvm_module = to_llvm_ir::convert_module_with_debug_info(
        ctx,
        &llvm_ctx,
        module,
        DebugInfoOptions::default(),
    )?;
    llvm_module.verify().expect("LLVM verifier failed");
    expect![[r#"
        ; ModuleID = 'debug_info_test'
        source_filename = "debug_info_test"

        define void @first() !dbg !4 {
        entry_block2v1:
          ret void, !dbg !7
        }

        define i32 @second(i1 %0, i32 %1) !dbg !8 {
        entry_block3v1:
          br i1 %0, label %then_block4v1, label %exit_block5v1, !dbg !9

        then_block4v1:                                    ; preds = %entry_block3v1
          br label %exit_block5v1, !dbg !10

        exit_block5v1:                                    ; preds = %then_block4v1, %entry_block3v1
          %v3 = phi i32 [ %1, %entry_block3v1 ], [ 0, %then_block4v1 ]
          ret i32 %v3, !dbg !11
        }

        !llvm.module.flags = !{!0, !1}
        !llvm.dbg.cu = !{!2}

        !0 = !{i32 7, !"Dwarf Version", i32 5}
        !1 = !{i32 2, !"Debug Info Version", i32 3}
        !2 = distinct !DICompileUnit(language: DW_LANG_C, file: !3, producer: "pliron", isOptimized: false, runtimeVersion: 0, emissionKind: LineTablesOnly, splitDebugInlining: false)
        !3 = !DIFile(filename: "k.rs", directory: "")
        !4 = distinct !DISubprogram(name: "first", scope: !3, file: !3, line: 1, type: !5, scopeLine: 1, spFlags: DISPFlagDefinition, unit: !2)
        !5 = !DISubroutineType(types: !6)
        !6 = !{}
        !7 = !DILocation(line: 1, column: 1, scope: !4)
        !8 = distinct !DISubprogram(name: "second", scope: !3, file: !3, line: 20, type: !5, scopeLine: 20, spFlags: DISPFlagDefinition, unit: !2)
        !9 = !DILocation(line: 21, column: 3, scope: !8)
        !10 = !DILocation(line: 22, column: 3, scope: !8)
        !11 = !DILocation(line: 9, column: 1, scope: !12)
        !12 = !DILexicalBlockFile(scope: !8, file: !13, discriminator: 0)
        !13 = !DIFile(filename: "inner.rs", directory: "")
    "#]].assert_eq(&llvm_module.to_string());
    Ok(())
}
