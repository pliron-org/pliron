// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around llvm_sys::lljit
//!
//! ### Example
//!```
//! use pliron_llvm::llvm_sys::target::initialize_native;
//! use pliron_llvm::llvm_sys::core::{LLVMContext, LLVMModule, LLVMMemoryBuffer};
//! use pliron_llvm::llvm_sys::lljit::{LLVMLLJIT, JITSymbolGenericFlags};
//! fn main() -> Result<(), String> {
//!    initialize_native()?;
//!    let context = LLVMContext::default();
//!
//!    fn my_rust_adder(a: i32, b: i32) -> i32 {
//!        a + b
//!    }
//!
//!    let ir = r#"
//!      declare i32 @my_rust_adder(i32, i32)
//!      define i32 @add(i32 %a, i32 %b) {
//!          %sum = call i32 @my_rust_adder(i32 %a, i32 %b)
//!          ret i32 %sum
//!      }"#;
//!    let ir_mb = LLVMMemoryBuffer::from_str(ir, "test_buffer");
//!    let module = LLVMModule::from_ir_in_memory_buffer(&context, ir_mb)?;
//!
//!    let jit = LLVMLLJIT::new_with_default_builder()?;
//!    jit.add_module(module)?;
//!    // Add the Rust function as a symbol mapping
//!    let rust_adder_addr = my_rust_adder as *const () as u64;
//!    jit.add_symbol_mapping
//!     ("my_rust_adder", rust_adder_addr,
//!       JITSymbolGenericFlags::JITSymbolGenericFlagsCallable
//!         | JITSymbolGenericFlags::JITSymbolGenericFlagsExported)?;
//!
//!    // Get symbol address for 'add' in the LLVM module
//!    let symbol_addr = jit.lookup_symbol("add")?;
//!    assert!(symbol_addr != 0);
//!
//!    let adder = unsafe { std::mem::transmute::<u64, fn(i32, i32) -> i32>(symbol_addr) };
//!    assert_eq!(adder(2, 3), 5);
//!    Ok(())
//! }
//! ```

use bitflags::bitflags;
use std::{mem::MaybeUninit, ptr};

use llvm_sys::orc2::{
    LLVMJITEvaluatedSymbol, LLVMJITSymbolFlags, LLVMJITSymbolGenericFlags, LLVMOrcAbsoluteSymbols,
    LLVMOrcCSymbolMapPair, LLVMOrcCreateNewThreadSafeContext, LLVMOrcCreateNewThreadSafeModule,
    LLVMOrcDisposeThreadSafeContext, LLVMOrcDisposeThreadSafeModule, lljit,
};

use crate::llvm_sys::{
    core::{LLVMModule, handle_err},
    cstr_to_string, to_c_str,
};

bitflags! {
     #[derive(PartialEq, Eq, Clone, Debug, Hash, Copy)]
    pub struct JITSymbolGenericFlags: u8 {
        const JITSymbolGenericFlagsNone = 0;
        const JITSymbolGenericFlagsExported = 1;
        const JITSymbolGenericFlagsWeak = 2;
        const JITSymbolGenericFlagsCallable = 4;
        const JITSymbolGenericFlagsMaterializationSideEffectsOnly = 8;
    }
}

impl From<LLVMJITSymbolGenericFlags> for JITSymbolGenericFlags {
    fn from(value: LLVMJITSymbolGenericFlags) -> Self {
        let mut flags = JITSymbolGenericFlags::empty();
        if (value as u8) & (LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsExported as u8) != 0
        {
            flags |= JITSymbolGenericFlags::JITSymbolGenericFlagsExported;
        }
        if (value as u8) & (LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsWeak as u8) != 0 {
            flags |= JITSymbolGenericFlags::JITSymbolGenericFlagsWeak;
        }
        if (value as u8) & (LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsCallable as u8) != 0
        {
            flags |= JITSymbolGenericFlags::JITSymbolGenericFlagsCallable;
        }
        if (value as u8)
            & (LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsMaterializationSideEffectsOnly
                as u8)
            != 0
        {
            flags |= JITSymbolGenericFlags::JITSymbolGenericFlagsMaterializationSideEffectsOnly;
        }
        flags
    }
}

impl From<JITSymbolGenericFlags> for u8 {
    fn from(value: JITSymbolGenericFlags) -> Self {
        let mut flags = LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsNone as u8;
        if value.contains(JITSymbolGenericFlags::JITSymbolGenericFlagsExported) {
            flags |= LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsExported as u8;
        }
        if value.contains(JITSymbolGenericFlags::JITSymbolGenericFlagsWeak) {
            flags |= LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsWeak as u8;
        }
        if value.contains(JITSymbolGenericFlags::JITSymbolGenericFlagsCallable) {
            flags |= LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsCallable as u8;
        }
        if value
            .contains(JITSymbolGenericFlags::JITSymbolGenericFlagsMaterializationSideEffectsOnly)
        {
            flags |=
                LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsMaterializationSideEffectsOnly
                    as u8;
        }
        flags
    }
}

pub struct LLVMLLJIT(lljit::LLVMOrcLLJITRef);

impl LLVMLLJIT {
    /// Create a new LLJIT instance with default settings.
    pub fn new_with_default_builder() -> Result<Self, String> {
        unsafe {
            let mut jit = MaybeUninit::uninit();
            let err = lljit::LLVMOrcCreateLLJIT(jit.as_mut_ptr(), ptr::null_mut());
            handle_err(err)?;
            Ok(LLVMLLJIT(jit.assume_init()))
        }
    }

    /// Add an [LLVMModule] to the JIT's main JITDylib, in its own thread-safe context.
    pub fn add_module(&self, module: LLVMModule) -> Result<(), String> {
        unsafe {
            let tsctx = LLVMOrcCreateNewThreadSafeContext();
            let tsm = LLVMOrcCreateNewThreadSafeModule(module.inner_ref(), tsctx);
            let main_jd = lljit::LLVMOrcLLJITGetMainJITDylib(self.0);
            let err = lljit::LLVMOrcLLJITAddLLVMIRModule(self.0, main_jd, tsm);
            // The underlying LLVMContext will be kept alive by our ThreadSafeModule
            // (See OrcV2CBindingsBasicUsage.c)
            LLVMOrcDisposeThreadSafeContext(tsctx);
            // Ownership of the module has been transferred to the JIT
            std::mem::forget(module);
            handle_err(err).inspect_err(|_| {
                // Dispose of the ThreadSafeModule on error
                LLVMOrcDisposeThreadSafeModule(tsm);
            })
        }
    }

    /// Lookup a symbol in the JIT.
    pub fn lookup_symbol(&self, name: &str) -> Result<u64, String> {
        unsafe {
            let mut addr = MaybeUninit::uninit();
            let err = lljit::LLVMOrcLLJITLookup(self.0, addr.as_mut_ptr(), to_c_str(name).as_ptr());
            handle_err(err)?;
            Ok(addr.assume_init())
        }
    }

    /// Get the target triple string for this JIT instance.
    pub fn get_triple_string(&self) -> String {
        unsafe {
            let triple_ptr = lljit::LLVMOrcLLJITGetTripleString(self.0);
            cstr_to_string(triple_ptr).unwrap()
        }
    }

    /// Add a symbol mapping to the JIT's main DyLib
    pub fn add_symbol_mapping(
        &self,
        name: &str,
        addr: u64,
        flags: JITSymbolGenericFlags,
    ) -> Result<(), String> {
        let symbol_pool_ref =
            unsafe { lljit::LLVMOrcLLJITMangleAndIntern(self.0, to_c_str(name).as_ptr()) };

        let jit_evaluated_symbol = LLVMJITEvaluatedSymbol {
            Address: addr,
            Flags: LLVMJITSymbolFlags {
                GenericFlags: flags.into(),
                TargetFlags: 0,
            },
        };

        let mut symbol_pair = LLVMOrcCSymbolMapPair {
            Name: symbol_pool_ref,
            Sym: jit_evaluated_symbol,
        };

        let materialization_unit = unsafe { LLVMOrcAbsoluteSymbols(&mut symbol_pair as *mut _, 1) };
        let main_dylib = unsafe { lljit::LLVMOrcLLJITGetMainJITDylib(self.0) };

        let res =
            unsafe { llvm_sys::orc2::LLVMOrcJITDylibDefine(main_dylib, materialization_unit) };
        handle_err(res)
    }
}

impl Drop for LLVMLLJIT {
    fn drop(&mut self) {
        unsafe {
            let err = lljit::LLVMOrcDisposeLLJIT(self.0);
            if let Err(err) = handle_err(err) {
                panic!("Error disposing LLJIT: {}", err);
            }
        }
    }
}
