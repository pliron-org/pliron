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

/// Get the LLI binary path based on the llvm-sys version used.
static LLI_BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
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

    let llvm_major_version = &llvm_version[..2];
    let lli_binary_name = format!("lli-{}", llvm_major_version);

    let env_var = env::var("LLI_BINARY_PATH");
    let is_env_var_set = env_var.is_ok();

    // Use LLI_BINARY_PATH if set, otherwise default to lli_binary_name
    let binary_to_find = env_var.unwrap_or(lli_binary_name.clone());

    // If LLI_BINARY_PATH is set and it's an absolute/relative path, do a cheap existence check first
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
                    "LLI binary not found at path specified by LLI_BINARY_PATH ({}) or in system PATH. Expected LLVM version {}.",
                    binary_to_find, llvm_major_version
                );
            } else {
                panic!(
                    "LLI binary '{}' not found in PATH. Please install LLVM version {} or set the LLI_BINARY_PATH environment variable to the path of your LLI binary.",
                    lli_binary_name, llvm_major_version
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

/// Test an LLVM-IR file by executing it and comparing the output.
/// The input file is `input_file`, which contains LLVM IR / Bitcode.
/// The expected output is `expected_output`.
fn test_llvm_ir_via_pliron(input_file: &str, mut opts: impl Pass, expected_output: i32) {
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
        59,
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

/// Round-trip the atomic and inline-asm constructs through the LLVM bridge
/// (LLVM-IR -> pliron -> LLVM-IR) and verify both representations. The IR is
/// provided inline rather than as a resource file, and is not executed (the
/// test only round-trips and verifies); `out_module.verify()` proves the
/// emitted IR is well-formed.
#[test]
fn test_atomics_and_inline_asm_roundtrip() {
    init_env_logger_for_tests!();

    let input_ir = r#"
define i32 @atomics_asm(ptr %p, i32 %v) {
entry:
  %old = atomicrmw add ptr %p, i32 %v monotonic
  %cx = cmpxchg ptr %p, i32 %old, i32 %v acquire monotonic
  fence seq_cst
  %la = load atomic i32, ptr %p monotonic, align 4
  store atomic i32 %la, ptr %p release, align 4
  %asm = call i32 asm sideeffect "mov $0, $1", "=r,r"(i32 %v)
  ret i32 %asm
}
"#;

    let llvm_context = LLVMContext::default();
    let tmp_dir = tempdir().unwrap();
    let in_path = tmp_dir.path().join("atomics_asm.ll");
    std::fs::write(&in_path, input_ir).unwrap();

    let module = LLVMModule::from_ir_in_file(&llvm_context, in_path.to_str().unwrap())
        .expect("input LLVM IR should parse");

    let ctx = &mut Context::new();
    let pliron_module =
        from_llvm_ir::convert_module(ctx, &module).expect("from_llvm_ir should succeed");
    verify_op(&pliron_module, ctx).expect("reconstructed pliron module should verify");

    let plir = pliron_module.get_operation().disp(ctx).to_string();
    for needle in [
        "llvm.atomicrmw",
        "llvm.cmpxchg",
        "llvm.fence",
        "llvm.atomic_load",
        "llvm.atomic_store",
        "llvm.inline_asm",
    ] {
        assert!(
            plir.contains(needle),
            "pliron IR missing `{needle}`:\n{plir}"
        );
    }

    // Round-trip back to LLVM IR and verify it.
    let out_module = to_llvm_ir::convert_module(ctx, &llvm_context, pliron_module)
        .expect("to_llvm_ir should succeed");
    out_module
        .verify()
        .expect("round-tripped LLVM module should verify");

    let out_ir = llvm_print_module_to_string(&out_module).expect("print module");
    for needle in [
        "atomicrmw add",
        "cmpxchg",
        "fence seq_cst",
        "load atomic",
        "store atomic",
        "asm sideeffect",
    ] {
        assert!(
            out_ir.contains(needle),
            "emitted IR missing `{needle}`:\n{out_ir}"
        );
    }
}
