// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for the pass manager. (`pliron::pass`).

// We use pliron-llvm in this test, which is not supported in wasm.
#![cfg(not(target_family = "wasm"))]

use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use expect_test::expect;
use pliron::{
    builtin::{op_interfaces::SymbolTableInterface, ops::ModuleOp},
    context::{Context, Ptr},
    dict_key,
    identifier::Identifier,
    init_env_logger_for_tests,
    irbuild::IRStatus,
    irfmt::parsers::spaced,
    linked_list::ContainsLinkedList,
    operation::{Operation, verify_operation},
    opts::dce::DCEPass,
    parsable::parse_from_str,
    pass::{
        Analysis, AnalysisManager, Guard, NestedOpsPass, OpGuard, OpInterfaceGuard,
        OpInterfacePass, OpPass, PMConfig, Pass, PassResult, Passes,
    },
    printable::Printable,
    result::{ExpectOk, Result},
};
use pliron_llvm::ops::FuncOp;

// ---------------------------------------------------------------------
// Test fixtures and helpers
// ---------------------------------------------------------------------

const SIMPLE_FUNC: &str = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.return c
    }
"#;

const TWO_FUNC_MODULE: &str = r#"
    builtin.module @m {
    ^entry():
      llvm.func @f1: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        live1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        dead1 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64;
        llvm.return live1
      };
      llvm.func @f2: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        live2 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        dead2 = builtin.constant <builtin.integer <22: i64>> : builtin.integer i64;
        llvm.return live2
      }
    }
"#;

fn parse(ctx: &mut Context, input: &str) -> Ptr<Operation> {
    init_env_logger_for_tests!();
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);
    verify_operation(op, ctx).expect_ok(ctx);
    op
}

/// Get a `Ptr` to the first operation immediately nested inside `op`
/// (i.e., the head of the first block of the first region).
fn first_nested_op(ctx: &Context, op: Ptr<Operation>) -> Ptr<Operation> {
    let region = op.deref(ctx).regions().next().expect("op has a region");
    let block = region.deref(ctx).get_head().expect("region has a block");
    block.deref(ctx).get_head().expect("block has an op")
}

fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("pliron_pass_test_{tag}_{nanos}"))
}

// A simple string log, stashed inside `PMState::custom_state` so that
// test passes can report what they observed.
dict_key!(PASS_TEST_LOG_KEY, "pass_test_log");

fn push_log(analyses: &mut AnalysisManager, entry: impl Into<String>) {
    let key = PASS_TEST_LOG_KEY.clone();
    let state = analyses.pm_data_mut().state_mut();
    let log = state
        .custom_state
        .entry(key)
        .or_insert_with(|| Box::new(Vec::<String>::new()));
    log.downcast_mut::<Vec<String>>()
        .unwrap()
        .push(entry.into());
}

fn get_log(analyses: &AnalysisManager) -> Vec<String> {
    analyses
        .pm_data()
        .state()
        .custom_state
        .get(&PASS_TEST_LOG_KEY)
        .map(|b| b.downcast_ref::<Vec<String>>().unwrap().clone())
        .unwrap_or_default()
}

/// A [Pass] that just records that it ran (via [push_log]) and reports a
/// configurable [IRStatus], without touching the IR at all.
struct RecordPass {
    tag: &'static str,
    changed: IRStatus,
}

impl RecordPass {
    fn new(tag: &'static str, changed: IRStatus) -> Self {
        Self { tag, changed }
    }
}

impl Pass for RecordPass {
    fn name(&self) -> &str {
        self.tag
    }

    fn run(
        &mut self,
        _op: Ptr<Operation>,
        _ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        push_log(analyses, self.tag);
        let mut res = PassResult::default();
        res.ir_changed = self.changed;
        Ok(res)
    }
}

/// A [Pass] that counts, in its own field, how many times it actually ran.
#[derive(Default)]
struct CountingPass {
    count: u32,
}

impl Pass for CountingPass {
    fn name(&self) -> &str {
        "counting_pass"
    }

    fn run(
        &mut self,
        _op: Ptr<Operation>,
        _ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        self.count += 1;
        Ok(PassResult::default())
    }
}

/// An [Analysis] whose payload is the compute-count at the time it was computed,
/// so that tests can verify caching/invalidation of analyses.
struct CountingAnalysis(u32);

fn counting_analysis_computed_key() -> Identifier {
    "counting_analysis_computed".try_into().unwrap()
}

impl Analysis for CountingAnalysis {
    fn name(&self) -> &str {
        "counting_analysis"
    }

    fn compute(_op: Ptr<Operation>, _ctx: &Context, analyses: &mut AnalysisManager) -> Result<Self>
    where
        Self: Sized,
    {
        let key = counting_analysis_computed_key();
        let state = analyses.pm_data_mut().state_mut();
        let counter = state
            .custom_state
            .entry(key)
            .or_insert_with(|| Box::new(0u32));
        let counter = counter.downcast_mut::<u32>().unwrap();
        *counter += 1;
        Ok(CountingAnalysis(*counter))
    }
}

fn compute_count(analyses: &AnalysisManager) -> u32 {
    analyses
        .pm_data()
        .state()
        .custom_state
        .get(&counting_analysis_computed_key())
        .map(|b| *b.downcast_ref::<u32>().unwrap())
        .unwrap_or(0)
}

/// A [Pass] that requests [CountingAnalysis], with configurable change/preservation
/// behavior.
struct AnalysisUserPass {
    changed: IRStatus,
    preserve: bool,
}

impl Pass for AnalysisUserPass {
    fn name(&self) -> &str {
        "analysis_user"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        analyses.get_analysis::<CountingAnalysis>(op, ctx)?;
        let mut res = PassResult::default();
        res.ir_changed = self.changed;
        if self.preserve {
            res.set_preserved::<CountingAnalysis>();
        }
        Ok(res)
    }
}

/// A [Pass] that erases the terminator of `op`'s first block, corrupting the IR
/// and reports [IRStatus::Changed].
struct CorruptTerminatorPass;

impl Pass for CorruptTerminatorPass {
    fn name(&self) -> &str {
        "corrupt_terminator"
    }

    fn run(
        &mut self,
        op: Ptr<Operation>,
        ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let region = op.deref(ctx).regions().next().expect("op has a region");
        let block = region.deref(ctx).get_head().expect("region has a block");
        let terminator = block.deref(ctx).get_tail().expect("block has a terminator");
        Operation::erase(terminator, ctx);
        let mut res = PassResult::default();
        res.ir_changed = IRStatus::Changed;
        Ok(res)
    }
}

// ---------------------------------------------------------------------
// Passes: sequencing and IRStatus aggregation
// ---------------------------------------------------------------------

#[test]
fn passes_run_in_order_and_aggregate_changed() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let mut passes = Passes::default();
    passes.add_pass(RecordPass::new("a", IRStatus::Unchanged));
    passes.add_pass(RecordPass::new("b", IRStatus::Changed));
    passes.add_pass(RecordPass::new("c", IRStatus::Unchanged));

    let mut analyses = AnalysisManager::default();
    let result = passes.run(op, ctx, &mut analyses)?;

    assert_eq!(result.ir_changed, IRStatus::Changed);
    assert_eq!(get_log(&analyses), vec!["a", "b", "c"]);
    assert_eq!(analyses.pm_data().state().pass_run_count, 3);
    Ok(())
}

#[test]
fn passes_all_unchanged_yields_unchanged() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let mut passes = Passes::default();
    passes.add_pass(RecordPass::new("a", IRStatus::Unchanged));
    passes.add_pass(RecordPass::new("b", IRStatus::Unchanged));

    let mut analyses = AnalysisManager::default();
    let result = passes.run(op, ctx, &mut analyses)?;

    assert_eq!(result.ir_changed, IRStatus::Unchanged);
    assert_eq!(get_log(&analyses), vec!["a", "b"]);
    Ok(())
}

// ---------------------------------------------------------------------
// NestedOpsPass
// ---------------------------------------------------------------------

#[test]
fn nested_ops_pass_visits_immediate_children_only() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);

    let mut nested = NestedOpsPass::new(RecordPass::new("visit", IRStatus::Unchanged));
    let mut analyses = AnalysisManager::default();
    let result = nested.run(op, ctx, &mut analyses)?;

    // Only the 2 llvm.func ops directly inside the module are visited;
    // the ops inside their bodies are not.
    assert_eq!(result.ir_changed, IRStatus::Unchanged);
    assert_eq!(get_log(&analyses), vec!["visit", "visit"]);
    Ok(())
}

#[test]
fn nested_ops_pass_composition_enables_recursive_traversal() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);

    // Nesting NestedOpsPass inside itself walks one level deeper each time:
    // module -> funcs -> ops inside each func's entry block.
    let inner = NestedOpsPass::new(RecordPass::new("deep", IRStatus::Unchanged));
    let mut outer = NestedOpsPass::new(inner);

    let mut analyses = AnalysisManager::default();
    outer.run(op, ctx, &mut analyses)?;

    // 3 ops (2 constants and a return) in each of the 2 functions.
    assert_eq!(get_log(&analyses).len(), 6);
    Ok(())
}

// ---------------------------------------------------------------------
// Guard / OpGuard / OpInterfaceGuard / GuardedPass
// ---------------------------------------------------------------------

#[test]
fn op_guard_allows_matching_op_type_only() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);
    let func_op = first_nested_op(ctx, op);

    let mut guarded = OpPass::<ModuleOp, RecordPass>::new(
        OpGuard::default(),
        RecordPass::new("module_only", IRStatus::Changed),
    );

    let mut analyses = AnalysisManager::default();
    let on_module = guarded.run(op, ctx, &mut analyses)?;
    assert_eq!(on_module.ir_changed, IRStatus::Changed);
    assert_eq!(get_log(&analyses), vec!["module_only"]);

    let on_func = guarded.run(func_op, ctx, &mut analyses)?;
    assert_eq!(on_func.ir_changed, IRStatus::Unchanged);
    // No new entry: the pass did not run on a non-ModuleOp operation.
    assert_eq!(get_log(&analyses), vec!["module_only"]);
    Ok(())
}

#[test]
fn op_interface_guard_filters_by_interface() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);
    let func_op = first_nested_op(ctx, op);

    // ModuleOp implements SymbolTableInterface; the llvm.func FuncOp does not.
    let guard = OpInterfaceGuard::<dyn SymbolTableInterface>::default();
    assert!(guard.is_allowed(op, ctx));
    assert!(!guard.is_allowed(func_op, ctx));

    let mut pass = OpInterfacePass::<dyn SymbolTableInterface, RecordPass>::new(
        OpInterfaceGuard::default(),
        RecordPass::new("symtab_only", IRStatus::Unchanged),
    );

    let mut analyses = AnalysisManager::default();
    pass.run(op, ctx, &mut analyses)?;
    assert_eq!(get_log(&analyses), vec!["symtab_only"]);

    pass.run(func_op, ctx, &mut analyses)?;
    assert_eq!(get_log(&analyses), vec!["symtab_only"]);
    Ok(())
}

#[test]
fn guarded_pass_deref_exposes_inner_pass() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);
    let func_op = first_nested_op(ctx, op);

    let mut guarded = OpPass::<ModuleOp, CountingPass>::default();
    let mut analyses = AnalysisManager::default();

    guarded.run(op, ctx, &mut analyses)?;
    assert_eq!(guarded.count, 1);

    // Guard rejects a non-ModuleOp: the inner pass must not run.
    guarded.run(func_op, ctx, &mut analyses)?;
    assert_eq!(guarded.count, 1);
    Ok(())
}

// ---------------------------------------------------------------------
// Analysis / AnalysisManager caching and invalidation
// ---------------------------------------------------------------------

#[test]
fn analysis_is_cached_and_recomputed_after_invalidation() -> Result<()> {
    // Case 1: Unchanged passes never invalidate; the analysis is computed once.
    {
        let ctx = &mut Context::new();
        let op = parse(ctx, SIMPLE_FUNC);
        let mut passes = Passes::default();
        passes.add_pass(AnalysisUserPass {
            changed: IRStatus::Unchanged,
            preserve: false,
        });
        passes.add_pass(AnalysisUserPass {
            changed: IRStatus::Unchanged,
            preserve: false,
        });
        let mut analyses = AnalysisManager::default();
        passes.run(op, ctx, &mut analyses)?;
        assert_eq!(compute_count(&analyses), 1);
    }

    // Case 2: A Changed pass that does not preserve the analysis invalidates it,
    // forcing recomputation on the next request.
    {
        let ctx = &mut Context::new();
        let op = parse(ctx, SIMPLE_FUNC);
        let mut passes = Passes::default();
        passes.add_pass(AnalysisUserPass {
            changed: IRStatus::Changed,
            preserve: false,
        });
        passes.add_pass(AnalysisUserPass {
            changed: IRStatus::Unchanged,
            preserve: false,
        });
        let mut analyses = AnalysisManager::default();
        passes.run(op, ctx, &mut analyses)?;
        assert_eq!(compute_count(&analyses), 2);
    }

    // Case 3: A Changed pass that explicitly preserves the analysis keeps it cached.
    {
        let ctx = &mut Context::new();
        let op = parse(ctx, SIMPLE_FUNC);
        let mut passes = Passes::default();
        passes.add_pass(AnalysisUserPass {
            changed: IRStatus::Changed,
            preserve: true,
        });
        passes.add_pass(AnalysisUserPass {
            changed: IRStatus::Unchanged,
            preserve: false,
        });
        let mut analyses = AnalysisManager::default();
        passes.run(op, ctx, &mut analyses)?;
        assert_eq!(compute_count(&analyses), 1);
    }

    Ok(())
}

#[test]
fn analysis_manager_cache_access_methods() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);
    let mut analyses = AnalysisManager::default();

    assert!(analyses.try_get_analysis::<CountingAnalysis>(op).is_none());

    analyses.compute_analysis::<CountingAnalysis>(op, ctx)?;
    assert_eq!(compute_count(&analyses), 1);
    assert!(analyses.try_get_analysis::<CountingAnalysis>(op).is_some());

    // Already cached: computing again must not recompute.
    analyses.compute_analysis::<CountingAnalysis>(op, ctx)?;
    assert_eq!(compute_count(&analyses), 1);

    // get_analysis_mut allows in-place mutation without recomputation.
    {
        let mut a = analyses.get_analysis_mut::<CountingAnalysis>(op, ctx)?;
        a.0 = 999;
    }
    assert_eq!(compute_count(&analyses), 1);
    assert_eq!(
        analyses.try_get_analysis::<CountingAnalysis>(op).unwrap().0,
        999
    );

    // try_get_analysis_mut similarly gives mutable access to the cached value.
    {
        let mut a = analyses
            .try_get_analysis_mut::<CountingAnalysis>(op)
            .unwrap();
        a.0 += 1;
    }
    assert_eq!(
        analyses.try_get_analysis::<CountingAnalysis>(op).unwrap().0,
        1000
    );

    Ok(())
}

#[test]
fn nested_ops_pass_keys_analysis_cache_per_operation() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);

    let mut nested = NestedOpsPass::new(AnalysisUserPass {
        changed: IRStatus::Unchanged,
        preserve: false,
    });
    let mut analyses = AnalysisManager::default();
    nested.run(op, ctx, &mut analyses)?;

    // The analysis is computed once per distinct nested operation (2 functions).
    assert_eq!(compute_count(&analyses), 2);
    Ok(())
}

// ---------------------------------------------------------------------
// PassManager::run_pass hooks: skip_passes, verify_before/after
// ---------------------------------------------------------------------

#[test]
fn skip_passes_prevents_pass_execution() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let mut config = PMConfig::default();
    config.skip_passes.insert("skip_me".to_string());

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(RecordPass::new("skip_me", IRStatus::Changed));
    passes.add_pass(RecordPass::new("run_me", IRStatus::Changed));

    let result = passes.run(op, ctx, &mut analyses)?;

    assert_eq!(result.ir_changed, IRStatus::Changed);
    assert_eq!(get_log(&analyses), vec!["run_me"]);
    // Skipped passes don't count towards pass_run_count either.
    assert_eq!(analyses.pm_data().state().pass_run_count, 1);
    Ok(())
}

#[test]
fn verify_after_all_detects_corruption_introduced_by_pass() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let config = PMConfig {
        verify_after_all: true,
        ..Default::default()
    };

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(CorruptTerminatorPass);

    let err = match passes.run(op, ctx, &mut analyses) {
        Err(e) => e,
        Ok(_) => panic!("corrupted IR must fail post-pass verification"),
    };
    expect![[r#"
        [<in-memory>: line: 3, column: 7] Compilation error: verification failed.
        Basic block "entry_block1v1" is missing a terminator"#]]
    .assert_eq(&err.disp(ctx).to_string());
    Ok(())
}

#[test]
fn verify_before_all_detects_corruption_from_previous_pass() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let config = PMConfig {
        verify_before_all: true,
        ..Default::default()
    };

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(CorruptTerminatorPass);
    passes.add_pass(RecordPass::new("after_corrupt", IRStatus::Unchanged));

    let result = passes.run(op, ctx, &mut analyses);
    assert!(result.is_err());
    // The second pass's pre-verify hook must have caught the corruption
    // before the pass body itself ran.
    assert!(get_log(&analyses).is_empty());
    Ok(())
}

#[test]
fn verify_before_named_set_scopes_to_specific_pass_name() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let mut config = PMConfig::default();
    config.verify_before.insert("record".to_string());

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(CorruptTerminatorPass);
    passes.add_pass(RecordPass::new("record", IRStatus::Unchanged));

    let result = passes.run(op, ctx, &mut analyses);
    assert!(result.is_err());
    assert!(get_log(&analyses).is_empty());
    Ok(())
}

#[test]
fn verify_disabled_allows_corrupted_ir_to_pass_through() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    // Default PMConfig: no verify_before/after flags set.
    let mut analyses = AnalysisManager::default();
    let mut passes = Passes::default();
    passes.add_pass(CorruptTerminatorPass);

    let result = passes.run(op, ctx, &mut analyses)?;
    assert_eq!(result.ir_changed, IRStatus::Changed);

    // The pass manager didn't check, but the IR is indeed now invalid.
    assert!(verify_operation(op, ctx).is_err());
    Ok(())
}

#[test]
fn time_all_passes_does_not_alter_pass_results() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let config = PMConfig {
        time_all_passes: true,
        ..Default::default()
    };

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(RecordPass::new("a", IRStatus::Changed));

    let result = passes.run(op, ctx, &mut analyses)?;
    assert_eq!(result.ir_changed, IRStatus::Changed);
    assert_eq!(get_log(&analyses), vec!["a"]);
    Ok(())
}

// ---------------------------------------------------------------------
// IR printing (print_before_all/print_after_all + ir_printing_dir)
// ---------------------------------------------------------------------

#[test]
fn ir_printing_dir_writes_before_and_after_files() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let dir = unique_temp_dir("ir_printing_dir");

    let config = PMConfig {
        print_before_all: true,
        print_after_all: true,
        ir_printing_dir: Some(dir.clone()),
        ..Default::default()
    };

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(CorruptTerminatorPass);
    passes.run(op, ctx, &mut analyses)?;

    let before = fs::read_to_string(dir.join("0-before-corrupt_terminator.plir")).unwrap();
    let after = fs::read_to_string(dir.join("0-after-corrupt_terminator.plir")).unwrap();

    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            llvm.return c_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 7], []
        !1 = @[<in-memory>: line: 4, column: 7], [builtin_debug_info = builtin.debug_info [c]]
        !2 = @[<in-memory>: line: 5, column: 7], []
        !3 = @[<in-memory>: line: 2, column: 5], []
    "#]]
    .assert_eq(&before);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1
        } !2

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 7], []
        !1 = @[<in-memory>: line: 4, column: 7], [builtin_debug_info = builtin.debug_info [c]]
        !2 = @[<in-memory>: line: 2, column: 5], []
    "#]]
    .assert_eq(&after);

    fs::remove_dir_all(&dir).ok();
    Ok(())
}

// ---------------------------------------------------------------------
// PMConfig::custom_config / PMState::custom_state / PMState::stats
// ---------------------------------------------------------------------

struct ConfigReaderPass;

impl Pass for ConfigReaderPass {
    fn name(&self) -> &str {
        "config_reader"
    }

    fn run(
        &mut self,
        _op: Ptr<Operation>,
        _ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let key: Identifier = "multiplier".try_into().unwrap();
        let multiplier = analyses
            .pm_data()
            .config()
            .custom_config
            .get(&key)
            .and_then(|b| b.downcast_ref::<u32>())
            .copied()
            .unwrap_or(0);

        let result_key: Identifier = "result".try_into().unwrap();
        analyses
            .pm_data_mut()
            .state_mut()
            .custom_state
            .insert(result_key, Box::new(multiplier * 2));

        Ok(PassResult::default())
    }
}

#[test]
fn custom_config_and_custom_state_round_trip() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let mut config = PMConfig::default();
    let multiplier_key: Identifier = "multiplier".try_into().unwrap();
    config.custom_config.insert(multiplier_key, Box::new(21u32));

    let mut analyses = AnalysisManager::default();
    analyses.set_config(config);

    let mut passes = Passes::default();
    passes.add_pass(ConfigReaderPass);
    passes.run(op, ctx, &mut analyses)?;

    let result_key: Identifier = "result".try_into().unwrap();
    let result = analyses
        .pm_data()
        .state()
        .custom_state
        .get(&result_key)
        .and_then(|b| b.downcast_ref::<u32>())
        .copied();
    assert_eq!(result, Some(42));
    Ok(())
}

struct StatsPass;

impl Pass for StatsPass {
    fn name(&self) -> &str {
        "stats_pass"
    }

    fn run(
        &mut self,
        _op: Ptr<Operation>,
        _ctx: &mut Context,
        analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        analyses
            .pm_data_mut()
            .state_mut()
            .stats
            .insert("visited_ops", Box::new(7u32));
        Ok(PassResult::default())
    }
}

#[test]
fn pm_state_stats_can_be_recorded_by_passes() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, SIMPLE_FUNC);

    let mut analyses = AnalysisManager::default();
    let mut passes = Passes::default();
    passes.add_pass(StatsPass);
    passes.run(op, ctx, &mut analyses)?;

    let stat = analyses.pm_data().state().stats.get("visited_ops").unwrap();
    assert_eq!(stat.disp(ctx).to_string(), "7");
    Ok(())
}

// ---------------------------------------------------------------------
// End-to-end: a production pass through the full guard/nesting machinery
// ---------------------------------------------------------------------

#[test]
fn end_to_end_dce_pipeline_through_nested_op_guards() -> Result<()> {
    let ctx = &mut Context::new();
    let op = parse(ctx, TWO_FUNC_MODULE);

    // Module-level pipeline that only runs on ModuleOp, and within it, only
    // runs DCE on each nested llvm.func.
    let mut module_passes = OpPass::<ModuleOp, Passes>::default();
    module_passes.add_pass(NestedOpsPass::new(OpPass::<FuncOp, DCEPass>::default()));

    let mut analyses = AnalysisManager::default();
    let result = module_passes.run(op, ctx, &mut analyses)?;

    assert_eq!(result.ir_changed, IRStatus::Changed);
    verify_operation(op, ctx)?;

    let after = Operation::get_op_dyn(op, ctx).disp(ctx).to_string();
    expect![[r#"
        builtin.module @m 
        {
          ^entry_block1v1() !0:
            llvm.func @f1: llvm.func <builtin.integer i64() variadic = false>
              [] 
            {
              ^entry_block2v1() !1:
                live1_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !2;
                llvm.return live1_v0 !3
            } !4;
            llvm.func @f2: llvm.func <builtin.integer i64() variadic = false>
              [] 
            {
              ^entry_block3v1() !5:
                live2_v2 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !6;
                llvm.return live2_v2 !7
            } !8
        }"#]]
    .assert_eq(&after);
    Ok(())
}
