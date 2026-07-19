// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around llvm_sys::target

use llvm_sys::target::{
    LLVM_InitializeAllAsmParsers, LLVM_InitializeAllAsmPrinters, LLVM_InitializeAllDisassemblers,
    LLVM_InitializeAllTargetInfos, LLVM_InitializeAllTargetMCs, LLVM_InitializeAllTargets,
    LLVM_InitializeNativeAsmParser, LLVM_InitializeNativeAsmPrinter,
    LLVM_InitializeNativeDisassembler, LLVM_InitializeNativeTarget,
};

use crate::llvm_sys::ToBool;

/// LLVM_InitializeAllTargetInfos
pub fn llvm_initialize_all_target_infos() {
    unsafe {
        LLVM_InitializeAllTargetInfos();
    }
}

/// LLVM_InitializeAllTargets
pub fn llvm_initialize_all_targets() {
    unsafe {
        LLVM_InitializeAllTargets();
    }
}

/// LLVM_InitializeAllTargetMCs
pub fn llvm_initialize_all_target_mcs() {
    unsafe {
        LLVM_InitializeAllTargetMCs();
    }
}

/// LLVM_InitializeAllAsmPrinters
pub fn llvm_initialize_all_asm_printers() {
    unsafe {
        LLVM_InitializeAllAsmPrinters();
    }
}

/// LLVM_InitializeAllAsmParsers
pub fn llvm_initialize_all_asm_parsers() {
    unsafe {
        LLVM_InitializeAllAsmParsers();
    }
}

/// LLVM_InitializeAllDisassemblers
pub fn llvm_initialize_all_disassemblers() {
    unsafe {
        LLVM_InitializeAllDisassemblers();
    }
}

/// LLVM_InitializeNativeTarget
pub fn llvm_initialize_native_target() -> Result<(), String> {
    if !unsafe { LLVM_InitializeNativeTarget().to_bool() } {
        Ok(())
    } else {
        Err("Failed to initialize native target".to_string())
    }
}

/// LLVM_InitializeNativeAsmParser
pub fn llvm_initialize_native_asm_parser() -> Result<(), String> {
    if !unsafe { LLVM_InitializeNativeAsmParser().to_bool() } {
        Ok(())
    } else {
        Err("Failed to initialize native asm parser".to_string())
    }
}

/// LLVM_InitializeNativeAsmParser
pub fn llvm_initialize_native_asm_printer() -> Result<(), String> {
    if !unsafe { LLVM_InitializeNativeAsmPrinter().to_bool() } {
        Ok(())
    } else {
        Err("Failed to initialize native asm printer".to_string())
    }
}

/// LLVM_InitializeNativeDisassembler
pub fn llvm_initialize_native_disassembler() -> Result<(), String> {
    if !unsafe { LLVM_InitializeNativeDisassembler().to_bool() } {
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
