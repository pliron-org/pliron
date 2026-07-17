//! Tests that compile code and run it.

#![cfg(feature = "llvm-sys")]

use std::{env, path::PathBuf, sync::LazyLock};
use which::which;

use assert_cmd::Command;
use cargo_manifest::Manifest;
use pliron::{
    arg_error_noloc,
    builtin::ops::ModuleOp,
    combine::Parser,
    context::Context,
    init_env_logger_for_tests, location,
    op::{Op, verify_op},
    operation::{Operation, verify_operation},
    parsable::{self, state_stream_from_iterator},
    pass::{AnalysisManager, OpPass, Pass, Passes},
    printable::Printable,
};
use pliron_llvm::{
    from_llvm_ir,
    llvm_sys::core::{LLVMContext, LLVMModule, llvm_print_module_to_string},
    to_llvm_ir,
};
use tempfile::tempdir;

/// Get the LLVM major version used, based on the llvm-sys dependency version.
fn llvm_major_version() -> String {
    let manifest =
        Manifest::from_path(env!("CARGO_MANIFEST_PATH")).expect("Could not read Cargo.toml");
    let llvm_version = manifest.dependencies.expect("Expected llvm-sys dependency")["llvm-sys"]
        .req()
        .to_string();
    assert!(
        llvm_version.len() == 3,
        "Unexpected llvm-sys version format: Expected two-digit major version and one digit minor version, got {}",
        llvm_version
    );
    llvm_version[..2].to_string()
}

/// Locate an LLVM tool binary (e.g. `lli`, `clang`) matching the llvm-sys version used.
/// `path_env_var` (e.g. `LLI_BINARY_PATH`), if set, overrides the default `<tool_name>-<major>`
/// lookup and may itself be a path to the binary.
fn find_llvm_tool(tool_name: &str, path_env_var: &str) -> PathBuf {
    let llvm_major_version = llvm_major_version();
    let default_binary_name = format!("{}-{}", tool_name, llvm_major_version);

    let env_var = env::var(path_env_var);
    let is_env_var_set = env_var.is_ok();

    // Use path_env_var if set, otherwise default to default_binary_name
    let binary_to_find = env_var.unwrap_or(default_binary_name.clone());

    // If path_env_var is set and it's an absolute/relative path, do a cheap existence check first
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
                    "{} binary not found at path specified by {} ({}) or in system PATH. Expected LLVM version {}.",
                    tool_name, path_env_var, binary_to_find, llvm_major_version
                );
            } else {
                panic!(
                    "{} binary '{}' not found in PATH. Please install LLVM version {} or set the {} environment variable to the path of your {} binary.",
                    tool_name, default_binary_name, llvm_major_version, path_env_var, tool_name
                );
            }
        }
    }
}

/// The LLI binary path, based on the llvm-sys version used.
static LLI_BINARY: LazyLock<PathBuf> = LazyLock::new(|| find_llvm_tool("lli", "LLI_BINARY_PATH"));

/// The clang binary path, based on the llvm-sys version used.
static CLANG_BINARY: LazyLock<PathBuf> =
    LazyLock::new(|| find_llvm_tool("clang", "CLANG_BINARY_PATH"));

static RESOURCES_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    [env!("CARGO_MANIFEST_DIR"), "tests", "resources"]
        .iter()
        .collect()
});

/// Convert `input_file` (LLVM IR / Bitcode) to pliron and back, running `opts`
/// on the pliron module in between, and write out the resulting bitcode.
/// Returns the `TempDir` holding the bitcode (kept alive by the caller) along
/// with the path to the bitcode file within it.
fn build_bitcode(input_file: &str, mut opts: impl Pass) -> (tempfile::TempDir, PathBuf) {
    let llvm_context = LLVMContext::default();
    let module = match LLVMModule::from_ir_in_file(&llvm_context, input_file) {
        Ok(module) => module,
        Err(err) => {
            eprintln!("{err}");
            panic!("Error reading {input_file}");
        }
    };

    let ctx = &mut Context::new();
    let pliron_module = match from_llvm_ir::convert_module(ctx, &module) {
        Ok(plir) => plir,
        Err(err) => {
            eprintln!("{}", err.disp(ctx));
            panic!("Error converting {input_file}");
        }
    };

    log::debug!(
        "pliron module constructed from input LLVM-IR:\n{}",
        pliron_module.disp(ctx)
    );

    match verify_op(&pliron_module, ctx) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("{}", err.disp(ctx));
            panic!("Error verifying {input_file}");
        }
    }

    // Write the plir to a file.
    let tmp_dir = tempdir().unwrap();
    let plir_path = tmp_dir.path().join("output.plir");
    // Write the plir to a file.
    std::fs::write(
        plir_path.clone(),
        pliron_module.get_operation().disp(ctx).to_string(),
    )
    .map_err(|e| arg_error_noloc!(e))
    .unwrap();

    // Parse the plir file and verify it.
    let plir_file = std::fs::File::open(&plir_path).unwrap();
    let mut plir_file = std::io::BufReader::new(plir_file);
    use utf8_chars::BufReadCharsExt;
    let chars_iter = plir_file.chars().map(|c| {
        c.inspect_err(|e| eprint!("Error reading chars from file: {e}"))
            .unwrap()
    });

    let source = location::Source::new_from_file(ctx, plir_path.to_str().unwrap());
    let state_stream = state_stream_from_iterator(chars_iter, parsable::State::new(ctx, source));

    let parsed_res = match Operation::top_level_parser().parse(state_stream) {
        Ok((parsed_res, _)) => parsed_res,
        Err(err) => {
            eprintln!("{err}");
            panic!("Error parsing {}", plir_path.to_str().unwrap());
        }
    };

    log::debug!(
        "pliron module re-parsed after printing:\n{}",
        parsed_res.disp(ctx)
    );

    match verify_operation(parsed_res, ctx) {
        Ok(_) => (),
        Err(err) => {
            eprintln!("{}", err.disp(ctx));
            panic!("Error verifying {}", plir_path.to_str().unwrap());
        }
    }

    if let Err(err) = opts.run(parsed_res, ctx, &mut AnalysisManager::default()) {
        eprintln!("Error during optimization: {}", err.disp(ctx));
        panic!("Error during optimization");
    }

    let parsed_module_op = Operation::get_op::<ModuleOp>(parsed_res, ctx)
        .expect("Parsed operation must be a ModuleOp");

    // Execute it and try.
    let module = match to_llvm_ir::convert_module(ctx, &llvm_context, parsed_module_op) {
        Ok(module) => module,
        Err(err) => {
            eprintln!("{}", err.disp(ctx));
            panic!("Error converting {}", plir_path.to_str().unwrap());
        }
    };

    log::debug!(
        "LLVM module generated from pliron LLVM-IR:\n{}",
        llvm_print_module_to_string(&module).unwrap()
    );

    match module.verify() {
        Ok(_) => (),
        Err(err) => {
            eprintln!("{err}");
            panic!("Error verifying {}", plir_path.to_str().unwrap());
        }
    }

    // Write the bitcode to a file.
    let bc_path = tmp_dir.path().join("output.bc");
    module
        .bitcode_to_file(bc_path.to_str().unwrap())
        .map_err(|_err| arg_error_noloc!("{}", "Error writing bitcode to file"))
        .unwrap();

    (tmp_dir, bc_path)
}

/// Test an LLVM-IR file by executing it with `lli` and comparing the output.
/// The input file is `input_file`, which contains LLVM IR / Bitcode.
/// The expected output is `expected_output`.
fn test_llvm_ir_via_pliron(input_file: &str, opts: impl Pass, expected_output: i32) {
    let (_tmp_dir, bc_path) = build_bitcode(input_file, opts);

    let mut cmd = Command::new(LLI_BINARY.clone());

    let run_output = cmd
        .current_dir(&*RESOURCES_DIR)
        .args([bc_path.to_str().unwrap()])
        .output()
        .expect("failed to execute LLi to execute output.bc");
    assert_eq!(
        run_output.status.code(),
        Some(expected_output),
        "{}",
        String::from_utf8(run_output.stderr).unwrap()
    );
}

/// Test an LLVM-IR file by compiling it to a native executable with `clang`
/// and running that, instead of interpreting it with `lli`.
/// Useful for IR constructs (e.g. cross-function `blockaddress` materialization)
/// that `lli`'s lazy JIT can't handle but that are well defined when statically
/// compiled and linked.
/// The input file is `input_file`, which contains LLVM IR / Bitcode.
/// The expected output is `expected_output`.
fn test_llvm_ir_via_pliron_compiled(input_file: &str, opts: impl Pass, expected_output: i32) {
    let (tmp_dir, bc_path) = build_bitcode(input_file, opts);

    let exe_path = tmp_dir.path().join("a.out");
    let compile_output = Command::new(CLANG_BINARY.clone())
        .args([bc_path.to_str().unwrap(), "-o", exe_path.to_str().unwrap()])
        .output()
        .expect("failed to execute clang to compile output.bc");
    assert!(
        compile_output.status.success(),
        "clang failed to compile {}: {}",
        bc_path.to_str().unwrap(),
        String::from_utf8(compile_output.stderr).unwrap()
    );

    let run_output = Command::new(exe_path)
        .current_dir(&*RESOURCES_DIR)
        .output()
        .expect("failed to execute compiled a.out");
    assert_eq!(
        run_output.status.code(),
        Some(expected_output),
        "{}",
        String::from_utf8(run_output.stderr).unwrap()
    );
}

fn create_opt_pass_manager() -> impl Pass {
    let mut passes = OpPass::<ModuleOp, Passes>::default();
    pliron_llvm::append_o1_passes(&mut passes);
    passes
}

/// Test simple-loop by compiling simple-loop.ll via pliron.
#[test]
fn test_simple_loop() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("simple-loop.ll").to_str().unwrap(),
        Passes::default(),
        15,
    );

    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("simple-loop.ll").to_str().unwrap(),
        create_opt_pass_manager(),
        15,
    );
}

/// Test insert_extract_value by compiling insert_extract_value.ll via pliron.
#[test]
fn test_insert_extract_value() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR
            .join("insert_extract_value.ll")
            .to_str()
            .unwrap(),
        create_opt_pass_manager(),
        103,
    );
}

/// Test SelectOp by compiling select.ll via pliron.
#[test]
fn test_select() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("select.ll").to_str().unwrap(),
        Passes::default(),
        100,
    );
}

/// Test blockaddress by compiling blockaddress.ll via pliron.
/// `lli` can't process cross-function blockaddress materialization, so this
/// is compiled to a native executable with clang instead of run under `lli`.
#[test]
fn test_blockaddress() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron_compiled(
        RESOURCES_DIR.join("blockaddress.ll").to_str().unwrap(),
        Passes::default(),
        6,
    );
}

/// Test SwitchOp by compiling switch.ll via pliron.
#[test]
fn test_switch() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("switch.ll").to_str().unwrap(),
        Passes::default(),
        68,
    );
}

/// Test const structs and arrays
#[test]
fn test_consts() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("consts.ll").to_str().unwrap(),
        Passes::default(),
        203,
    );
}

/// Test globals
#[test]
fn test_globals() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("globals.ll").to_str().unwrap(),
        Passes::default(),
        64,
    );
}

/// Test casts
#[test]
fn test_casts() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("casts.ll").to_str().unwrap(),
        Passes::default(),
        88,
    );
}

/// Test fib by compiling fib.ll via pliron.
#[test]
fn test_fib() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("fib.ll").to_str().unwrap(),
        Passes::default(),
        3,
    );
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("fib.ll").to_str().unwrap(),
        create_opt_pass_manager(),
        3,
    );
}

/// Test fib.mem2reg by compiling fib.ll via pliron.
#[test]
fn test_fib_mem2reg() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("fib.mem2reg.ll").to_str().unwrap(),
        Passes::default(),
        5,
    );
}

/// Test floating point operations by compiling fpops.ll via pliron
#[test]
fn test_fpops() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("fpops.ll").to_str().unwrap(),
        Passes::default(),
        45,
    );
}

/// Test intrinsics by compiling intrinsics.ll via pliron
#[test]
fn test_intrinsics() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("intrinsics.ll").to_str().unwrap(),
        Passes::default(),
        66,
    );
}

/// Test `va_arg` by compiling va_arg.ll via pliron.
/// `va_arg` is poorly supported in LLVM.
/// If the test fails on non unix-x86_64 platforms, that wouldn't be surprising.
/// We'll need to fix it then.
#[test]
fn test_va_arg() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("va_arg.ll").to_str().unwrap(),
        Passes::default(),
        75,
    );
}

/// Test indirect-call by compiling indirect_call.ll via pliron
#[test]
fn test_indirect_call() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("indirect_call.ll").to_str().unwrap(),
        Passes::default(),
        84,
    );
}

/// Test vector operations by compiling vector_ops.ll via pliron
#[test]
fn test_vector_ops() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR.join("vector_ops.ll").to_str().unwrap(),
        Passes::default(),
        0,
    );
}

/// Test atomics and inline assembly by compiling atomics_inline_asm.ll via pliron
#[test]
fn test_atomics_and_inline_asm() {
    init_env_logger_for_tests!();
    test_llvm_ir_via_pliron(
        RESOURCES_DIR
            .join("atomics_inline_asm.ll")
            .to_str()
            .unwrap(),
        Passes::default(),
        33,
    );
}
