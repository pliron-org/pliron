// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Safe(r) wrappers around llvm_sys::debuginfo.

use std::ptr;

use llvm_sys::{
    debuginfo::{
        LLVMChecksumKind, LLVMDIBuilderCreateCompileUnit, LLVMDIBuilderCreateDebugLocation,
        LLVMDIBuilderCreateFile, LLVMDIBuilderCreateFileWithChecksum, LLVMDIBuilderCreateFunction,
        LLVMDIBuilderCreateLexicalBlockFile, LLVMDIBuilderCreateSubroutineType, LLVMDIFlags,
        LLVMDIScopeGetFile, LLVMDWARFEmissionKind, LLVMDWARFSourceLanguage,
        LLVMDebugMetadataVersion, LLVMMetadataKind, LLVMSetSubprogram,
    },
    prelude::LLVMMetadataRef,
};

use crate::{
    llvm_sys::core::{LLVMContext, LLVMMetadata, LLVMValue, llvm_get_metadata_kind, llvm_is_a},
    metadata_conversions::to_llvm_ir::{DwarfEmissionKind, DwarfSourceLanguage},
};

// We wrap LLVMDIBuilder in a module to limit its visibility for constructing
mod llvm_di_builder {
    use core::marker::PhantomData;

    use llvm_sys::{
        debuginfo::{LLVMCreateDIBuilder, LLVMDIBuilderFinalize, LLVMDisposeDIBuilder},
        prelude::LLVMDIBuilderRef,
    };

    use crate::llvm_sys::core::LLVMModule;

    /// Managed LLVMDIBuilder. Its module's metadata is finalized on [Drop::drop].
    pub struct LLVMDIBuilder<'a>(LLVMDIBuilderRef, PhantomData<&'a LLVMModule>);

    impl<'a> LLVMDIBuilder<'a> {
        /// Create a new LLVMDIBuilder for the given module.
        pub fn new(module: &'a LLVMModule) -> Self {
            unsafe { LLVMDIBuilder(LLVMCreateDIBuilder(module.inner_ref()), PhantomData) }
        }

        /// Get the inner LLVMDIBuilderRef
        pub(in crate::llvm_sys) fn inner_ref(&self) -> LLVMDIBuilderRef {
            self.0
        }
    }

    impl Drop for LLVMDIBuilder<'_> {
        fn drop(&mut self) {
            unsafe {
                LLVMDIBuilderFinalize(self.0);
                LLVMDisposeDIBuilder(self.0);
            }
        }
    }
}
pub use llvm_di_builder::LLVMDIBuilder;

impl From<LLVMDWARFEmissionKind> for DwarfEmissionKind {
    fn from(kind: LLVMDWARFEmissionKind) -> Self {
        match kind {
            LLVMDWARFEmissionKind::LLVMDWARFEmissionKindNone => DwarfEmissionKind::None,
            LLVMDWARFEmissionKind::LLVMDWARFEmissionKindFull => DwarfEmissionKind::Full,
            LLVMDWARFEmissionKind::LLVMDWARFEmissionKindLineTablesOnly => {
                DwarfEmissionKind::LineTablesOnly
            }
        }
    }
}

impl From<DwarfEmissionKind> for LLVMDWARFEmissionKind {
    fn from(kind: DwarfEmissionKind) -> Self {
        match kind {
            DwarfEmissionKind::None => LLVMDWARFEmissionKind::LLVMDWARFEmissionKindNone,
            DwarfEmissionKind::Full => LLVMDWARFEmissionKind::LLVMDWARFEmissionKindFull,
            DwarfEmissionKind::LineTablesOnly => {
                LLVMDWARFEmissionKind::LLVMDWARFEmissionKindLineTablesOnly
            }
        }
    }
}

impl From<LLVMDWARFSourceLanguage> for DwarfSourceLanguage {
    fn from(language: LLVMDWARFSourceLanguage) -> Self {
        match language {
            LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC => DwarfSourceLanguage::C,
            LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC_plus_plus => {
                DwarfSourceLanguage::CPlusPlus
            }
            LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageRust => DwarfSourceLanguage::Rust,
            language => todo!("DWARF source language {language:?}"),
        }
    }
}

impl From<DwarfSourceLanguage> for LLVMDWARFSourceLanguage {
    fn from(language: DwarfSourceLanguage) -> Self {
        match language {
            DwarfSourceLanguage::C => LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC,
            DwarfSourceLanguage::CPlusPlus => {
                LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC_plus_plus
            }
            DwarfSourceLanguage::Rust => LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageRust,
        }
    }
}

/// Is `md` of `kind`?
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

/// LLVMDebugMetadataVersion
pub fn llvm_debug_metadata_version() -> u32 {
    unsafe { LLVMDebugMetadataVersion() }
}

/// LLVMDIBuilderCreateFile
pub fn llvm_di_builder_create_file(
    builder: &LLVMDIBuilder,
    filename: &str,
    directory: &str,
) -> LLVMMetadata {
    unsafe {
        LLVMDIBuilderCreateFile(
            builder.inner_ref(),
            filename.as_ptr() as *const core::ffi::c_char,
            filename.len(),
            directory.as_ptr() as *const core::ffi::c_char,
            directory.len(),
        )
        .into()
    }
}

/// LLVMDIBuilderCreateFileWithChecksum
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
            filename.as_ptr() as *const core::ffi::c_char,
            filename.len(),
            directory.as_ptr() as *const core::ffi::c_char,
            directory.len(),
            checksum_kind,
            checksum.as_ptr() as *const core::ffi::c_char,
            checksum.len(),
            source.as_ptr() as *const core::ffi::c_char,
            source.len(),
        )
        .into()
    }
}

/// LLVMDIBuilderCreateCompileUnit
#[allow(clippy::too_many_arguments)]
pub fn llvm_di_builder_create_compile_unit(
    builder: &LLVMDIBuilder,
    language: LLVMDWARFSourceLanguage,
    file: LLVMMetadata,
    producer: &str,
    is_optimized: bool,
    flags: &str,
    runtime_version: u32,
    split_name: &str,
    kind: LLVMDWARFEmissionKind,
    dwo_id: u32,
    split_debug_inlining: bool,
    debug_info_for_profiling: bool,
    sys_root: &str,
    sdk: &str,
) -> LLVMMetadata {
    assert!(is_md_kind(file, LLVMMetadataKind::LLVMDIFileMetadataKind));
    unsafe {
        LLVMDIBuilderCreateCompileUnit(
            builder.inner_ref(),
            language,
            file.into(),
            producer.as_ptr() as *const core::ffi::c_char,
            producer.len(),
            is_optimized.into(),
            flags.as_ptr() as *const core::ffi::c_char,
            flags.len(),
            runtime_version,
            split_name.as_ptr() as *const core::ffi::c_char,
            split_name.len(),
            kind,
            dwo_id,
            split_debug_inlining.into(),
            debug_info_for_profiling.into(),
            sys_root.as_ptr() as *const core::ffi::c_char,
            sys_root.len(),
            sdk.as_ptr() as *const core::ffi::c_char,
            sdk.len(),
        )
        .into()
    }
}

/// LLVMDIBuilderCreateSubroutineType
pub fn llvm_di_builder_create_subroutine_type(
    builder: &LLVMDIBuilder,
    file: LLVMMetadata,
    parameter_types: &[LLVMMetadata],
    flags: LLVMDIFlags,
) -> LLVMMetadata {
    assert!(is_md_kind(file, LLVMMetadataKind::LLVMDIFileMetadataKind));
    let mut parameter_types: Vec<LLVMMetadataRef> =
        parameter_types.iter().map(|ty| (*ty).into()).collect();
    unsafe {
        LLVMDIBuilderCreateSubroutineType(
            builder.inner_ref(),
            file.into(),
            parameter_types.as_mut_ptr(),
            parameter_types.len().try_into().unwrap(),
            flags,
        )
        .into()
    }
}

/// LLVMDIBuilderCreateFunction
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
    flags: LLVMDIFlags,
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
            name.as_ptr() as *const core::ffi::c_char,
            name.len(),
            linkage_name.as_ptr() as *const core::ffi::c_char,
            linkage_name.len(),
            file.into(),
            line,
            ty.into(),
            is_local_to_unit.into(),
            is_definition.into(),
            scope_line,
            flags,
            is_optimized.into(),
        )
        .into()
    }
}

/// LLVMDIBuilderCreateLexicalBlockFile
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

/// LLVMDIBuilderCreateDebugLocation
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

/// LLVMDIScopeGetFile
pub fn llvm_di_scope_get_file(scope: LLVMMetadata) -> Option<LLVMMetadata> {
    let file = unsafe { LLVMDIScopeGetFile(scope.into()) };
    (!file.is_null()).then(|| file.into())
}

/// LLVMSetSubprogram
pub fn llvm_set_subprogram(func: LLVMValue, subprogram: LLVMMetadata) {
    assert!(llvm_is_a::function(func));
    assert!(is_md_kind(
        subprogram,
        LLVMMetadataKind::LLVMDISubprogramMetadataKind
    ));
    unsafe { LLVMSetSubprogram(func.into(), subprogram.into()) }
}
