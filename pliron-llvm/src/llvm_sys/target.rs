// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around llvm_sys::target

use crate::llvm_sys::{
    ToBool,
    core::{LLVMType, llvm_count_struct_element_types, llvm_get_type_kind, llvm_type_is_sized},
    cstr_to_string, to_c_str,
};
use llvm_sys::{
    LLVMTypeKind,
    core::LLVMDisposeMessage,
    target::{
        LLVM_InitializeAllAsmParsers, LLVM_InitializeAllAsmPrinters,
        LLVM_InitializeAllDisassemblers, LLVM_InitializeAllTargetInfos,
        LLVM_InitializeAllTargetMCs, LLVM_InitializeAllTargets, LLVM_InitializeNativeAsmParser,
        LLVM_InitializeNativeAsmPrinter, LLVM_InitializeNativeDisassembler,
        LLVM_InitializeNativeTarget, LLVMABIAlignmentOfType, LLVMABISizeOfType,
        LLVMCopyStringRepOfTargetData, LLVMCreateTargetData, LLVMDisposeTargetData,
        LLVMOffsetOfElement, LLVMPointerSizeForAS, LLVMPreferredAlignmentOfType,
        LLVMSizeOfTypeInBits, LLVMStoreSizeOfType, LLVMTargetDataRef,
    },
    target_machine::{
        LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetDataLayout, LLVMCreateTargetMachine,
        LLVMDisposeTargetMachine, LLVMGetDefaultTargetTriple, LLVMGetTargetFromTriple,
        LLVMRelocMode,
    },
};
use std::{
    mem::MaybeUninit,
    sync::{Mutex, PoisonError},
};

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

// We wrap LLVMTargetData in a module to limit its visibility for constructing
mod llvm_target_data {
    use super::*;

    /// RAII wrapper around LLVMTargetDataReff
    pub struct LLVMTargetData(LLVMTargetDataRef);

    impl Drop for LLVMTargetData {
        fn drop(&mut self) {
            unsafe { LLVMDisposeTargetData(self.0) }
        }
    }

    impl LLVMTargetData {
        /// LLVMCreateTargetData
        ///
        /// **Note**: `layout` is not validated (the C-API gives no way to do that).
        pub fn new(layout: &str) -> Self {
            LLVMTargetData(unsafe { LLVMCreateTargetData(to_c_str(layout).as_ptr()) })
        }

        /// Build the layout of the machine that this program runs on.
        pub fn host() -> Result<Self, String> {
            // The layout comes from a target machine
            llvm_initialize_native_target()?;

            let triple_ptr = unsafe { LLVMGetDefaultTargetTriple() };
            let triple = cstr_to_string(triple_ptr).unwrap_or_default();
            unsafe { LLVMDisposeMessage(triple_ptr) };

            let mut target = MaybeUninit::uninit();
            let mut err_string = MaybeUninit::uninit();
            let failed = unsafe {
                LLVMGetTargetFromTriple(
                    to_c_str(&triple).as_ptr(),
                    target.as_mut_ptr(),
                    err_string.as_mut_ptr(),
                )
                .to_bool()
            };
            if failed {
                unsafe {
                    let err_ptr = err_string.assume_init();
                    let err = cstr_to_string(err_ptr).unwrap_or_default();
                    LLVMDisposeMessage(err_ptr);
                    return Err(err);
                }
            }

            let machine = unsafe {
                LLVMCreateTargetMachine(
                    target.assume_init(),
                    to_c_str(&triple).as_ptr(),
                    // CPU and its features do not affect the layout
                    to_c_str("").as_ptr(),
                    to_c_str("").as_ptr(),
                    LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
                    LLVMRelocMode::LLVMRelocDefault,
                    LLVMCodeModel::LLVMCodeModelDefault,
                )
            };
            if machine.is_null() {
                return Err(format!("Failed to create a target machine for {triple}"));
            }

            let target_data = unsafe { LLVMCreateTargetDataLayout(machine) };
            unsafe { LLVMDisposeTargetMachine(machine) };
            Ok(LLVMTargetData(target_data))
        }

        /// LLVMCopyStringRepOfTargetData
        pub fn copy_string_rep_of_target_data(&self) -> String {
            let buf_ptr = unsafe { LLVMCopyStringRepOfTargetData(self.0) };
            let layout = cstr_to_string(buf_ptr).unwrap_or_default();
            unsafe { LLVMDisposeMessage(buf_ptr) };
            layout
        }

        /// LLVMSizeOfTypeInBits
        pub fn size_of_type_in_bits(&self, ty: LLVMType) -> u64 {
            assert!(llvm_type_is_sized(ty));
            unsafe { LLVMSizeOfTypeInBits(self.0, ty.into()) }
        }

        /// LLVMStoreSizeOfType
        pub fn store_size_of_type(&self, ty: LLVMType) -> u64 {
            assert!(llvm_type_is_sized(ty));
            unsafe { LLVMStoreSizeOfType(self.0, ty.into()) }
        }

        /// LLVMABISizeOfType: Misnomer. It actually is `llvm::DataLayout::getTypeAllocSize.
        pub fn abi_size_of_type(&self, ty: LLVMType) -> u64 {
            assert!(llvm_type_is_sized(ty));
            unsafe { LLVMABISizeOfType(self.0, ty.into()) }
        }

        /// LLVMABIAlignmentOfType
        pub fn abi_alignment_of_type(&self, ty: LLVMType) -> u32 {
            assert!(llvm_type_is_sized(ty));
            unsafe { LLVMABIAlignmentOfType(self.0, ty.into()) }
        }

        /// LLVMPreferredAlignmentOfType
        pub fn preferred_alignment_of_type(&self, ty: LLVMType) -> u32 {
            assert!(llvm_type_is_sized(ty));
            unsafe { LLVMPreferredAlignmentOfType(self.0, ty.into()) }
        }

        /// LLVMOffsetOfElement
        pub fn offset_of_element(&self, struct_ty: LLVMType, index: u32) -> u64 {
            assert!(llvm_get_type_kind(struct_ty) == LLVMTypeKind::LLVMStructTypeKind);
            assert!(llvm_type_is_sized(struct_ty));
            assert!(index < llvm_count_struct_element_types(struct_ty));
            unsafe { LLVMOffsetOfElement(self.0, struct_ty.into(), index) }
        }

        /// LLVMPointerSizeForAS
        pub fn pointer_size_for_as(&self, addr_space: u32) -> u32 {
            unsafe { LLVMPointerSizeForAS(self.0, addr_space) }
        }
    }
}
pub use llvm_target_data::LLVMTargetData;
