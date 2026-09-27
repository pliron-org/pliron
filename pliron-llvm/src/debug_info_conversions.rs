// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Conversion of op locations to LLVM debug data.

/// Conversion of op locations to LLVM debug data, a companion to [crate::to_llvm_ir].
///
/// Each function that has a location gets a `DISubprogram`.
/// Each instruction in such a function gets a `DILocation`:
/// - [Location::SrcPos] gives the line and the column.
/// - [Location::Named] gives the location of its child. The name of the
///   outermost frame of a location is the name of a function.
/// - [Location::CallSite] gives the location of its callee, inlined at the
///   location of its caller. The name of the outermost frame of the callee
///   gives a `DISubprogram` for the callee. If the callee has no name, the
///   conversion uses the location of the caller.
/// - [Location::Fused] gives its first location that converts. The C-API
///   cannot merge locations.
/// - [Location::Unknown] gives line 0. Line 0 is code with no source line.
///
/// A position in a file that is not the file of its scope gets a
/// `DILexicalBlockFile`.
///
/// [Location::SrcPos]: pliron::location::Location::SrcPos
/// [Location::Named]: pliron::location::Location::Named
/// [Location::CallSite]: pliron::location::Location::CallSite
/// [Location::Fused]: pliron::location::Location::Fused
/// [Location::Unknown]: pliron::location::Location::Unknown
pub mod to_llvm_ir {
    use std::string::{String, ToString};

    use llvm_sys::{LLVMModuleFlagBehavior, debuginfo::LLVMDWARFEmissionKind};
    use pliron::{
        builtin::op_interfaces::SymbolOpInterface,
        context::Context,
        graph::walkers::{
            IRNode, WALKCONFIG_PREORDER_FORWARD,
            interruptible::{WalkResult, immutable::walk_op, walk_advance, walk_break},
        },
        location::{Located, Location, Source},
        op::Op,
        uniqued_any,
        utils::table::HMap,
    };

    pub use llvm_sys::debuginfo::LLVMDWARFSourceLanguage;

    use crate::{
        llvm_sys::{
            core::{
                LLVMContext, LLVMMetadata, LLVMModule, LLVMValue, llvm_const_int,
                llvm_int_type_in_context, llvm_value_as_metadata,
            },
            debuginfo::{
                LLVMDIBuilder, llvm_add_module_flag, llvm_debug_metadata_version,
                llvm_di_builder_create_compile_unit, llvm_di_builder_create_debug_location,
                llvm_di_builder_create_file, llvm_di_builder_create_function,
                llvm_di_builder_create_lexical_block_file, llvm_di_builder_create_subroutine_type,
                llvm_di_scope_get_file, llvm_get_module_flag, llvm_set_current_debug_location2,
                llvm_set_subprogram,
            },
        },
        op_interfaces::LlvmSymbolName,
        ops::FuncOp,
        to_llvm_ir::ConversionContext,
    };

    /// The amount of debug data to emit.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum EmissionKind {
        /// Line tables and inlined frames only.
        #[default]
        LineTablesOnly,
        /// All the debug data that the locations give.
        Full,
    }

    /// Options for [convert_module_with_debug_info](crate::to_llvm_ir::convert_module_with_debug_info).
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct DebugInfoOptions {
        /// The amount of debug data to emit.
        pub emission_kind: EmissionKind,
        /// The source language of the compile unit.
        pub language: LLVMDWARFSourceLanguage,
        /// The producer of the compile unit.
        pub producer: String,
        /// Whether the code is optimized.
        pub optimized: bool,
        /// The directory for relative file names.
        pub directory: String,
        /// The DWARF version. It is not set if the module already has one.
        pub dwarf_version: u32,
    }

    impl Default for DebugInfoOptions {
        fn default() -> Self {
            Self {
                emission_kind: EmissionKind::default(),
                language: LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC,
                producer: "pliron".to_string(),
                optimized: false,
                directory: String::new(),
                dwarf_version: 4,
            }
        }
    }

    /// State for converting op locations to LLVM debug data.
    pub(crate) struct DIConversionContext {
        options: DebugInfoOptions,
        builder: LLVMDIBuilder,
        /// The compile unit. It is created with the first subprogram.
        unit: Option<LLVMMetadata>,
        subroutine_type: Option<LLVMMetadata>,
        files: HMap<String, LLVMMetadata>,
        /// Subprograms of inlined functions, by name and file.
        inlined: HMap<(String, LLVMMetadata), LLVMMetadata>,
        /// Scopes in a different file, by parent scope and file.
        block_files: HMap<(LLVMMetadata, LLVMMetadata), LLVMMetadata>,
        /// The subprogram of the current function.
        subprogram: Option<LLVMMetadata>,
        /// Converted locations of the current function.
        locations: HMap<Location, LLVMMetadata>,
    }

    impl DIConversionContext {
        pub(crate) fn new(module: &LLVMModule, options: DebugInfoOptions) -> Self {
            Self {
                options,
                builder: LLVMDIBuilder::new(module),
                unit: None,
                subroutine_type: None,
                files: HMap::default(),
                inlined: HMap::default(),
                block_files: HMap::default(),
                subprogram: None,
                locations: HMap::default(),
            }
        }

        /// The `DIFile` of `src`.
        fn file(&mut self, ctx: &Context, src: &Source) -> LLVMMetadata {
            let name = match src {
                Source::File(key) => uniqued_any::get(ctx, *key).to_string_lossy().into_owned(),
                Source::InMemory => "<in-memory>".to_string(),
            };
            if let Some(file) = self.files.get(&name) {
                return *file;
            }
            let file = llvm_di_builder_create_file(&self.builder, &name, &self.options.directory);
            self.files.insert(name, file);
            file
        }

        /// The subroutine type of all subprograms.
        /// The first call also creates the compile unit in `file`.
        fn subroutine_type(&mut self, file: LLVMMetadata) -> LLVMMetadata {
            if let Some(ty) = self.subroutine_type {
                return ty;
            }
            let kind = match self.options.emission_kind {
                EmissionKind::LineTablesOnly => {
                    LLVMDWARFEmissionKind::LLVMDWARFEmissionKindLineTablesOnly
                }
                EmissionKind::Full => LLVMDWARFEmissionKind::LLVMDWARFEmissionKindFull,
            };
            // The C enum has no `Clone`, and the unit is made one time.
            let language = core::mem::replace(
                &mut self.options.language,
                LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC,
            );
            let unit = llvm_di_builder_create_compile_unit(
                &self.builder,
                language,
                file,
                &self.options.producer,
                self.options.optimized,
                kind,
            );
            let ty = llvm_di_builder_create_subroutine_type(&self.builder, file, &[]);
            self.unit = Some(unit);
            self.subroutine_type = Some(ty);
            ty
        }

        /// `scope`, or a lexical block of `scope` in the file of `src`.
        fn scope_in_file(
            &mut self,
            ctx: &Context,
            scope: LLVMMetadata,
            src: &Source,
        ) -> LLVMMetadata {
            let file = self.file(ctx, src);
            if llvm_di_scope_get_file(scope) == Some(file) {
                return scope;
            }
            *self.block_files.entry((scope, file)).or_insert_with(|| {
                llvm_di_builder_create_lexical_block_file(&self.builder, scope, file, 0)
            })
        }

        /// The subprogram of an inlined callee, from its outermost name.
        fn inlined_subprogram(&mut self, ctx: &Context, callee: &Location) -> Option<LLVMMetadata> {
            let callee = outermost(callee);
            let name = name(callee)?.to_string();
            let (src, _) = src_pos(callee).unwrap_or((Source::InMemory, 0));
            let file = self.file(ctx, &src);
            let key = (name, file);
            if let Some(subprogram) = self.inlined.get(&key) {
                return Some(*subprogram);
            }
            let ty = self.subroutine_type(file);
            let subprogram = llvm_di_builder_create_function(
                &self.builder,
                file,
                &key.0,
                "",
                file,
                0,
                ty,
                true,
                true,
                0,
                self.options.optimized,
            );
            self.inlined.insert(key, subprogram);
            Some(subprogram)
        }

        /// The `DILocation` of `loc` in `scope`, inlined at `inlined_at`.
        fn translate(
            &mut self,
            ctx: &Context,
            llvm_ctx: &LLVMContext,
            loc: &Location,
            scope: LLVMMetadata,
            inlined_at: Option<LLVMMetadata>,
        ) -> Option<LLVMMetadata> {
            match loc {
                Location::SrcPos { src, pos } => {
                    let scope = self.scope_in_file(ctx, scope, src);
                    Some(llvm_di_builder_create_debug_location(
                        llvm_ctx,
                        pos.line.max(0) as u32,
                        pos.column.max(0) as u32,
                        scope,
                        inlined_at,
                    ))
                }
                Location::Named { child_loc, .. } => {
                    self.translate(ctx, llvm_ctx, child_loc, scope, inlined_at)
                }
                Location::CallSite { callee, caller } => {
                    let Some(caller) = self.translate(ctx, llvm_ctx, caller, scope, inlined_at)
                    else {
                        return self.translate(ctx, llvm_ctx, callee, scope, inlined_at);
                    };
                    let Some(callee_scope) = self.inlined_subprogram(ctx, callee) else {
                        return Some(caller);
                    };
                    self.translate(ctx, llvm_ctx, callee, callee_scope, Some(caller))
                        .or(Some(caller))
                }
                Location::Fused { locations, .. } => locations
                    .iter()
                    .find_map(|loc| self.translate(ctx, llvm_ctx, loc, scope, inlined_at)),
                Location::Unknown => None,
            }
        }
    }

    /// The outermost frame of `loc`: the last caller of a call site chain.
    fn outermost(mut loc: &Location) -> &Location {
        while let Location::CallSite { caller, .. } = loc {
            loc = caller;
        }
        loc
    }

    /// The first name in a frame.
    fn name(loc: &Location) -> Option<&str> {
        match loc {
            Location::Named { name, .. } => Some(name),
            Location::Fused { locations, .. } => locations.iter().find_map(name),
            Location::CallSite { caller, .. } => name(caller),
            Location::SrcPos { .. } | Location::Unknown => None,
        }
    }

    /// The first source position in a frame.
    fn src_pos(loc: &Location) -> Option<(Source, u32)> {
        match loc {
            Location::SrcPos { src, pos } => Some((*src, pos.line.max(0) as u32)),
            Location::Named { child_loc, .. } => src_pos(child_loc),
            Location::Fused { locations, .. } => locations.iter().find_map(src_pos),
            Location::CallSite { caller, .. } => src_pos(caller),
            Location::Unknown => None,
        }
    }

    /// The location of `func_op`, or else the first known location in its body.
    fn function_location(ctx: &Context, func_op: FuncOp) -> Option<Location> {
        let loc = func_op.loc(ctx);
        if !loc.is_unknown() {
            return Some(loc);
        }
        let result = walk_op(
            ctx,
            &mut (),
            &WALKCONFIG_PREORDER_FORWARD,
            func_op.get_operation(),
            |ctx: &Context, _: &mut (), node: IRNode| -> WalkResult<Location> {
                let IRNode::Operation(op) = node else {
                    return walk_advance();
                };
                let loc = op.deref(ctx).loc();
                if loc.is_unknown() {
                    walk_advance()
                } else {
                    walk_break(outermost(&loc).clone())
                }
            },
        );
        match result {
            WalkResult::Break(loc) => Some(loc),
            WalkResult::Continue(_) => None,
        }
    }

    /// Create the subprogram of `func_op`, which converts to `func_llvm`.
    /// A function without a location gets no subprogram, and its instructions get no location.
    pub(crate) fn begin_function(
        ctx: &Context,
        cctx: &mut ConversionContext,
        func_op: FuncOp,
        func_llvm: LLVMValue,
    ) {
        let Some(di) = cctx.di.as_mut() else {
            return;
        };
        let symbol = func_op.get_symbol_name(ctx);
        let llvm_name = func_op
            .llvm_symbol_name(ctx)
            .unwrap_or_else(|| symbol.to_string());
        di.subprogram = None;
        di.locations.clear();
        let Some(loc) = function_location(ctx, func_op) else {
            return;
        };
        let loc = outermost(&loc);
        let name = name(loc).unwrap_or(&llvm_name).to_string();
        let (src, line) = src_pos(loc).unwrap_or((Source::InMemory, 0));
        let file = di.file(ctx, &src);
        let ty = di.subroutine_type(file);
        let linkage_name = if name == llvm_name { "" } else { &llvm_name };
        let subprogram = llvm_di_builder_create_function(
            &di.builder,
            file,
            &name,
            linkage_name,
            file,
            line,
            ty,
            false,
            true,
            line,
            di.options.optimized,
        );
        llvm_set_subprogram(func_llvm, subprogram);
        di.subprogram = Some(subprogram);
    }

    /// Set the location of the instructions that `loc` converts to.
    pub(crate) fn set_location(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        loc: &Location,
    ) {
        let Some(di) = cctx.di.as_mut() else {
            return;
        };
        let Some(subprogram) = di.subprogram else {
            llvm_set_current_debug_location2(&cctx.builder, None);
            return;
        };
        let di_loc = match di.locations.get(loc) {
            Some(di_loc) => *di_loc,
            None => {
                let di_loc = di
                    .translate(ctx, llvm_ctx, loc, subprogram, None)
                    .unwrap_or_else(|| {
                        llvm_di_builder_create_debug_location(llvm_ctx, 0, 0, subprogram, None)
                    });
                di.locations.insert(loc.clone(), di_loc);
                di_loc
            }
        };
        llvm_set_current_debug_location2(&cctx.builder, Some(di_loc));
    }

    /// Add the module flags that the debug data needs, and finalize it.
    pub(crate) fn finish(llvm_ctx: &LLVMContext, cctx: &mut ConversionContext) {
        let Some(di) = cctx.di.take() else {
            return;
        };
        llvm_set_current_debug_location2(&cctx.builder, None);
        if di.unit.is_some() {
            let int32 = llvm_int_type_in_context(llvm_ctx, 32);
            for (key, value) in [
                ("Debug Info Version", llvm_debug_metadata_version()),
                ("Dwarf Version", di.options.dwarf_version),
            ] {
                if llvm_get_module_flag(cctx.cur_llvm_module, key).is_none() {
                    let value = llvm_value_as_metadata(llvm_const_int(int32, value.into(), false));
                    llvm_add_module_flag(
                        cctx.cur_llvm_module,
                        LLVMModuleFlagBehavior::LLVMModuleFlagBehaviorWarning,
                        key,
                        value,
                    );
                }
            }
        }
    }
}
