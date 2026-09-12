// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Benchmarks for the SSA liveness implementation.
//!
//! The corpus is read from `PLIRON_LIVENESS_BENCH_DIR`. Each `.ll`
//! file in that directory is parsed independently, matching the per-module
//! measurements requested by issue #100.
//!
//! This benchmark intentionally performs one exhaustive query sweep per module
//! instead of statistical resampling. Large real-world modules can contain tens
//! of millions of value/program-point pairs, so repeating the complete sweep
//! would dominate the benchmark runtime without measuring a different workload.

use std::{
    any::Any,
    env, fs,
    hint::black_box,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use pliron::{
    analyses::liveness::{Liveness, LivenessTq},
    builtin::op_interfaces::{AtMostOneRegionInterface, SingleBlockRegionInterface},
    context::{Context, Ptr},
    graph::{
        dominance::DomInfo,
        walkers::{IRNode, WALKCONFIG_PREORDER_FORWARD, uninterruptible::immutable::walk_op},
    },
    irbuild::inserter::OpInsertionPoint,
    linked_list::ContainsLinkedList,
    op::Op,
    operation::Operation,
    pass::AnalysisManager,
    value::Value,
};
use pliron_llvm::{
    from_llvm_ir,
    llvm_sys::core::{LLVMContext, LLVMModule},
    ops::FuncOp,
};

const CORPUS_ENV: &str = "PLIRON_LIVENESS_BENCH_DIR";

struct FunctionWorkload {
    op: Ptr<Operation>,
    values: Vec<Value>,
    points: Vec<OpInsertionPoint>,
}

struct LoadedModule {
    ctx: Context,
    functions: Vec<FunctionWorkload>,
    query_count: u64,
}

impl LoadedModule {
    fn value_count(&self) -> usize {
        self.functions
            .iter()
            .map(|workload| workload.values.len())
            .sum()
    }

    fn point_count(&self) -> usize {
        self.functions
            .iter()
            .map(|workload| workload.points.len())
            .sum()
    }
}

fn collect_ir_node(ctx: &Context, workload: &mut FunctionWorkload, node: IRNode) {
    match node {
        IRNode::BasicBlock(block) => {
            workload.values.extend(block.deref(ctx).arguments());
            workload.points.push(OpInsertionPoint::AtBlockStart(block));
            workload.points.push(OpInsertionPoint::AtBlockEnd(block));
        }
        IRNode::Operation(op) => {
            let op_ref = op.deref(ctx);
            workload.values.extend(op_ref.results());
            if op_ref.get_parent_block().is_some() {
                workload.points.push(OpInsertionPoint::BeforeOperation(op));
                workload.points.push(OpInsertionPoint::AfterOperation(op));
            }
        }
        IRNode::Region(_) => {}
    }
}

fn collect_function_workloads(
    ctx: &Context,
    module: &pliron::builtin::ops::ModuleOp,
) -> Vec<FunctionWorkload> {
    module
        .get_body(ctx, 0)
        .deref(ctx)
        .iter(ctx)
        .filter_map(|op| Operation::get_op::<FuncOp>(op, ctx))
        .filter_map(|func| {
            // LLVM declarations have no body and therefore no liveness workload.
            func.get_region(ctx)?;

            let mut workload = FunctionWorkload {
                op: func.get_operation(),
                values: Vec::new(),
                points: Vec::new(),
            };
            walk_op(
                ctx,
                &mut workload,
                &WALKCONFIG_PREORDER_FORWARD,
                func.get_operation(),
                collect_ir_node,
            );
            Some(workload)
        })
        .collect()
}

fn load_module(path: &Path) -> LoadedModule {
    let mut ctx = Context::default();
    let llvm_ctx = LLVMContext::default();
    let llvm_module = LLVMModule::from_ir_in_file(
        &llvm_ctx,
        path.to_str().expect("benchmark corpus path must be UTF-8"),
    )
    .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
    let module = from_llvm_ir::convert_module(&mut ctx, &llvm_module)
        .unwrap_or_else(|err| panic!("failed to convert {} to pliron IR: {err}", path.display()));

    let functions = collect_function_workloads(&ctx, &module);
    let query_count = functions
        .iter()
        .map(|workload| (workload.values.len() as u64) * (workload.points.len() as u64))
        .sum();

    LoadedModule {
        ctx,
        functions,
        query_count,
    }
}

fn collect_corpus_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read benchmark corpus {}: {err}", dir.display()));

    for entry in entries {
        let path = entry.expect("failed to read corpus directory entry").path();
        if path.is_dir() {
            collect_corpus_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("ll") {
            files.push(path);
        }
    }
}

fn corpus_files() -> Vec<PathBuf> {
    let corpus_dir = env::var_os(CORPUS_ENV).unwrap_or_else(|| {
        panic!("{CORPUS_ENV} must point to a directory containing LLVM .ll modules")
    });
    let corpus_dir = PathBuf::from(corpus_dir);
    let mut files = Vec::new();
    collect_corpus_files(&corpus_dir, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "{} contains no .ll modules",
        corpus_dir.display()
    );
    files
}

fn precompute_all(module: &LoadedModule) -> AnalysisManager {
    let mut analyses = AnalysisManager::default();
    for workload in &module.functions {
        analyses
            .compute_analysis::<Liveness<LivenessTq>>(workload.op, &module.ctx)
            .expect("liveness analysis must compute successfully");
    }
    analyses
}

fn run_all_queries(module: &LoadedModule, analyses: &mut AnalysisManager) -> usize {
    let mut live_answers = 0usize;

    for workload in &module.functions {
        let op = workload.op;
        let mut liveness = analyses
            .try_get_analysis_mut::<Liveness<LivenessTq>>(op)
            .expect("precomputed liveness analysis missing");
        let mut dom_info = analyses
            .try_get_analysis_mut::<DomInfo>(op)
            .expect("precomputed dominance analysis missing");

        for &value in &workload.values {
            for &point in &workload.points {
                live_answers +=
                    liveness.is_live_at_point(&module.ctx, &mut dom_info, value, point) as usize;
            }
        }
    }

    live_answers
}

fn module_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("module")
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "non-string panic payload"
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn mega_queries_per_second(query_count: u64, duration: Duration) -> f64 {
    if duration.is_zero() {
        return f64::INFINITY;
    }
    query_count as f64 / duration.as_secs_f64() / 1_000_000.0
}

fn benchmark_module(path: &Path) {
    let name = module_name(path);

    let load_result = catch_unwind(AssertUnwindSafe(|| load_module(path)));
    let module = match load_result {
        Ok(module) => module,
        Err(payload) => {
            println!("{name}\tLOAD_PANIC\t{}", panic_message(&*payload));
            return;
        }
    };

    let construction_start = Instant::now();
    let construction_result = catch_unwind(AssertUnwindSafe(|| precompute_all(&module)));
    let construction_time = construction_start.elapsed();
    let mut analyses = match construction_result {
        Ok(analyses) => analyses,
        Err(payload) => {
            println!(
                "{name}\tCONSTRUCTION_PANIC\tfunctions={}\tvalues={}\tpoints={}\tqueries={}\tconstruction_ms={:.3}\t{}",
                module.functions.len(),
                module.value_count(),
                module.point_count(),
                module.query_count,
                millis(construction_time),
                panic_message(&*payload)
            );
            return;
        }
    };
    black_box(&mut analyses);

    let query_start = Instant::now();
    let query_result = catch_unwind(AssertUnwindSafe(|| {
        black_box(run_all_queries(&module, &mut analyses))
    }));
    let query_time = query_start.elapsed();

    match query_result {
        Ok(live_answers) => println!(
            "{name}\tOK\tfunctions={}\tvalues={}\tpoints={}\tqueries={}\tconstruction_ms={:.3}\tquery_ms={:.3}\tquery_mqps={:.3}\tlive_answers={}",
            module.functions.len(),
            module.value_count(),
            module.point_count(),
            module.query_count,
            millis(construction_time),
            millis(query_time),
            mega_queries_per_second(module.query_count, query_time),
            live_answers
        ),
        Err(payload) => println!(
            "{name}\tQUERY_PANIC\tfunctions={}\tvalues={}\tpoints={}\tqueries={}\tconstruction_ms={:.3}\tquery_ms={:.3}\t{}",
            module.functions.len(),
            module.value_count(),
            module.point_count(),
            module.query_count,
            millis(construction_time),
            millis(query_time),
            panic_message(&*payload)
        ),
    }
}

fn main() {
    println!(
        "module\tstatus\tfunctions\tvalues\tpoints\tqueries\tconstruction_ms\tquery_ms\tquery_mqps\tlive_answers"
    );
    for path in corpus_files() {
        benchmark_module(&path);
    }
}
