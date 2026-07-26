// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for printing IR to files before/after passes ([PMConfig::ir_printing_dir]).
//! File system access requires the `std` feature.
#![cfg(feature = "std")]

use std::{fs, path::Path};

use pliron::{
    context::{Context, Ptr},
    irbuild::IRStatus,
    op::Op,
    operation::Operation,
    pass::{AnalysisManager, PMConfig, Pass, PassResult, Passes},
    result::{ErrorKind, Result},
    std_deps::io::PathBuf,
};

use crate::common::const_ret_in_mod;

mod common;

#[derive(Default)]
struct NoOpPass;

impl Pass for NoOpPass {
    fn name(&self) -> &str {
        "noop"
    }

    fn run(
        &mut self,
        _op: Ptr<Operation>,
        _ctx: &mut Context,
        _analyses: &mut AnalysisManager,
    ) -> Result<PassResult> {
        let mut result = PassResult::default();
        result.ir_changed = IRStatus::Unchanged;
        Ok(result)
    }
}

fn run_noop_pass_with_ir_printing_dir(ctx: &mut Context, dir: &Path) -> Result<()> {
    let (module, ..) = const_ret_in_mod(ctx).unwrap();
    let mut passes = Passes::default();
    passes.add_pass(NoOpPass);
    let mut analyses = AnalysisManager::default();
    analyses.set_config(PMConfig {
        print_before_all: true,
        print_after_all: true,
        ir_printing_dir: Some(dir.to_path_buf()),
        ..Default::default()
    });
    passes
        .run(module.get_operation(), ctx, &mut analyses)
        .map(|_| ())
}

/// The printing directory (including parents) must be created if it doesn't exist.
#[test]
fn ir_printing_creates_dir_and_files() {
    let ctx = &mut Context::new();
    fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pass_ir_printing");
    let dir = base.join("nested");
    let _ = fs::remove_dir_all(&base);

    run_noop_pass_with_ir_printing_dir(ctx, &dir).unwrap();

    for kind in ["before", "after"] {
        let file = dir.join(format!("0-{kind}-noop.plir"));
        let contents = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("Failed to read {}: {err}", file.display()));
        assert!(
            contents.contains("foo"),
            "Expected printed IR to contain the function name, got:\n{contents}"
        );
    }

    let _ = fs::remove_dir_all(&base);
}

/// Failure to create/write into the printing directory must be an error, not a panic.
#[test]
fn ir_printing_failure_is_an_error() {
    let ctx = &mut Context::new();
    fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).unwrap();
    // A regular file at the printing directory path makes its creation fail.
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pass_ir_printing_not_a_dir");
    fs::write(&file, b"").unwrap();

    let err = run_noop_pass_with_ir_printing_dir(ctx, &file).unwrap_err();
    assert!(
        matches!(err.kind, ErrorKind::InvalidArgument),
        "unexpected error kind: {err}"
    );

    let _ = fs::remove_file(&file);
}
