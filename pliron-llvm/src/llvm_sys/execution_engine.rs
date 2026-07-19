// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around llvm_sys::execution_engine

use std::mem::{MaybeUninit, forget};

use llvm_sys::{
    core::LLVMDisposeMessage,
    execution_engine::{
        LLVMAddGlobalMapping, LLVMCreateExecutionEngineForModule, LLVMCreateGenericValueOfFloat,
        LLVMCreateGenericValueOfInt, LLVMCreateGenericValueOfPointer,
        LLVMCreateInterpreterForModule, LLVMCreateJITCompilerForModule,
        LLVMCreateMCJITCompilerForModule, LLVMDisposeExecutionEngine, LLVMDisposeGenericValue,
        LLVMExecutionEngineRef, LLVMFindFunction, LLVMGenericValueRef, LLVMGenericValueToFloat,
        LLVMGenericValueToInt, LLVMGenericValueToPointer, LLVMInitializeMCJITCompilerOptions,
        LLVMLinkInInterpreter, LLVMLinkInMCJIT, LLVMMCJITCompilerOptions, LLVMRunFunction,
        LLVMRunFunctionAsMain, LLVMRunStaticConstructors, LLVMRunStaticDestructors,
    },
};

use crate::llvm_sys::{
    core::{LLVMModule, LLVMType, LLVMValue},
    cstr_to_string, to_c_str,
};

/// Wrapper around [LLVMGenericValueRef].
pub struct GenericValue(LLVMGenericValueRef);

impl GenericValue {
    /// Creates a [GenericValue] representing an integer.
    pub fn from_u64(ty: LLVMType, n: u64, is_signed: bool) -> GenericValue {
        let gv = unsafe { LLVMCreateGenericValueOfInt(ty.into(), n, is_signed.into()) };
        GenericValue(gv)
    }

    /// Creates a [GenericValue] representing a pointer.
    pub fn from_pointer<T>(ptr: *mut T) -> GenericValue {
        let gv = unsafe { LLVMCreateGenericValueOfPointer(ptr as _) };
        GenericValue(gv)
    }

    /// Creates a [GenericValue] representing a float.
    pub fn from_f64(ty: LLVMType, n: f64) -> GenericValue {
        let gv = unsafe { LLVMCreateGenericValueOfFloat(ty.into(), n) };
        GenericValue(gv)
    }

    /// Returns [Self] as u64.
    pub fn to_u64(&self) -> u64 {
        unsafe { LLVMGenericValueToInt(self.0, 0) as u64 }
    }

    /// Returns [Self] as pointer.
    pub fn to_pointer<T>(&self) -> *mut T {
        unsafe { LLVMGenericValueToPointer(self.0) as *mut T }
    }

    /// Returns [Self] as f64.
    pub fn to_f64(&self, ty: LLVMType) -> f64 {
        unsafe { LLVMGenericValueToFloat(ty.into(), self.0) }
    }
}

impl Drop for GenericValue {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeGenericValue(self.0);
        }
    }
}

/// Rust wrapper around [LLVMMCJITCompilerOptions]
#[derive(Clone, Debug, Copy)]
pub struct MCJITCompilerOptions(pub LLVMMCJITCompilerOptions);

impl Default for MCJITCompilerOptions {
    fn default() -> Self {
        let mut options = MaybeUninit::uninit();
        unsafe {
            LLVMInitializeMCJITCompilerOptions(
                options.as_mut_ptr(),
                std::mem::size_of::<LLVMMCJITCompilerOptions>(),
            );
            MCJITCompilerOptions(options.assume_init())
        }
    }
}

/// Rust wrapper around [LLVMExecutionEngineRef]
pub struct ExecutionEngine(LLVMExecutionEngineRef);

/// Code generation optimization level.
/// This is a copy of LLVM's `CodeGenOptLevel` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeGenOptLevel {
    /// -O0
    None = 0,
    /// -O1
    Less = 1,
    /// -O2, -Os
    Default = 2,
    /// -O3
    Aggressive = 3,
}

/// Kind of execution engine to create.
#[derive(Clone, Copy, Debug)]
pub enum EngineKind {
    JIT(CodeGenOptLevel),
    Interpreter,
    // Either of JIT or Interpreter.
    Either,
    MCJIT(MCJITCompilerOptions),
}

impl ExecutionEngine {
    /// Creates a new [ExecutionEngine] for the given module.
    ///
    /// ### Example usage:
    /// ```
    /// use pliron_llvm::llvm_sys::{
    ///     core::{LLVMMemoryBuffer, LLVMContext, LLVMModule},
    ///     execution_engine::{ExecutionEngine, EngineKind, MCJITCompilerOptions},
    ///     target::initialize_native,
    /// };
    /// fn main() -> Result<(), String> {
    ///     let llvm_ctx = LLVMContext::default();
    ///     ExecutionEngine::link_in_mcjit();
    ///     initialize_native()?;
    ///
    ///     let llvm_ir = r#"
    ///         define i32 @main() {
    ///             ret i32 0
    ///         }"#;
    ///     let ir_mb = LLVMMemoryBuffer::from_str(llvm_ir, "test_buffer");
    ///     let module = LLVMModule::from_ir_in_memory_buffer(&llvm_ctx, ir_mb)?;
    ///
    ///     let mcjit_options = MCJITCompilerOptions::default();
    ///     let ee = ExecutionEngine::new_for_module(module, EngineKind::MCJIT(mcjit_options))?;
    ///     let main = ee
    ///         .find_function("main")
    ///         .ok_or("Function 'main' not found")?;
    ///     let ret_gv = unsafe { ee.run_function_as_main(main, &[]) };
    ///     assert_eq!(ret_gv, 0);
    ///     Ok(())
    /// }
    /// ```
    pub fn new_for_module(module: LLVMModule, kind: EngineKind) -> Result<Self, String> {
        let mut ee = MaybeUninit::uninit();
        let mut error_string = MaybeUninit::uninit();
        let result = unsafe {
            match kind {
                EngineKind::Either => LLVMCreateExecutionEngineForModule(
                    ee.as_mut_ptr(),
                    module.inner_ref(),
                    error_string.as_mut_ptr(),
                ),
                EngineKind::Interpreter => LLVMCreateInterpreterForModule(
                    ee.as_mut_ptr(),
                    module.inner_ref(),
                    error_string.as_mut_ptr(),
                ),
                EngineKind::JIT(opt_level) => LLVMCreateJITCompilerForModule(
                    ee.as_mut_ptr(),
                    module.inner_ref(),
                    opt_level as _,
                    error_string.as_mut_ptr(),
                ),
                EngineKind::MCJIT(options) => LLVMCreateMCJITCompilerForModule(
                    ee.as_mut_ptr(),
                    module.inner_ref(),
                    // This is ok, `LLVMCreateMCJITCompilerForModule` creates a copy.
                    &options.0 as *const LLVMMCJITCompilerOptions as *mut LLVMMCJITCompilerOptions,
                    std::mem::size_of::<LLVMMCJITCompilerOptions>(),
                    error_string.as_mut_ptr(),
                ),
            }
        };
        // We can forget the module, as the execution engine now owns it.
        forget(module);
        if result != 0 {
            unsafe {
                let err_str = error_string.assume_init();
                let err_string = cstr_to_string(err_str).unwrap();
                LLVMDisposeMessage(err_str);
                Err(err_string)
            }
        } else {
            let ee = unsafe { ee.assume_init() };
            Ok(ExecutionEngine(ee))
        }
    }

    /// Runs static constructors.
    ///
    /// ### Safety
    /// This function executes arbitrary code.
    pub unsafe fn run_static_constructors(&self) {
        unsafe {
            LLVMRunStaticConstructors(self.0);
        }
    }

    /// Runs static destructors.
    ///
    /// ### Safety
    /// This function executes arbitrary code.
    pub unsafe fn run_static_destructors(&self) {
        unsafe {
            LLVMRunStaticDestructors(self.0);
        }
    }

    /// Find a function by name.
    pub fn find_function(&self, name: &str) -> Option<LLVMValue> {
        let mut func = MaybeUninit::uninit();
        let result =
            unsafe { LLVMFindFunction(self.0, to_c_str(name).as_ptr(), func.as_mut_ptr()) };
        if result == 0 {
            let func = unsafe { func.assume_init() };
            Some(func.into())
        } else {
            None
        }
    }

    /// Map an external global into the execution engine.
    ///
    /// ### Example usage:
    /// ```
    /// use pliron_llvm::llvm_sys::{
    ///     core::{LLVMMemoryBuffer, LLVMContext, LLVMModule, llvm_get_named_function},
    ///     execution_engine::{ExecutionEngine, EngineKind, CodeGenOptLevel},
    ///     target::initialize_native,
    /// };
    /// fn main() -> Result<(), String> {
    ///     let llvm_ctx = LLVMContext::default();
    ///     ExecutionEngine::link_in_mcjit();
    ///     initialize_native()?;
    ///
    ///     let llvm_ir = r#"
    ///         declare i32 @external_function()
    ///
    ///         define i32 @call_external() {
    ///             %result = call i32 @external_function()
    ///             ret i32 %result
    ///         }"#;
    ///     let ir_mb = LLVMMemoryBuffer::from_str(llvm_ir, "test_buffer");
    ///     let module = LLVMModule::from_ir_in_memory_buffer(&llvm_ctx, ir_mb)?;
    ///     let ext_fn_value = llvm_get_named_function(&module, "external_function")
    ///         .ok_or("Function 'external_function' not found")?;
    ///
    ///     let ee =
    ///         ExecutionEngine::new_for_module(module, EngineKind::JIT(CodeGenOptLevel::Default))?;
    ///     let call_external_fn = ee
    ///         .find_function("call_external")
    ///         .ok_or("Function 'call_external' not found")?;
    ///
    ///     extern "C" fn external_function() -> i32 {
    ///         1234
    ///     }
    ///
    ///     ee.add_global_mapping(ext_fn_value, external_function as *mut fn() -> i32);
    ///
    ///     let ret_gv = unsafe { ee.run_function(call_external_fn, &[]) };
    ///     let ret_val = ret_gv.to_u64();
    ///     assert_eq!(ret_val, 1234);
    ///     Ok(())
    /// }
    /// ```
    pub fn add_global_mapping<T>(&self, function: LLVMValue, addr: *mut T) {
        unsafe {
            LLVMAddGlobalMapping(self.0, function.into(), addr as _);
        }
    }

    /// Execute the function with the given arguments.
    ///
    /// ### Safety
    /// This function executes arbitrary code.
    ///
    /// ### Example usage:
    /// ```
    /// use pliron_llvm::llvm_sys::{
    ///     core::{LLVMMemoryBuffer, LLVMContext, LLVMModule, llvm_int_type_in_context},
    ///     execution_engine::{ExecutionEngine, EngineKind, GenericValue},
    ///     target::initialize_native,
    /// };
    /// fn main() -> Result<(), String> {
    ///     let llvm_ctx = LLVMContext::default();
    ///     ExecutionEngine::link_in_interpreter();
    ///     initialize_native()?;
    ///
    ///     let llvm_ir = r#"
    ///         define i32 @add(i32 %a, i32 %b) {
    ///             %sum = add i32 %a, %b
    ///             ret i32 %sum
    ///         }"#;
    ///     let ir_mb = LLVMMemoryBuffer::from_str(llvm_ir, "test_buffer");
    ///     let module = LLVMModule::from_ir_in_memory_buffer(&llvm_ctx, ir_mb)?;
    ///     let ee = ExecutionEngine::new_for_module(module, EngineKind::Interpreter)?;
    ///     let add_fn = ee.find_function("add").ok_or("Function 'add' not found")?;
    ///
    ///     let i32_type = llvm_int_type_in_context(&llvm_ctx, 32);
    ///     let arg1 = GenericValue::from_u64(i32_type, 10, false);
    ///     let arg2 = GenericValue::from_u64(i32_type, 32, false);
    ///     let ret_gv = unsafe { ee.run_function(add_fn, &[arg1, arg2]) };
    ///     let ret_val = ret_gv.to_u64();
    ///     assert_eq!(ret_val, 42);
    ///     Ok(())
    /// }
    /// ```
    pub unsafe fn run_function(&self, function: LLVMValue, args: &[GenericValue]) -> GenericValue {
        let mut args = args
            .iter()
            .map(|gv| gv.0)
            .collect::<Vec<LLVMGenericValueRef>>();
        let gv = unsafe {
            LLVMRunFunction(
                self.0,
                function.into(),
                args.len().try_into().unwrap(),
                args.as_mut_ptr(),
            )
        };
        GenericValue(gv)
    }

    /// Execute a function as main.
    ///
    /// ### Safety
    /// This function executes arbitrary code.
    //
    /// ### Example usage:
    /// ```
    /// use pliron_llvm::llvm_sys::{
    ///     core::{LLVMMemoryBuffer, LLVMContext, LLVMModule},
    ///     execution_engine::{ExecutionEngine, EngineKind},
    ///     target::initialize_native,
    /// };
    /// fn main() -> Result<(), String> {
    ///     let llvm_ctx = LLVMContext::default();
    ///     ExecutionEngine::link_in_interpreter();
    ///     initialize_native()?;
    ///
    ///     let llvm_ir = r#"
    ///         define i32 @main() {
    ///             ret i32 42
    ///         }"#;
    ///     let ir_mb = LLVMMemoryBuffer::from_str(llvm_ir, "test_buffer");
    ///     let module = LLVMModule::from_ir_in_memory_buffer(&llvm_ctx, ir_mb)?;
    ///     let ee = ExecutionEngine::new_for_module(module, EngineKind::Interpreter)?;
    ///     let main = ee
    ///         .find_function("main")
    ///         .ok_or("Function 'main' not found")?;
    ///     let ret_gv = unsafe { ee.run_function_as_main(main, &[]) };
    ///     assert_eq!(ret_gv, 42);
    ///     Ok(())
    /// }
    /// ```
    pub unsafe fn run_function_as_main(&self, function: LLVMValue, args: &[&str]) -> i32 {
        let c_args: Vec<_> = args.iter().map(|arg| to_c_str(arg)).collect();
        let mut c_args_ptrs: Vec<_> = c_args.iter().map(|cstr| cstr.as_ptr()).collect();
        unsafe {
            LLVMRunFunctionAsMain(
                self.0,
                function.into(),
                c_args_ptrs.len().try_into().unwrap(),
                c_args_ptrs.as_mut_ptr(),
                // TODO: Support env variables.
                [std::ptr::null()].as_ptr(),
            )
        }
    }

    /// Link in Interpreter.
    pub fn link_in_interpreter() {
        unsafe {
            LLVMLinkInInterpreter();
        }
    }

    /// Link in MCJIT.
    pub fn link_in_mcjit() {
        unsafe {
            LLVMLinkInMCJIT();
        }
    }
}

impl Drop for ExecutionEngine {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeExecutionEngine(self.0);
        }
    }
}
