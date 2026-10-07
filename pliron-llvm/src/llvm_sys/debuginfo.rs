// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around `llvm_sys::debuginfo`, and the debug location
//! functions of `llvm_sys::core`.

use std::{ffi::c_char, ptr};

use llvm_sys::{
    LLVMModuleFlagBehavior,
    core::{LLVMAddModuleFlag, LLVMGetModuleFlag, LLVMSetCurrentDebugLocation2},
    debuginfo::{
        LLVMChecksumKind, LLVMDIBuilderCreateCompileUnit, LLVMDIBuilderCreateDebugLocation,
        LLVMDIBuilderCreateFile, LLVMDIBuilderCreateFileWithChecksum, LLVMDIBuilderCreateFunction,
        LLVMDIBuilderCreateLexicalBlockFile, LLVMDIBuilderCreateSubroutineType, LLVMDIFlagZero,
        LLVMDIScopeGetFile, LLVMDWARFEmissionKind, LLVMDWARFSourceLanguage,
        LLVMDebugMetadataVersion, LLVMMetadataKind, LLVMSetSubprogram,
    },
    prelude::LLVMMetadataRef,
};

use crate::llvm_sys::core::{
    LLVMBuilder, LLVMContext, LLVMMetadata, LLVMModule, LLVMValue, llvm_get_metadata_kind,
    llvm_is_a,
};

// We wrap LLVMDIBuilder in a module to limit its visibility for constructing
mod llvm_di_builder {
    use llvm_sys::{
        debuginfo::{LLVMCreateDIBuilder, LLVMDIBuilderFinalize, LLVMDisposeDIBuilder},
        prelude::LLVMDIBuilderRef,
    };

    use super::LLVMModule;

    /// RAII wrapper around `LLVMDIBuilderRef`.
    /// Dropping it finalizes the debug data of its module.
    pub struct LLVMDIBuilder(LLVMDIBuilderRef);

    impl LLVMDIBuilder {
        /// `LLVMCreateDIBuilder`
        ///
        /// The builder must be dropped before `module`.
        pub fn new(module: &LLVMModule) -> Self {
            unsafe { LLVMDIBuilder(LLVMCreateDIBuilder(module.inner_ref())) }
        }

        /// Get the inner `LLVMDIBuilderRef`
        pub(in crate::llvm_sys) fn inner_ref(&self) -> LLVMDIBuilderRef {
            self.0
        }
    }

    impl Drop for LLVMDIBuilder {
        fn drop(&mut self) {
            unsafe {
                LLVMDIBuilderFinalize(self.0);
                LLVMDisposeDIBuilder(self.0);
            }
        }
    }
}
pub use llvm_di_builder::LLVMDIBuilder;

fn is_md_kind(md: LLVMMetadata, kind: LLVMMetadataKind) -> bool {
    // `LLVMMetadataKind` has no `PartialEq`. Thus we compare the discriminants.
    llvm_get_metadata_kind(md) as u32 == kind as u32
}

/// Is `md` a scope that a `DILocation` can refer to?
fn is_local_scope(md: LLVMMetadata) -> bool {
    matches!(
        llvm_get_metadata_kind(md),
        LLVMMetadataKind::LLVMDISubprogramMetadataKind
            | LLVMMetadataKind::LLVMDILexicalBlockMetadataKind
            | LLVMMetadataKind::LLVMDILexicalBlockFileMetadataKind
    )
}

/// `LLVMDebugMetadataVersion`
pub fn llvm_debug_metadata_version() -> u32 {
    unsafe { LLVMDebugMetadataVersion() }
}

/// `LLVMDIBuilderCreateFile`
pub fn llvm_di_builder_create_file(
    builder: &LLVMDIBuilder,
    filename: &str,
    directory: &str,
) -> LLVMMetadata {
    unsafe {
        LLVMDIBuilderCreateFile(
            builder.inner_ref(),
            filename.as_ptr().cast::<c_char>(),
            filename.len(),
            directory.as_ptr().cast::<c_char>(),
            directory.len(),
        )
        .into()
    }
}

/// `LLVMDIBuilderCreateFileWithChecksum`
///
/// `checksum` is in hexadecimal. An empty `source` means none.
///
/// # Panics
///
/// If `checksum` is not a hexadecimal value of the length that `checksum_kind` requires.
pub fn llvm_di_builder_create_file_with_checksum(
    builder: &LLVMDIBuilder,
    filename: &str,
    directory: &str,
    checksum_kind: LLVMChecksumKind,
    checksum: &str,
    source: &str,
) -> LLVMMetadata {
    let len = match checksum_kind {
        LLVMChecksumKind::CSK_MD5 => 32,
        LLVMChecksumKind::CSK_SHA1 => 40,
        LLVMChecksumKind::CSK_SHA256 => 64,
    };
    assert!(checksum.len() == len && checksum.bytes().all(|b| b.is_ascii_hexdigit()));
    unsafe {
        LLVMDIBuilderCreateFileWithChecksum(
            builder.inner_ref(),
            filename.as_ptr().cast::<c_char>(),
            filename.len(),
            directory.as_ptr().cast::<c_char>(),
            directory.len(),
            checksum_kind,
            checksum.as_ptr().cast::<c_char>(),
            checksum.len(),
            source.as_ptr().cast::<c_char>(),
            source.len(),
        )
        .into()
    }
}

/// `LLVMDIBuilderCreateCompileUnit`
///
/// The unit has no flags, no split DWARF, no SDK and no system root.
///
/// # Panics
///
/// If `file` is not a `DIFile`.
pub fn llvm_di_builder_create_compile_unit(
    builder: &LLVMDIBuilder,
    language: LLVMDWARFSourceLanguage,
    file: LLVMMetadata,
    producer: &str,
    is_optimized: bool,
    kind: LLVMDWARFEmissionKind,
) -> LLVMMetadata {
    assert!(is_md_kind(file, LLVMMetadataKind::LLVMDIFileMetadataKind));
    unsafe {
        LLVMDIBuilderCreateCompileUnit(
            builder.inner_ref(),
            language,
            file.into(),
            producer.as_ptr().cast::<c_char>(),
            producer.len(),
            is_optimized.into(),
            ptr::null(),
            0,
            0,
            ptr::null(),
            0,
            kind,
            0,
            0,
            0,
            ptr::null(),
            0,
            ptr::null(),
            0,
        )
        .into()
    }
}

/// `LLVMDIBuilderCreateSubroutineType`
///
/// # Panics
///
/// If `file` is not a `DIFile`, or if there are more than `u32::MAX` parameter types.
pub fn llvm_di_builder_create_subroutine_type(
    builder: &LLVMDIBuilder,
    file: LLVMMetadata,
    parameter_types: &[LLVMMetadata],
) -> LLVMMetadata {
    assert!(is_md_kind(file, LLVMMetadataKind::LLVMDIFileMetadataKind));
    let mut parameter_types: Vec<LLVMMetadataRef> =
        parameter_types.iter().map(|ty| (*ty).into()).collect();
    unsafe {
        LLVMDIBuilderCreateSubroutineType(
            builder.inner_ref(),
            file.into(),
            parameter_types.as_mut_ptr(),
            u32::try_from(parameter_types.len()).expect("too many parameter types"),
            LLVMDIFlagZero,
        )
        .into()
    }
}

/// `LLVMDIBuilderCreateFunction`
///
/// The subprogram has no flags. An empty `linkage_name` means none.
///
/// # Panics
///
/// If `file` is not a `DIFile`, or `ty` is not a `DISubroutineType`.
#[allow(clippy::too_many_arguments)]
pub fn llvm_di_builder_create_function(
    builder: &LLVMDIBuilder,
    scope: LLVMMetadata,
    name: &str,
    linkage_name: &str,
    file: LLVMMetadata,
    line: u32,
    ty: LLVMMetadata,
    is_local_to_unit: bool,
    is_definition: bool,
    scope_line: u32,
    is_optimized: bool,
) -> LLVMMetadata {
    assert!(is_md_kind(file, LLVMMetadataKind::LLVMDIFileMetadataKind));
    assert!(is_md_kind(
        ty,
        LLVMMetadataKind::LLVMDISubroutineTypeMetadataKind
    ));
    unsafe {
        LLVMDIBuilderCreateFunction(
            builder.inner_ref(),
            scope.into(),
            name.as_ptr().cast::<c_char>(),
            name.len(),
            linkage_name.as_ptr().cast::<c_char>(),
            linkage_name.len(),
            file.into(),
            line,
            ty.into(),
            is_local_to_unit.into(),
            is_definition.into(),
            scope_line,
            LLVMDIFlagZero,
            is_optimized.into(),
        )
        .into()
    }
}

/// `LLVMDIBuilderCreateLexicalBlockFile`
///
/// # Panics
///
/// If `scope` is not a local scope, or `file` is not a `DIFile`.
pub fn llvm_di_builder_create_lexical_block_file(
    builder: &LLVMDIBuilder,
    scope: LLVMMetadata,
    file: LLVMMetadata,
    discriminator: u32,
) -> LLVMMetadata {
    assert!(is_local_scope(scope));
    assert!(is_md_kind(file, LLVMMetadataKind::LLVMDIFileMetadataKind));
    unsafe {
        LLVMDIBuilderCreateLexicalBlockFile(
            builder.inner_ref(),
            scope.into(),
            file.into(),
            discriminator,
        )
        .into()
    }
}

/// `LLVMDIBuilderCreateDebugLocation`
///
/// # Panics
///
/// If `scope` is not a local scope, or `inlined_at` is not a `DILocation`.
pub fn llvm_di_builder_create_debug_location(
    ctx: &LLVMContext,
    line: u32,
    column: u32,
    scope: LLVMMetadata,
    inlined_at: Option<LLVMMetadata>,
) -> LLVMMetadata {
    assert!(is_local_scope(scope));
    assert!(
        inlined_at.is_none_or(|loc| is_md_kind(loc, LLVMMetadataKind::LLVMDILocationMetadataKind))
    );
    unsafe {
        LLVMDIBuilderCreateDebugLocation(
            ctx.inner_ref(),
            line,
            column,
            scope.into(),
            inlined_at.map_or(ptr::null_mut(), Into::into),
        )
        .into()
    }
}

/// `LLVMDIScopeGetFile`
pub fn llvm_di_scope_get_file(scope: LLVMMetadata) -> Option<LLVMMetadata> {
    let file = unsafe { LLVMDIScopeGetFile(scope.into()) };
    (!file.is_null()).then(|| file.into())
}

/// `LLVMSetSubprogram`
///
/// # Panics
///
/// If `func` is not a function, or `subprogram` is not a `DISubprogram`.
pub fn llvm_set_subprogram(func: LLVMValue, subprogram: LLVMMetadata) {
    assert!(llvm_is_a::function(func));
    assert!(is_md_kind(
        subprogram,
        LLVMMetadataKind::LLVMDISubprogramMetadataKind
    ));
    unsafe { LLVMSetSubprogram(func.into(), subprogram.into()) }
}

/// `LLVMSetCurrentDebugLocation2`
///
/// `None` clears the location.
///
/// # Panics
///
/// If `loc` is not a `DILocation`.
pub fn llvm_set_current_debug_location2(builder: &LLVMBuilder, loc: Option<LLVMMetadata>) {
    assert!(loc.is_none_or(|loc| is_md_kind(loc, LLVMMetadataKind::LLVMDILocationMetadataKind)));
    unsafe {
        LLVMSetCurrentDebugLocation2(builder.inner_ref(), loc.map_or(ptr::null_mut(), Into::into));
    }
}

/// `LLVMGetModuleFlag`
pub fn llvm_get_module_flag(module: &LLVMModule, key: &str) -> Option<LLVMMetadata> {
    let flag =
        unsafe { LLVMGetModuleFlag(module.inner_ref(), key.as_ptr().cast::<c_char>(), key.len()) };
    (!flag.is_null()).then(|| flag.into())
}

/// `LLVMAddModuleFlag`
pub fn llvm_add_module_flag(
    module: &LLVMModule,
    behavior: LLVMModuleFlagBehavior,
    key: &str,
    value: LLVMMetadata,
) {
    unsafe {
        LLVMAddModuleFlag(
            module.inner_ref(),
            behavior,
            key.as_ptr().cast::<c_char>(),
            key.len(),
            value.into(),
        );
    }
}
