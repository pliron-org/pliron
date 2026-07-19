// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! JIT compilation example for Kaleidoscope using pliron-llvm

use pliron::{
    builtin::{op_interfaces::SingleBlockRegionInterface, ops::ModuleOp},
    context::Context,
    input_error_noloc,
    op::Op,
    operation::verify_operation,
    result::Result,
};
use pliron_llvm::{
    llvm_sys::{
        core::{
            LLVMContext, LLVMModule, llvm_get_named_function, llvm_get_param_types,
            llvm_get_return_type, llvm_global_get_value_type, llvm_int_type_in_context,
        },
        target::initialize_native,
    },
    to_llvm_ir,
};

use crate::{ast::parse_program, from_ast::lower_function, to_llvm::lower_module};

// Lower a Kaleidoscope program to LLVM dialect and return the module operation
// ANCHOR: lower_to_llvm_ir
fn lower_to_llvm_ir(src: &str, llvm_ctx: &LLVMContext) -> Result<LLVMModule> {
    let funcs =
        parse_program(src).map_err(|e| input_error_noloc!("Failed to parse program: {}", e))?;
    let ctx = &mut Context::new();
    let module = ModuleOp::new(ctx, "test".try_into().expect("valid module name"));
    for func in &funcs {
        let func_op = lower_function(ctx, func)?;
        module.append_operation(ctx, func_op.get_operation(), 0);
    }
    lower_module(ctx, module)?;
    verify_operation(module.get_operation(), ctx)?;
    // Convert from LLVM dialect to LLVM IR
    let llvm_module = to_llvm_ir::convert_module(ctx, llvm_ctx, module)?;
    llvm_module
        .verify()
        .map_err(|e| input_error_noloc!("Generated LLVM module is invalid: {}", e))?;
    Ok(llvm_module)
}
// ANCHOR_END: lower_to_llvm_ir

/// Execute the function `name` of a Kaleidoscope program using JIT compilation
/// The function must have the signature `fn(i64) -> i64`
// ANCHOR: exec_fn
pub fn exec_fn(src: &str, name: &str, arg: i64) -> Result<i64> {
    initialize_native()
        .map_err(|e| input_error_noloc!("Failed to initialize native target: {}", e))?;
    let llvm_ctx = LLVMContext::default();
    let llvm_module = lower_to_llvm_ir(src, &llvm_ctx)?;

    let Some(f) = llvm_get_named_function(&llvm_module, name) else {
        return Err(input_error_noloc!(
            "Function '{}' not found in generated LLVM module",
            name
        ));
    };
    let f_ty = llvm_global_get_value_type(f);
    let param_types = llvm_get_param_types(f_ty);
    let ret_type = llvm_get_return_type(f_ty);
    let llvm_int64_ty = llvm_int_type_in_context(&llvm_ctx, 64);
    if param_types.len() != 1 || param_types[0] != llvm_int64_ty || ret_type != llvm_int64_ty {
        return Err(input_error_noloc!(
            "Expected function '{}' to have exactly one parameter of type i64 and return type i64, but found different signature",
            name
        ));
    }

    // println!("Generated LLVM IR:\n{}", llvm_module.to_string());

    // JIT compile and execute the main function
    let lljit = pliron_llvm::llvm_sys::lljit::LLVMLLJIT::new_with_default_builder()
        .map_err(|e| input_error_noloc!("Failed to create JIT execution engine: {}", e))?;
    lljit
        .add_module(llvm_module)
        .map_err(|e| input_error_noloc!("Failed to add module to JIT: {}", e))?;
    let main_fn = lljit
        .lookup_symbol(name)
        .map_err(|e| input_error_noloc!("Failed to find main function in JIT: {}", e))?;

    let main_fn: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(main_fn) };
    Ok(main_fn(arg))
}
// ANCHOR_END: exec_fn

#[cfg(test)]
mod tests {
    use super::*;

    // ANCHOR: fibonacci_jit_test
    #[test]
    fn fibonacci_jit() {
        let src = std::fs::read_to_string("examples/kaleidoscope/fibonacci.kal")
            .expect("failed to read fibonacci.kal");
        let result = exec_fn(&src, "main", 5).expect("failed to execute main function");
        assert_eq!(result, 5);
    }
    // ANCHOR_END: fibonacci_jit_test

    // ANCHOR: factorial_jit_test
    #[test]
    fn factorial_jit() {
        let src = std::fs::read_to_string("examples/kaleidoscope/factorial.kal")
            .expect("failed to read factorial.kal");
        let result = exec_fn(&src, "main", 5).expect("failed to execute main function");
        assert_eq!(result, 120);
    }
    // ANCHOR_END: factorial_jit_test

    // ANCHOR: if_else_jit_test
    #[test]
    fn if_else_jit() {
        let src = "
            def abs(x) {
                var result = 0;
                if x < 0 {
                    result = 0 - x;
                } else {
                    result = x;
                }
                return result;
            }
        ";
        let result = exec_fn(src, "abs", 42).expect("failed to execute main function");
        assert_eq!(result, 42);
        let result = exec_fn(src, "abs", -42).expect("failed to execute main function");
        assert_eq!(result, 42);
        let result = exec_fn(src, "abs", 0).expect("failed to execute main function");
        assert_eq!(result, 0);
    }
    // ANCHOR_END: if_else_jit_test
}
