// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around llvm_sys::target

use crate::llvm_sys::ToBool;
use llvm_sys::target::{
    LLVM_InitializeAllAsmParsers, LLVM_InitializeAllAsmPrinters, LLVM_InitializeAllDisassemblers,
    LLVM_InitializeAllTargetInfos, LLVM_InitializeAllTargetMCs, LLVM_InitializeAllTargets,
    LLVM_InitializeNativeAsmParser, LLVM_InitializeNativeAsmPrinter,
    LLVM_InitializeNativeDisassembler, LLVM_InitializeNativeTarget,
};
use std::sync::{Mutex, PoisonError};

/// Exclusive access to LLVM's target registry.
static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

/// Do `f` with exclusive access to LLVM's target registry.
fn with_registry_lock<R>(f: impl FnOnce() -> R) -> R {
    // The lock protects LLVM's registry, not Rust data. A panic in another
    // thread thus leaves no invalid state here. Ignore the poison flag.
    let _guard = REGISTRY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    f()
}

/// LLVM_InitializeAllTargetInfos
pub fn llvm_initialize_all_target_infos() {
    with_registry_lock(|| unsafe {
        LLVM_InitializeAllTargetInfos();
    });
}

/// LLVM_InitializeAllTargets
pub fn llvm_initialize_all_targets() {
    with_registry_lock(|| unsafe {
        LLVM_InitializeAllTargets();
    });
}

/// LLVM_InitializeAllTargetMCs
pub fn llvm_initialize_all_target_mcs() {
    with_registry_lock(|| unsafe {
        LLVM_InitializeAllTargetMCs();
    });
}

/// LLVM_InitializeAllAsmPrinters
pub fn llvm_initialize_all_asm_printers() {
    with_registry_lock(|| unsafe {
        LLVM_InitializeAllAsmPrinters();
    });
}

/// LLVM_InitializeAllAsmParsers
pub fn llvm_initialize_all_asm_parsers() {
    with_registry_lock(|| unsafe {
        LLVM_InitializeAllAsmParsers();
    });
}

/// LLVM_InitializeAllDisassemblers
pub fn llvm_initialize_all_disassemblers() {
    with_registry_lock(|| unsafe {
        LLVM_InitializeAllDisassemblers();
    });
}

/// LLVM_InitializeNativeTarget
pub fn llvm_initialize_native_target() -> Result<(), String> {
    if !with_registry_lock(|| unsafe { LLVM_InitializeNativeTarget().to_bool() }) {
        Ok(())
    } else {
        Err("Failed to initialize native target".to_string())
    }
}

/// LLVM_InitializeNativeAsmParser
pub fn llvm_initialize_native_asm_parser() -> Result<(), String> {
    if !with_registry_lock(|| unsafe { LLVM_InitializeNativeAsmParser().to_bool() }) {
        Ok(())
    } else {
        Err("Failed to initialize native asm parser".to_string())
    }
}

/// LLVM_InitializeNativeAsmParser
pub fn llvm_initialize_native_asm_printer() -> Result<(), String> {
    if !with_registry_lock(|| unsafe { LLVM_InitializeNativeAsmPrinter().to_bool() }) {
        Ok(())
    } else {
        Err("Failed to initialize native asm printer".to_string())
    }
}

/// LLVM_InitializeNativeDisassembler
pub fn llvm_initialize_native_disassembler() -> Result<(), String> {
    if !with_registry_lock(|| unsafe { LLVM_InitializeNativeDisassembler().to_bool() }) {
        Ok(())
    } else {
        Err("Failed to initialize native disassembler".to_string())
    }
}

/// Initialize native everything.
pub fn initialize_native() -> Result<(), String> {
    llvm_initialize_native_target()?;
    llvm_initialize_native_asm_printer()?;
    llvm_initialize_native_asm_parser()?;
    llvm_initialize_native_disassembler()?;
    Ok(())
}

/// Initialize all targets everything.
pub fn initialize_all() {
    llvm_initialize_all_target_infos();
    llvm_initialize_all_targets();
    llvm_initialize_all_target_mcs();
    llvm_initialize_all_asm_printers();
    llvm_initialize_all_asm_parsers();
    llvm_initialize_all_disassemblers();
}
