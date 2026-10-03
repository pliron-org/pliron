// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for reporting JIT'd code to `perf`.
//!
//! LLVM's perf listener is a process-wide singleton that reads `JITDUMPDIR`
//! once, so this file holds a single test, run in its own process.

#![cfg(feature = "llvm-sys")]

use std::{fs, process};

use pliron_llvm::llvm_sys::{
    core::{LLVMContext, LLVMModule},
    lljit::SimpleJIT,
};
use tempfile::tempdir;

#[test]
fn perf_listener_writes_jitdump() {
    let dump_dir = tempdir().unwrap();
    // SAFETY: the only test in this process, so no other thread reads the environment.
    unsafe { std::env::set_var("JITDUMPDIR", dump_dir.path()) };

    let context = LLVMContext::default();
    let ir = r"
      define i32 @perf_add(i32 %a, i32 %b) {
          %sum = add i32 %a, %b
          ret i32 %sum
      }";
    let module = LLVMModule::from_ir_in_str(&context, ir, None).unwrap();

    let jit = match SimpleJIT::new_with_perf_listener(context, module) {
        Err(err) if err.contains("LLVM_USE_PERF") => {
            eprintln!("Skipping: {err}");
            return;
        }
        jit => jit.unwrap(),
    };
    let add = unsafe {
        jit.lookup_symbol::<fn(i32, i32) -> i32>("perf_add")
            .unwrap()
    };
    assert_eq!(add(2, 3), 5);

    // LLVM writes `$JITDUMPDIR/.debug/jit/llvm-IR-jit-<date>-<random>/jit-<pid>.dump`.
    // The dump directory is new, thus it holds only the directory of this process.
    let run_dir = fs::read_dir(dump_dir.path().join(".debug/jit"))
        .expect("No jitdump directory created")
        .next()
        .expect("No jitdump directory created")
        .unwrap()
        .path();
    let dump = run_dir.join(format!("jit-{}.dump", process::id()));
    let contents = fs::read(&dump).unwrap_or_else(|err| panic!("{}: {err}", dump.display()));
    assert!(
        contents
            .windows(b"perf_add".len())
            .any(|w| w == b"perf_add"),
        "jitdump has no record for perf_add"
    );
}
