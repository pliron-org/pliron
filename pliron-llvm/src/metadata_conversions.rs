// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Conversion of [LLVM metadata](crate::metadata) to and from LLVM-IR.

pub mod from_llvm_ir {
    use alloc::{
        boxed::Box,
        string::{String, ToString},
        vec,
        vec::Vec,
    };

    use llvm_sys::debuginfo::LLVMMetadataKind;
    use pliron::{
        builtin::ops::ModuleOp,
        combine::stream::position::SourcePosition,
        context::{Context, Ptr},
        input_error_noloc,
        location::{Location, Source},
        operation::Operation,
        result::Result,
        std_deps::path::PathBuf,
        utils::table::{HMap, HSet},
    };
    use thiserror::Error;

    use crate::{
        from_llvm_ir::{ConversionContext, const_llvm_value_to_attr},
        llvm_sys::{
            core::{
                LLVMMetadata, LLVMModule, LLVMValue, llvm_get_md_kind_id_in_module,
                llvm_get_md_node_operands, llvm_get_md_string, llvm_get_metadata_kind,
                llvm_get_named_metadata_operands, llvm_global_copy_all_metadata,
                llvm_instruction_get_all_metadata_other_than_debug_loc, llvm_is_a,
                llvm_md_node_in_module, llvm_metadata_as_value_in_module,
                llvm_named_metadata_names, llvm_print_module_to_string, llvm_print_value_to_string,
                llvm_value_as_metadata,
            },
            debuginfo::{
                llvm_di_file_get_directory, llvm_di_file_get_filename, llvm_di_location_get_column,
                llvm_di_location_get_inlined_at, llvm_di_location_get_line,
                llvm_di_location_get_scope, llvm_di_scope_get_file, llvm_di_subprogram_get_line,
                llvm_get_subprogram, llvm_instruction_get_debug_loc,
            },
        },
        metadata::{
            MdAttachmentsAttr, MdNodeAttr, MdNodeId, MdOperandAttr, MdTableAttr, NamedMdAttr,
            set_attachments, set_metadata_table, set_named_metadata,
        },
    };

    /// State for converting LLVM metadata
    #[derive(Default)]
    pub(crate) struct MdConversionContext {
        /// Already converted metadata nodes, mapped to their entry in [Self::table],
        /// or to `None` if the node is one we cannot represent and dropped.
        node_map: HMap<LLVMMetadata, Option<MdNodeId>>,
        /// The module's metadata table, built up as metadata is converted.
        table: MdTableAttr,
        /// Metadata kind ids mapped to their [names](md_kind_name).
        kind_names: HMap<u32, String>,
        /// Whether the module's textual form has been scanned for metadata kind names.
        kind_names_scraped: bool,
    }

    /// Metadata conversion errors.
    #[derive(Error, Debug)]
    pub enum MdConversionErr {
        #[error("Cannot determine the name of metadata kind id {0}")]
        UnknownKind(u32),
    }

    /// LLVM's metadata kind for an instruction's debug location.
    pub(crate) const MD_KIND_DBG: &str = "dbg";

    /// LLVM metadata kind names that LLVM pre-registers in every `LLVMContext`.
    /// A stale list is not incorrect, it only costs a fallback to [scrape_md_kind_names].
    const FIXED_MD_KIND_NAMES: &[&str] = &[
        "dbg",
        "tbaa",
        "prof",
        "fpmath",
        "range",
        "tbaa.struct",
        "invariant.load",
        "alias.scope",
        "noalias",
        "nontemporal",
        "llvm.mem.parallel_loop_access",
        "nonnull",
        "dereferenceable",
        "dereferenceable_or_null",
        "make.implicit",
        "unpredictable",
        "invariant.group",
        "align",
        "llvm.loop",
        "type",
        "section_prefix",
        "absolute_symbol",
        "associated",
        "callees",
        "irr_loop",
        "llvm.access.group",
        "callback",
        "llvm.preserve.access.index",
        "vcall_visibility",
        "noundef",
        "annotation",
        "nosanitize",
        "func_sanitize",
        "exclude",
        "memprof",
        "callsite",
        "kcfi_type",
        "pcsections",
        "DIAssignID",
        "coro.outside.frame",
        "mmra",
        "noalias.addrspace",
        "callee_type",
        "nofree",
        "captures",
        "alloc_token",
        "implicit.ref",
    ];

    /// Collect every `!name` token that could be a metadata kind name from the textual
    /// form of a module.
    ///
    /// The C-API maps a metadata kind name to its id but not the other way round,
    /// So for kinds that LLVM doesn't pre-register the name can only be recovered
    /// from the module's printed form.
    ///
    /// The scan over-approximates: a token that isn't a kind name just registers
    /// an unused kind, which changes nothing about the module.
    fn scrape_md_kind_names(module_text: &str) -> Vec<String> {
        fn is_name_start(c: u8) -> bool {
            c.is_ascii_alphabetic() || matches!(c, b'-' | b'$' | b'.' | b'_')
        }
        fn is_name_char(c: u8) -> bool {
            c.is_ascii_alphanumeric() || matches!(c, b'-' | b'$' | b'.' | b'_')
        }
        fn hex_digit(c: Option<&u8>) -> Option<u8> {
            c.and_then(|c| (*c as char).to_digit(16)).map(|d| d as u8)
        }

        let text = module_text.as_bytes();
        let mut seen = HSet::default();
        let mut names = vec![];
        let mut idx = 0;
        while idx < text.len() {
            if text[idx] != b'!' {
                idx += 1;
                continue;
            }
            idx += 1;
            // Undo the escaping LLVM's printer applies to a metadata name: a name
            // character stands for itself and every other byte is printed as `\XX`.
            let mut name = vec![];
            while idx < text.len() {
                let c = text[idx];
                if is_name_char(c) && (!name.is_empty() || is_name_start(c)) {
                    name.push(c);
                    idx += 1;
                } else if c == b'\\'
                    && let (Some(hi), Some(lo)) =
                        (hex_digit(text.get(idx + 1)), hex_digit(text.get(idx + 2)))
                {
                    name.push((hi << 4) | lo);
                    idx += 3;
                } else {
                    break;
                }
            }
            // A name whose bytes aren't UTF-8 has no [String] counterpart to register.
            if let Ok(name) = String::from_utf8(name)
                && !name.is_empty()
                && seen.insert(name.clone())
            {
                names.push(name);
            }
        }
        names
    }

    /// The name of the metadata kind `kind_id` in `module`'s context.
    pub(crate) fn md_kind_name(
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        kind_id: u32,
    ) -> Result<String> {
        if cctx.md.kind_names.is_empty() {
            for name in FIXED_MD_KIND_NAMES {
                let id = llvm_get_md_kind_id_in_module(module, name);
                cctx.md
                    .kind_names
                    .entry(id)
                    .or_insert_with(|| name.to_string());
            }
        }

        if let Some(name) = cctx.md.kind_names.get(&kind_id) {
            return Ok(name.clone());
        }

        // An id we don't know: recover all names in use from the module's printed form.
        if !cctx.md.kind_names_scraped {
            cctx.md.kind_names_scraped = true;
            let module_text = llvm_print_module_to_string(module)
                .ok_or_else(|| input_error_noloc!(MdConversionErr::UnknownKind(kind_id)))?;
            for name in scrape_md_kind_names(&module_text) {
                let id = llvm_get_md_kind_id_in_module(module, &name);
                cctx.md.kind_names.entry(id).or_insert(name);
            }
        }

        cctx.md
            .kind_names
            .get(&kind_id)
            .cloned()
            .ok_or_else(|| input_error_noloc!(MdConversionErr::UnknownKind(kind_id)))
    }

    /// Convert an LLVM metadata node, and everything it refers to, into entries of the
    /// module's metadata table, returning the [MdNodeId] of `md` itself.
    fn convert_md_node(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        md: LLVMMetadata,
    ) -> Result<Option<MdNodeId>> {
        if let Some(id) = cctx.md.node_map.get(&md) {
            return Ok(*id);
        }

        // Metadata we have no representation for is dropped.
        let kind = llvm_get_metadata_kind(md);
        if !matches!(kind, LLVMMetadataKind::LLVMMDTupleMetadataKind) {
            log::warn!("Dropping unsupported metadata of kind {kind:?}");
            cctx.md.node_map.insert(md, None);
            return Ok(None);
        }

        // Reserve this node's id before converting its operands: metadata nodes are
        // commonly self referential (`!0 = distinct !{!0, ...}`).
        let id = cctx.md.table.reserve();
        cctx.md.node_map.insert(md, Some(id));

        let md_val = llvm_metadata_as_value_in_module(module, md);
        let llvm_operands = llvm_get_md_node_operands(md_val);
        let mut operands = Vec::with_capacity(llvm_operands.len());
        for operand in &llvm_operands {
            // If any operand cannot be represented, we drop the entire node.
            // (missing operands may make the node inconsistent with its semantics).
            let Some(operand) = convert_md_operand(ctx, cctx, module, *operand)? else {
                // LLVM prints an unnamed node as `<0x...> = !{...}`.
                // We only want its definition for warning.
                let printed = llvm_print_value_to_string(md_val).unwrap_or_default();
                let printed = printed.split_once(" = ").map_or(&*printed, |(_, def)| def);
                log::warn!("Dropping metadata node {printed} with an operand we cannot represent");
                cctx.md.node_map.insert(md, None);
                // The id reserved above goes unused; its empty table entry is harmless.
                return Ok(None);
            };
            operands.push(operand);
        }

        // The C-API can neither tell us whether a node is `distinct` nor create a distinct
        // node directly. Uniquing a node with the same operands answers the question: for a
        // uniqued node LLVM hands back the very same node, for a distinct one it cannot.
        let llvm_md_operands: Vec<_> = llvm_operands
            .iter()
            .map(|operand| operand.map(llvm_value_as_metadata))
            .collect();
        let distinct = llvm_md_node_in_module(module, &llvm_md_operands) != md;

        cctx.md.table.set(
            id,
            if distinct {
                MdNodeAttr::new_distinct_tuple(operands)
            } else {
                MdNodeAttr::new_tuple(operands)
            },
        );

        Ok(Some(id))
    }

    /// Convert one operand of an LLVM metadata node. An `operand` of `None` is LLVM's
    /// `null` operand; a `None` result is an operand that cannot be represented.
    fn convert_md_operand(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        operand: Option<LLVMValue>,
    ) -> Result<Option<MdOperandAttr>> {
        let Some(val) = operand else {
            return Ok(Some(MdOperandAttr::Null));
        };

        // A constant operand comes back as the constant itself and anything else as a value
        // wrapping metadata; going back to metadata classifies both uniformly.
        let md = llvm_value_as_metadata(val);
        match llvm_get_metadata_kind(md) {
            LLVMMetadataKind::LLVMMDStringMetadataKind => match llvm_get_md_string(val) {
                Some(s) => Ok(Some(MdOperandAttr::String(s))),
                None => {
                    log::warn!("Dropping metadata string operand whose contents aren't UTF-8");
                    Ok(None)
                }
            },
            LLVMMetadataKind::LLVMMDTupleMetadataKind => {
                Ok(convert_md_node(ctx, cctx, module, md)?.map(MdOperandAttr::Node))
            }
            LLVMMetadataKind::LLVMConstantAsMetadataMetadataKind => {
                match const_llvm_value_to_attr(ctx, cctx, val)? {
                    Some(attr) => Ok(Some(MdOperandAttr::Constant(attr))),
                    None => {
                        log::warn!(
                            "Dropping unsupported constant metadata operand {}",
                            llvm_print_value_to_string(val).unwrap_or_default()
                        );
                        Ok(None)
                    }
                }
            }
            kind => {
                log::warn!("Dropping unsupported metadata operand of kind {kind:?}");
                Ok(None)
            }
        }
    }

    /// Attach the metadata in `entries` (kind id, node) to the pliron [Operation] `m_op`.
    fn convert_md_attachments(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        entries: Vec<(u32, LLVMMetadata)>,
        m_op: Ptr<Operation>,
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut attachments = MdAttachmentsAttr::new();
        for (kind_id, md) in entries {
            let kind = md_kind_name(cctx, module, kind_id)?;
            if kind == MD_KIND_DBG {
                // Debug data converts to [Location].
                continue;
            }
            let Some(node) = convert_md_node(ctx, cctx, module, md)? else {
                log::warn!("Dropping metadata attached under kind \"{kind}\"");
                continue;
            };
            attachments.set(kind, node);
        }
        if !attachments.is_empty() {
            set_attachments(ctx, m_op, attachments);
        }
        Ok(())
    }

    /// Convert the metadata attached to the LLVM instruction `inst`.
    pub(crate) fn convert_instruction_metadata(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        inst: LLVMValue,
        m_inst: Ptr<Operation>,
    ) -> Result<()> {
        convert_instruction_debug_loc(ctx, inst, m_inst);
        let entries = llvm_instruction_get_all_metadata_other_than_debug_loc(inst);
        convert_md_attachments(ctx, cctx, module, entries, m_inst)
    }

    /// Convert the metadata attached to an LLVM global object.
    pub(crate) fn convert_global_object_metadata(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        global: LLVMValue,
        m_op: Ptr<Operation>,
    ) -> Result<()> {
        if llvm_is_a::function(global) {
            convert_function_debug_loc(ctx, global, m_op);
        }
        let entries = llvm_global_copy_all_metadata(global);
        convert_md_attachments(ctx, cctx, module, entries, m_op)
    }

    /// A position at `line` and `column` in the file of the debug data `scope`.
    fn src_pos(ctx: &Context, scope: LLVMMetadata, line: u32, column: u32) -> Location {
        if line == 0 {
            // Line 0 is LLVM's convention for no source position.
            return Location::Unknown;
        }
        let src = llvm_di_scope_get_file(scope).map_or(Source::InMemory, |file| {
            // A relative file name is relative to the directory of the file.
            let path = PathBuf::from(llvm_di_file_get_directory(file))
                .join(llvm_di_file_get_filename(file));
            Source::new_from_file(ctx, path)
        });
        let pos = SourcePosition {
            line: line.try_into().unwrap_or(i32::MAX),
            column: column.try_into().unwrap_or(i32::MAX),
        };
        Location::SrcPos { src, pos }
    }

    /// The [Location] of the `DILocation` `di_loc`.
    /// Inlined positions becomes [Location::CallSite].
    fn convert_di_location(ctx: &Context, di_loc: LLVMMetadata) -> Location {
        let loc = src_pos(
            ctx,
            llvm_di_location_get_scope(di_loc),
            llvm_di_location_get_line(di_loc),
            llvm_di_location_get_column(di_loc),
        );
        match llvm_di_location_get_inlined_at(di_loc) {
            // TODO: Wrap `loc` in a [Location::Named] with the name of the subprogram
            // of its scope. The C-API has no getter for this name.
            Some(caller) => Location::CallSite {
                callee: Box::new(loc),
                caller: Box::new(convert_di_location(ctx, caller)),
            },
            None => loc,
        }
    }

    /// Set the location of `m_inst` from the debug location of `inst`.
    fn convert_instruction_debug_loc(ctx: &Context, inst: LLVMValue, m_inst: Ptr<Operation>) {
        if let Some(di_loc) = llvm_instruction_get_debug_loc(inst) {
            let loc = convert_di_location(ctx, di_loc);
            m_inst.deref_mut(ctx).set_loc(loc);
        }
    }

    /// Set the location of `m_func` from the subprogram of `function`.
    fn convert_function_debug_loc(ctx: &Context, function: LLVMValue, m_func: Ptr<Operation>) {
        if let Some(subprogram) = llvm_get_subprogram(function) {
            let line = llvm_di_subprogram_get_line(subprogram);
            // TODO: Wrap `loc` in a [Location::Named] with the name of `subprogram`.
            // The C-API has no getter for this name.
            let loc = src_pos(ctx, subprogram, line, 0);
            m_func.deref_mut(ctx).set_loc(loc);
        }
    }

    /// Convert the module's named metadata (`!llvm.module.flags = !{!0, !1}`).
    fn convert_named_metadata(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
    ) -> Result<NamedMdAttr> {
        let mut named = NamedMdAttr::new();
        for name in llvm_named_metadata_names(module) {
            for operand in llvm_get_named_metadata_operands(module, &name) {
                let md = llvm_value_as_metadata(operand);
                let Some(node) = convert_md_node(ctx, cctx, module, md)? else {
                    log::warn!("Dropping an operand of named metadata \"{name}\"");
                    continue;
                };
                named.push(name.clone(), node);
            }
        }
        Ok(named)
    }

    /// Attach the module's metadata to `module_op`.
    ///
    /// Must be called after the module's functions have been converted, since their
    /// instructions are what put most nodes in the table.
    pub(crate) fn convert_module_metadata(
        ctx: &Context,
        cctx: &mut ConversionContext,
        module: &LLVMModule,
        module_op: ModuleOp,
    ) -> Result<()> {
        let named_md = convert_named_metadata(ctx, cctx, module)?;
        if !named_md.is_empty() {
            set_named_metadata(ctx, module_op, named_md);
        }
        if !cctx.md.table.is_empty() {
            set_metadata_table(ctx, module_op, cctx.md.table.clone());
        }
        Ok(())
    }
}

/// Conversion of LLVM metadata (including debug-info) to LLVM-IR,
/// a companion to [crate::to_llvm_ir].
pub mod to_llvm_ir {
    use alloc::{
        string::{String, ToString},
        vec::Vec,
    };

    use llvm_sys::{
        LLVMModuleFlagBehavior,
        debuginfo::{LLVMChecksumKind, LLVMDIFlagZero},
    };
    use pliron::{
        attribute::attr_cast,
        builtin::{op_interfaces::SymbolOpInterface, ops::ModuleOp},
        context::{Context, Ptr},
        graph::walkers::{
            IRNode, WALKCONFIG_PREORDER_FORWARD,
            interruptible::{WalkResult, immutable::walk_op, walk_advance, walk_break},
        },
        input_err_noloc, input_error_noloc,
        location::{Location, Source},
        op::Op,
        operation::Operation,
        printable::Printable,
        result::Result,
        uniqued_any,
        utils::table::{HMap, HSet},
    };
    use thiserror::Error;

    use crate::{
        llvm_sys::{
            core::{
                LLVMContext, LLVMMetadata, LLVMModule, LLVMValue, llvm_add_module_flag,
                llvm_add_named_metadata_operand, llvm_const_int, llvm_get_md_kind_id_in_context,
                llvm_get_module_flag, llvm_global_set_metadata, llvm_int_type_in_context,
                llvm_is_a, llvm_md_node_in_context2, llvm_md_string_in_context2,
                llvm_metadata_as_value, llvm_metadata_replace_all_uses_with,
                llvm_set_current_debug_location2, llvm_set_metadata, llvm_temporary_md_node,
                llvm_value_as_metadata,
            },
            debuginfo::{
                LLVMDIBuilder, llvm_debug_metadata_version, llvm_di_builder_create_compile_unit,
                llvm_di_builder_create_debug_location, llvm_di_builder_create_file,
                llvm_di_builder_create_file_with_checksum, llvm_di_builder_create_function,
                llvm_di_builder_create_lexical_block_file, llvm_di_builder_create_subroutine_type,
                llvm_di_scope_get_file, llvm_set_subprogram,
            },
        },
        metadata::{
            MdNodeId, MdOperandAttr, MdTableAttr, get_attachments, get_metadata_table,
            get_named_metadata,
        },
        op_interfaces::LlvmSymbolName,
        ops::FuncOp,
        to_llvm_ir::{AttrToLLVMConst, ConversionContext},
    };

    /// State for converting metadata to LLVM.
    pub(crate) struct MdConversionContext<'a> {
        // The module's metadata table, that metadata references resolve against.
        table: MdTableAttr,
        // Metadata nodes that have already been built.
        node_map: HMap<MdNodeId, LLVMMetadata>,
        // Temporary nodes standing in for nodes that are still being built.
        temporaries: HMap<MdNodeId, LLVMMetadata>,
        // Metadata nodes currently being built, to detect cyclic references.
        in_progress: HSet<MdNodeId>,
        // State for debug info conversion.
        di: Option<DiConversionContext<'a>>,
    }

    impl<'a> MdConversionContext<'a> {
        pub(crate) fn new(
            module: &'a LLVMModule,
            debug_info: Option<DebugInfoOptions<'a>>,
        ) -> Self {
            Self {
                table: MdTableAttr::default(),
                node_map: HMap::default(),
                temporaries: HMap::default(),
                in_progress: HSet::default(),
                di: debug_info.map(|options| DiConversionContext::new(module, options)),
            }
        }
    }

    /// Metadata conversion errors.
    #[derive(Error, Debug)]
    pub enum MdToLLVMErr {
        #[error("Metadata node #{0} is not in the module's metadata table")]
        DanglingNodeRef(MdNodeId),
        #[error("Metadata operand {0} is not convertible to an LLVM constant")]
        OperandNotConst(String),
        #[error(
            "Metadata node #{0} is `distinct`, but the LLVM C-API can only create a distinct \
             node that refers to itself"
        )]
        UnrepresentableDistinct(MdNodeId),
    }

    /// Build the LLVM metadata node for entry `id` of the module's metadata table,
    /// building whatever it refers to along the way.
    fn convert_md_node(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        id: MdNodeId,
    ) -> Result<LLVMMetadata> {
        if let Some(md) = cctx.md.node_map.get(&id) {
            return Ok(*md);
        }
        // Metadata nodes may be cyclic (`!0 = distinct !{!0, ...}`), which the C-API can only
        // express by creating a temporary node, referring to that, and then replacing it with
        // the real node.
        if cctx.md.in_progress.contains(&id) {
            // A back edge: refer to a temporary that is replaced with the real node once
            // that node has been built.
            let temp = *cctx
                .md
                .temporaries
                .entry(id)
                .or_insert_with(|| llvm_temporary_md_node(llvm_ctx, &[]));
            return Ok(temp);
        }

        let node = cctx
            .md
            .table
            .get(id)
            .cloned()
            .ok_or_else(|| input_error_noloc!(MdToLLVMErr::DanglingNodeRef(id)))?;

        cctx.md.in_progress.insert(id);
        let mut operands = Vec::with_capacity(node.operands().len());
        for operand in node.operands() {
            operands.push(convert_md_operand(ctx, llvm_ctx, cctx, operand)?);
        }
        cctx.md.in_progress.remove(&id);

        let md = llvm_md_node_in_context2(llvm_ctx, &operands);
        if let Some(temp) = cctx.md.temporaries.remove(&id) {
            llvm_metadata_replace_all_uses_with(temp, md);
        }
        cctx.md.node_map.insert(id, md);

        Ok(md)
    }

    /// Build the LLVM metadata for one operand of a metadata node.
    fn convert_md_operand(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        operand: &MdOperandAttr,
    ) -> Result<Option<LLVMMetadata>> {
        let md = match operand {
            MdOperandAttr::Null => None,
            MdOperandAttr::String(s) => Some(llvm_md_string_in_context2(llvm_ctx, s)),
            MdOperandAttr::Node(id) => Some(convert_md_node(ctx, llvm_ctx, cctx, *id)?),
            MdOperandAttr::Constant(attr) => {
                let const_val = attr_cast::<dyn AttrToLLVMConst>(&**attr).ok_or_else(|| {
                    input_error_noloc!(MdToLLVMErr::OperandNotConst(attr.disp(ctx).to_string()))
                })?;
                Some(llvm_value_as_metadata(
                    const_val.convert(ctx, llvm_ctx, cctx)?,
                ))
            }
        };
        Ok(md)
    }

    /// Build every node of the module's metadata table, and add the module's named
    /// metadata (`!llvm.module.flags = !{!0, !1}`).
    pub(crate) fn convert_module_metadata(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        module: ModuleOp,
    ) -> Result<()> {
        let Some(table) = get_metadata_table(ctx, module) else {
            return Ok(());
        };
        cctx.md.table = table;

        for id in 0..cctx.md.table.len() as MdNodeId {
            convert_md_node(ctx, llvm_ctx, cctx, id)?;
        }

        // A node that must not be uniqued with a structurally identical one can only be
        // made `distinct` by being self referential.
        for (id, node) in cctx.md.table.clone().iter() {
            if !node.is_distinct() {
                continue;
            }
            let md = cctx.md.node_map[&id];
            let mut operands = Vec::with_capacity(node.operands().len());
            for operand in node.operands() {
                operands.push(convert_md_operand(ctx, llvm_ctx, cctx, operand)?);
            }
            if llvm_md_node_in_context2(llvm_ctx, &operands) == md {
                return input_err_noloc!(MdToLLVMErr::UnrepresentableDistinct(id));
            }
        }

        if let Some(named) = get_named_metadata(ctx, module) {
            for (name, nodes) in named.iter() {
                for node in nodes {
                    let md = convert_md_node(ctx, llvm_ctx, cctx, *node)?;
                    llvm_add_named_metadata_operand(
                        cctx.cur_llvm_module,
                        name,
                        llvm_metadata_as_value(llvm_ctx, md),
                    );
                }
            }
        }

        Ok(())
    }

    /// Attach the metadata of the pliron [Operation] `op` to the LLVM value it converted to.
    pub(crate) fn convert_md_attachments(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        op: Ptr<Operation>,
        op_llvm: LLVMValue,
    ) -> Result<()> {
        let Some(attachments) = get_attachments(ctx, op) else {
            return Ok(());
        };
        let is_instruction = llvm_is_a::instruction(op_llvm);
        for (kind, node) in attachments
            .iter()
            .map(|(kind, node)| (kind.to_string(), node))
            .collect::<Vec<_>>()
        {
            let md = convert_md_node(ctx, llvm_ctx, cctx, node)?;
            let kind_id = llvm_get_md_kind_id_in_context(llvm_ctx, &kind);
            if is_instruction {
                llvm_set_metadata(op_llvm, kind_id, llvm_metadata_as_value(llvm_ctx, md));
            } else if llvm_is_a::global_object(op_llvm) {
                llvm_global_set_metadata(op_llvm, kind_id, md);
            } else {
                // LLVM's builder constant folds, so an operation carrying metadata can
                // convert to a constant, which has nowhere to hold it. LLVM drops metadata
                // when it folds too.
                log::warn!(
                    "Dropping metadata \"{kind}\" of {}, which did not convert to an instruction",
                    Operation::get_opid(op, ctx)
                );
            }
        }
        Ok(())
    }

    /// The amount of debug data to emit.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum DwarfEmissionKind {
        /// No debug data.
        None,
        /// All the debug data.
        Full,
        /// Line tables and inlined frames only.
        #[default]
        LineTablesOnly,
    }

    /// The source language of a compile unit.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum DwarfSourceLanguage {
        /// C.
        #[default]
        C,
        /// C++.
        CPlusPlus,
        /// Rust.
        Rust,
    }

    /// The text of a source file, to embed in its `DIFile`.
    #[derive(Clone, Debug)]
    pub struct SourceText<'a> {
        /// The contents of the file.
        pub text: &'a str,
        /// Hexadecimal MD5 of [text](Self::text). Not verified.
        pub md5: String,
    }

    /// Debug data options of [ToLLVMOptions](crate::to_llvm_ir::ToLLVMOptions).
    #[derive(Debug)]
    pub struct DebugInfoOptions<'a> {
        /// The amount of debug data to emit.
        pub emission_kind: DwarfEmissionKind,
        /// The source language of the compile unit.
        pub language: DwarfSourceLanguage,
        /// The producer of the compile unit.
        pub producer: String,
        /// Is the code optimized?
        pub optimized: bool,
        /// Base directory for relative file names.
        pub directory: String,
        /// Not set if the module already has one.
        pub dwarf_version: u32,
        /// Text to embed in the `DIFile` of each location path.
        pub source_text: HMap<String, SourceText<'a>>,
    }

    impl Default for DebugInfoOptions<'_> {
        fn default() -> Self {
            Self {
                emission_kind: DwarfEmissionKind::default(),
                language: DwarfSourceLanguage::default(),
                producer: "pliron".to_string(),
                optimized: false,
                directory: String::new(),
                dwarf_version: 4,
                source_text: HMap::default(),
            }
        }
    }

    /// State for converting op locations to debug data.
    struct DiConversionContext<'a> {
        options: DebugInfoOptions<'a>,
        builder: LLVMDIBuilder<'a>,
        // The compile unit. Created with the first subprogram.
        unit: Option<LLVMMetadata>,
        // The `DIFile` of each source.
        files: HMap<Source, LLVMMetadata>,
        // Lexical block files, by their scope and file.
        block_files: HMap<(LLVMMetadata, LLVMMetadata), LLVMMetadata>,
        // Subprograms of inlined functions, by file and name.
        inlined: HMap<LLVMMetadata, HMap<String, LLVMMetadata>>,
        // The subprogram of the function that is being converted.
        subprogram: Option<LLVMMetadata>,
    }

    impl<'a> DiConversionContext<'a> {
        fn new(module: &'a LLVMModule, options: DebugInfoOptions<'a>) -> Self {
            Self {
                options,
                builder: LLVMDIBuilder::new(module),
                unit: None,
                files: HMap::default(),
                block_files: HMap::default(),
                inlined: HMap::default(),
                subprogram: None,
            }
        }

        fn file(&mut self, ctx: &Context, src: &Source) -> LLVMMetadata {
            *self.files.entry(*src).or_insert_with(|| {
                let name = match src {
                    Source::File(key) => uniqued_any::get(ctx, *key).to_string_lossy().into_owned(),
                    Source::InMemory => "<in-memory>".to_string(),
                };
                let directory = &self.options.directory;
                let Some(source) = self.options.source_text.get(&name) else {
                    return llvm_di_builder_create_file(&self.builder, &name, directory);
                };
                llvm_di_builder_create_file_with_checksum(
                    &self.builder,
                    &name,
                    directory,
                    LLVMChecksumKind::CSK_MD5,
                    &source.md5,
                    source.text,
                )
            })
        }

        fn ensure_unit(&mut self, file: LLVMMetadata) {
            if self.unit.is_some() {
                return;
            }
            self.unit = Some(llvm_di_builder_create_compile_unit(
                &self.builder,
                self.options.language.into(),
                file,
                &self.options.producer,
                self.options.optimized,
                "",
                0,
                "",
                self.options.emission_kind.into(),
                0,
                false,
                false,
                "",
                "",
            ));
        }

        fn create_subprogram(
            &mut self,
            name: &str,
            linkage_name: &str,
            file: LLVMMetadata,
            line: u32,
            is_local_to_unit: bool,
        ) -> LLVMMetadata {
            self.ensure_unit(file);
            let ty =
                llvm_di_builder_create_subroutine_type(&self.builder, file, &[], LLVMDIFlagZero);
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
                LLVMDIFlagZero,
                self.options.optimized,
            )
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

        /// Keyed by the outermost name of `callee`.
        fn inlined_subprogram(&mut self, ctx: &Context, callee: &Location) -> Option<LLVMMetadata> {
            let name = frame_name(callee)?;
            let (src, _) = frame_src_pos(callee).unwrap_or((Source::InMemory, 0));
            let file = self.file(ctx, &src);
            if let Some(subprogram) = self.inlined.get(&file).and_then(|names| names.get(name)) {
                return Some(*subprogram);
            }
            let subprogram = self.create_subprogram(name, "", file, 0, true);
            self.inlined
                .entry(file)
                .or_default()
                .insert(name.to_string(), subprogram);
            Some(subprogram)
        }

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

    fn non_negative(value: i32) -> u32 {
        u32::try_from(value).unwrap_or(0)
    }

    /// The first name in the outermost frame of `loc`.
    fn frame_name(loc: &Location) -> Option<&str> {
        match loc {
            Location::Named { name, .. } => Some(name),
            Location::Fused { locations, .. } => locations.iter().find_map(frame_name),
            Location::CallSite { caller, .. } => frame_name(caller),
            Location::SrcPos { .. } | Location::Unknown => None,
        }
    }

    /// The first source and line in the outermost frame of `loc`.
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
                if op.deref(ctx).loc().is_unknown() {
                    walk_advance()
                } else {
                    walk_break(op.deref(ctx).loc().clone())
                }
            },
        );
        match result {
            WalkResult::Break(loc) => Some(loc),
            WalkResult::Continue(_) => None,
        }
    }

    /// Create the subprogram of `func_op`, if debug data is requested.
    pub(crate) fn begin_function_debug_info(
        ctx: &Context,
        cctx: &mut ConversionContext,
        func_op: FuncOp,
        func_llvm: LLVMValue,
    ) {
        let Some(di) = cctx.md.di.as_mut() else {
            return;
        };
        di.subprogram = None;
        let Some(loc) = function_location(ctx, func_op) else {
            return;
        };
        let llvm_name = func_op
            .llvm_symbol_name(ctx)
            .unwrap_or_else(|| func_op.get_symbol_name(ctx).to_string());
        let name = frame_name(&loc).unwrap_or(&llvm_name);
        let linkage_name = if name == llvm_name { "" } else { &llvm_name };
        let (src, line) = frame_src_pos(&loc).unwrap_or((Source::InMemory, 0));
        let file = di.file(ctx, &src);
        let subprogram = di.create_subprogram(name, linkage_name, file, line, false);
        llvm_set_subprogram(func_llvm, subprogram);
        di.subprogram = Some(subprogram);
    }

    /// Set the debug location of the instructions that `op` converts to,
    /// if debug data is requested.
    pub(crate) fn set_debug_location(
        ctx: &Context,
        llvm_ctx: &LLVMContext,
        cctx: &mut ConversionContext,
        op: Ptr<Operation>,
    ) {
        let Some(di) = cctx.md.di.as_mut() else {
            return;
        };
        let op_ref = op.deref(ctx);
        let di_loc = di.subprogram.map(|subprogram| {
            di.translate(ctx, llvm_ctx, op_ref.loc(), subprogram, None)
                .unwrap_or_else(|| {
                    llvm_di_builder_create_debug_location(llvm_ctx, 0, 0, subprogram, None)
                })
        });
        llvm_set_current_debug_location2(&cctx.builder, di_loc);
    }

    /// Add the module flags that the debug data needs, and finalize it.
    pub(crate) fn finish_debug_info(llvm_ctx: &LLVMContext, cctx: &mut ConversionContext) {
        let Some(di) = cctx.md.di.take() else {
            return;
        };
        if di.unit.is_none() {
            return;
        }
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
