// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

#![cfg(feature = "llvm-sys")]

use std::path::PathBuf;

use pliron::context::Context;
use pliron_llvm::{
    from_llvm_ir,
    llvm_sys::core::{LLVMContext, LLVMModule, llvm_print_module_to_string},
    to_llvm_ir,
};

#[test]
fn global_constant_roundtrips() {
    let input_file: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "resources",
        "global_constant.ll",
    ]
    .iter()
    .collect();

    let llvm_context = LLVMContext::default();
    let module = LLVMModule::from_ir_in_file(&llvm_context, input_file.to_str().unwrap())
        .expect("failed to parse global_constant.ll");

    let ctx = &mut Context::new();
    let pliron_module =
        from_llvm_ir::convert_module(ctx, &module).expect("failed to convert LLVM IR to pliron");
    let roundtripped = to_llvm_ir::convert_module(ctx, &llvm_context, pliron_module)
        .expect("failed to convert pliron back to LLVM IR");
    let output_ir =
        llvm_print_module_to_string(&roundtripped).expect("failed to print round-tripped LLVM IR");

    let ro = output_ir
        .lines()
        .find(|line| line.starts_with("@ro ="))
        .expect("missing @ro global after round-trip");
    let rw = output_ir
        .lines()
        .find(|line| line.starts_with("@rw ="))
        .expect("missing @rw global after round-trip");

    assert_eq!(ro, "@ro = constant i32 42");
    assert_eq!(rw, "@rw = global i32 42");
}
