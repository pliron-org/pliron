// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for the conversion of op locations to LLVM debug data.

#![cfg(feature = "debug-info")]

use expect_test::expect;
use pliron::{
    builtin::{
        op_interfaces::{SingleBlockRegionInterface, SymbolOpInterface},
        ops::ModuleOp,
    },
    combine::stream::position::SourcePosition,
    context::{Context, Ptr},
    init_env_logger_for_tests,
    linked_list::ContainsLinkedList,
    location::{Located, Location, Source},
    operation::Operation,
    result::Result,
};
use pliron_llvm::{
    debug_info_conversions::to_llvm_ir::{DebugInfoOptions, EmissionKind},
    llvm_sys::core::LLVMContext,
    ops::{ConstantOp, FuncOp},
    to_llvm_ir,
};

mod common;

/// `kernel` calls `helper`. `helper` has no locations.
const INPUT_LL: &str = r#"
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
"#;

/// The ops of the entry block of the function `name`, in order, without constants.
fn function_ops(ctx: &Context, module: ModuleOp, name: &str) -> Vec<Ptr<Operation>> {
    let func = module
        .get_body(ctx, 0)
        .deref(ctx)
        .iter(ctx)
        .filter_map(|op| Operation::get_op::<FuncOp>(op, ctx))
        .find(|func| func.get_symbol_name(ctx).to_string() == name)
        .expect("function not found");
    let entry = func.get_entry_block(ctx).expect("function has no body");
    entry
        .deref(ctx)
        .iter(ctx)
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

/// Parse [INPUT_LL], and give the ops of `kernel_sym` these locations:
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

/// [to_llvm_ir::convert_module] ignores locations.
#[test]
fn convert_module_ignores_locations() -> Result<()> {
    let ctx = &mut Context::new();
    let llvm_ctx = LLVMContext::default();
    let module = located_module(ctx, &llvm_ctx)?;
    let llvm_module = common::to_llvm_ir_verify(ctx, &llvm_ctx, module)?;
    let ir = llvm_module.to_string();
    assert!(
        !ir.contains("!dbg") && !ir.contains("DICompileUnit"),
        "{ir}"
    );
    Ok(())
}
