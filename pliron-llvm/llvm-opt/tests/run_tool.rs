// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use std::{env, path::PathBuf, process::Output, sync::LazyLock};

use assert_cmd::{Command, cargo::cargo_bin_cmd};
use cargo_manifest::Manifest;
use expect_test::expect;
use tempfile::tempdir;
use which::which;

static CLANG_BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
    // Read Cargo.toml from pliron-llvm to get the llvm-sys version.
    let manifest = Manifest::from_path(env!("CARGO_MANIFEST_DIR").to_string() + "/../Cargo.toml")
        .expect("Could not read pliron-llvm Cargo.toml");
    let llvm_version = manifest.dependencies.expect("Expected llvm-sys dependency")["llvm-sys"]
        .req()
        .to_string();
    assert!(
        llvm_version.len() == 3,
        "Unexpected llvm-sys version format: Expected two-digit major version and one digit minor version, got {}",
        llvm_version
    );

    let llvm_major_version = &llvm_version[..2];
    let clang_binary_name = format!("clang-{}", llvm_major_version);

    let env_var = env::var("CLANG_BINARY_PATH");
    let is_env_var_set = env_var.is_ok();

    // Use CLANG_BINARY_PATH if set, otherwise default to clang_binary_name
    let binary_to_find = env_var.unwrap_or(clang_binary_name.clone());

    // If CLANG_BINARY_PATH is set and it's an absolute/relative path, do a cheap existence check first
    if is_env_var_set {
        let path = PathBuf::from(&binary_to_find);
        if path.exists() {
            return path;
        }
    }

    // Use which to find the binary in PATH or verify the custom path
    match which(&binary_to_find) {
        Ok(path) => path,
        Err(_) => {
            if is_env_var_set {
                panic!(
                    "Clang binary not found at path specified by CLANG_BINARY_PATH ({}) or in system PATH. Expected LLVM version {}.",
                    binary_to_find, llvm_major_version
                );
            } else {
                panic!(
                    "Clang binary '{}' not found in PATH. Please install LLVM version {} or set the CLANG_BINARY_PATH environment variable to the path of your Clang binary.",
                    clang_binary_name, llvm_major_version
                );
            }
        }
    }
});

static RESOURCES_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    [env!("CARGO_MANIFEST_DIR"), "tests", "resources"]
        .iter()
        .collect()
});

/// Utility to compile and run a C file through Pliron-IR.
/// 1. C file to LLVM-IR
/// 2. LLVM-IR -> Pliron's LLVM dialect -> LLVM-IR
/// 3. Compile the result LLVM-IR to an executable
/// 4. Run the executable and return the Output
fn c_clang_ir_pliron_ir_exe(input_c_file: &str) -> Output {
    // Create a tempdir() to place the temporary compiled files.
    let tmp_dir = tempdir().unwrap();

    // clang -S -emit-llvm -o $tmp/input.ll $input_c_file
    let mut cmd = Command::new(CLANG_BINARY.clone());
    let compile_c = cmd
        .current_dir(&*RESOURCES_DIR)
        .args([
            "-S",
            "-emit-llvm",
            "-o",
            tmp_dir.path().join("input.ll").to_str().unwrap(),
            input_c_file,
        ])
        .output()
        .expect("failed to execute clang to compile C to LLVM-IR");
    assert!(
        compile_c.status.success(),
        "{}",
        String::from_utf8(compile_c.stderr).unwrap()
    );

    // llvm-opt -o $tmp/input.pliron.ll -S -i $tmp/input.ll
    let mut cmd = cargo_bin_cmd!("llvm-opt");
    let compile_pliron = cmd
        .current_dir(&*RESOURCES_DIR)
        .args([
            "-o",
            tmp_dir.path().join("input.pliron.ll").to_str().unwrap(),
            "-S",
            "-i",
            tmp_dir.path().join("input.ll").to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute llvm-opt to compile LLVM-IR via pliron");
    assert!(
        compile_pliron.status.success(),
        "{}",
        String::from_utf8(compile_pliron.stderr).unwrap()
    );

    // clang -o $tmp/input.exe $tmp/input.pliron.ll
    let mut cmd = Command::new(CLANG_BINARY.clone());
    let compile_exe = cmd
        .current_dir(&*RESOURCES_DIR)
        .args([
            "-o",
            tmp_dir.path().join("input.exe").to_str().unwrap(),
            tmp_dir.path().join("input.pliron.ll").to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute clang to final LLVM-IR to executable");
    assert!(
        compile_exe.status.success(),
        "{}",
        String::from_utf8(compile_exe.stderr).unwrap()
    );

    // $tmp/input.exe
    let mut cmd = Command::new(tmp_dir.path().join("input.exe"));
    cmd.output().expect("Error executing compiled program")
}

#[test]
fn test_va_arg() {
    let output = c_clang_ir_pliron_ir_exe("va_arg.c");
    assert_eq!(output.status.code(), Some(75));
}

#[test]
fn test_fib() {
    let output = c_clang_ir_pliron_ir_exe("fib.c");
    assert!(output.status.success());
    let fib_output = String::from_utf8(output.stdout).unwrap();
    expect![[r#"
        fib(0): 0
        fib(1): 0
        fib(2): 1
        fib(3): 1
        fib(4): 2
    "#]]
    .assert_eq(&fib_output);
}
