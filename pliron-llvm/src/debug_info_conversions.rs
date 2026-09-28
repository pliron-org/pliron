// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

/// Conversion of op locations to LLVM debug data, a companion to [`crate::to_llvm_ir`].
///
/// The conversion maps each located function to a `DISubprogram` and each
/// located op to a `DILocation` in that subprogram. Call site locations become
/// inlined frames. The conversion is lossy: a `DILocation` holds one position,
/// so the conversion keeps the first position of each location.
///
/// Use [`convert_module_with_debug_info`](crate::to_llvm_ir::convert_module_with_debug_info)
/// to get this data. [`DebugInfoOptions`] controls the compile unit and the
/// emission kind.
pub mod to_llvm_ir {
    use alloc::string::{String, ToString};

    use llvm_sys::{LLVMModuleFlagBehavior, debuginfo::LLVMDWARFEmissionKind};
    use pliron::{
        builtin::op_interfaces::SymbolOpInterface,
        context::{Context, Ptr},
        graph::walkers::{
            IRNode, WALKCONFIG_PREORDER_FORWARD,
            interruptible::{WalkResult, immutable::walk_op, walk_advance, walk_break},
        },
        location::{Located, Location, Source},
        op::Op,
        operation::Operation,
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

    /// Options for [`convert_module_with_debug_info`](crate::to_llvm_ir::convert_module_with_debug_info).
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct DebugInfoOptions {
        /// The amount of debug data to emit.
        pub emission_kind: EmissionKind,
        /// The source language of the compile unit.
        /// DWARF has many languages. Thus this field uses the llvm-sys type.
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
    ///
    /// LLVM uniques the `DIFile`, `DILexicalBlockFile`, `DISubroutineType`
    /// and `DILocation` nodes. Thus only the distinct nodes need a map here.
    pub(crate) struct DIConversionContext {
        options: DebugInfoOptions,
        builder: LLVMDIBuilder,
        // The compile unit. It is created with the first subprogram.
        unit: Option<LLVMMetadata>,
        // Subprograms of inlined functions, by name and file.
        inlined: HMap<(String, LLVMMetadata), LLVMMetadata>,
        // The subprogram of the current function.
        subprogram: Option<LLVMMetadata>,
    }

    impl DIConversionContext {
        pub(crate) fn new(module: &LLVMModule, options: DebugInfoOptions) -> Self {
            Self {
                options,
                builder: LLVMDIBuilder::new(module),
                unit: None,
                inlined: HMap::default(),
                subprogram: None,
            }
        }

        /// The `DIFile` of `src`.
        fn file(&self, ctx: &Context, src: &Source) -> LLVMMetadata {
            let name = match src {
                Source::File(key) => uniqued_any::get(ctx, *key).to_string_lossy().into_owned(),
                Source::InMemory => "<in-memory>".to_string(),
            };
            llvm_di_builder_create_file(&self.builder, &name, &self.options.directory)
        }

        /// Create the compile unit in `file`, if it does not exist.
        fn ensure_unit(&mut self, file: LLVMMetadata) {
            if self.unit.is_some() {
                return;
            }
            let kind = match self.options.emission_kind {
                EmissionKind::LineTablesOnly => {
                    LLVMDWARFEmissionKind::LLVMDWARFEmissionKindLineTablesOnly
                }
                EmissionKind::Full => LLVMDWARFEmissionKind::LLVMDWARFEmissionKindFull,
            };
            // The C enum has no `Clone`, and the unit is created one time.
            let language = core::mem::replace(
                &mut self.options.language,
                LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC,
            );
            self.unit = Some(llvm_di_builder_create_compile_unit(
                &self.builder,
                language,
                file,
                &self.options.producer,
                self.options.optimized,
                kind,
            ));
        }

        /// Create the subprogram of a function definition in `file`.
        /// An empty `linkage_name` means none.
        fn create_subprogram(
            &mut self,
            name: &str,
            linkage_name: &str,
            file: LLVMMetadata,
            line: u32,
            is_local_to_unit: bool,
        ) -> LLVMMetadata {
            self.ensure_unit(file);
            let ty = llvm_di_builder_create_subroutine_type(&self.builder, file, &[]);
            llvm_di_builder_create_function(
                &self.builder,
                file,
                name,
                linkage_name,
                file,
                line,
                ty,
                is_local_to_unit,
                true,
                line,
                self.options.optimized,
            )
        }

        /// `scope`, or a lexical block of `scope` in the file of `src`.
        fn scope_in_file(&self, ctx: &Context, scope: LLVMMetadata, src: &Source) -> LLVMMetadata {
            let file = self.file(ctx, src);
            if llvm_di_scope_get_file(scope) == Some(file) {
                scope
            } else {
                llvm_di_builder_create_lexical_block_file(&self.builder, scope, file, 0)
            }
        }

        /// The subprogram of an inlined callee, from its outermost name.
        fn inlined_subprogram(&mut self, ctx: &Context, callee: &Location) -> Option<LLVMMetadata> {
            let callee = outermost(callee);
            let name = frame_name(callee)?;
            let (src, _) = frame_src_pos(callee).unwrap_or((Source::InMemory, 0));
            let file = self.file(ctx, &src);
            let key = (name.to_string(), file);
            if let Some(subprogram) = self.inlined.get(&key) {
                return Some(*subprogram);
            }
            let subprogram = self.create_subprogram(name, "", file, 0, true);
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
                        non_negative(pos.line),
                        non_negative(pos.column),
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

    /// `value`, or 0 if `value` is negative.
    fn non_negative(value: i32) -> u32 {
        u32::try_from(value).unwrap_or(0)
    }

    /// The outermost frame of `loc`: the last caller of a call site chain.
    fn outermost(mut loc: &Location) -> &Location {
        while let Location::CallSite { caller, .. } = loc {
            loc = caller;
        }
        loc
    }

    /// The first name in a frame.
    fn frame_name(loc: &Location) -> Option<&str> {
        match loc {
            Location::Named { name, .. } => Some(name),
            Location::Fused { locations, .. } => locations.iter().find_map(frame_name),
            Location::CallSite { caller, .. } => frame_name(caller),
            Location::SrcPos { .. } | Location::Unknown => None,
        }
    }

    /// The first source position in a frame, as a source and a line.
    fn frame_src_pos(loc: &Location) -> Option<(Source, u32)> {
        match loc {
            Location::SrcPos { src, pos } => Some((*src, non_negative(pos.line))),
            Location::Named { child_loc, .. } => frame_src_pos(child_loc),
            Location::Fused { locations, .. } => locations.iter().find_map(frame_src_pos),
            Location::CallSite { caller, .. } => frame_src_pos(caller),
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
            |ctx: &Context, _state: &mut (), node: IRNode| -> WalkResult<Location> {
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
        di.subprogram = None;
        let Some(loc) = function_location(ctx, func_op) else {
            return;
        };
        let loc = outermost(&loc);
        let llvm_name = func_op
            .llvm_symbol_name(ctx)
            .unwrap_or_else(|| func_op.get_symbol_name(ctx).to_string());
        let name = frame_name(loc).unwrap_or(&llvm_name);
        let linkage_name = if name == llvm_name { "" } else { &llvm_name };
        let (src, line) = frame_src_pos(loc).unwrap_or((Source::InMemory, 0));
        let file = di.file(ctx, &src);
        let subprogram = di.create_subprogram(name, linkage_name, file, line, false);
        llvm_set_subprogram(func_llvm, subprogram);
        di.subprogram = Some(subprogram);
    }

    /// Set the location of the instructions that `op` converts to.
    pub(crate) fn set_location(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        op: Ptr<Operation>,
    ) {
        // Clone the location only after this check. A clone of a nested
        // location allocates, and this function runs for each op.
        let Some(di) = cctx.di.as_mut() else {
            return;
        };
        let loc = op.deref(ctx).loc();
        let di_loc = di.subprogram.map(|subprogram| {
            di.translate(ctx, llvm_ctx, &loc, subprogram, None)
                .unwrap_or_else(|| {
                    llvm_di_builder_create_debug_location(llvm_ctx, 0, 0, subprogram, None)
                })
        });
        llvm_set_current_debug_location2(&cctx.builder, di_loc);
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
