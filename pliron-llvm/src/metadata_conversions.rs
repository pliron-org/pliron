// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Conversion of [LLVM metadata](crate::metadata) to and from LLVM-IR.

/// Conversion of LLVM metadata from LLVM-IR, a companion to [crate::from_llvm_ir].
pub mod from_llvm_ir {
    use alloc::{
        string::{String, ToString},
        vec,
        vec::Vec,
    };

    use llvm_sys::{LLVMValueKind, debuginfo::LLVMMetadataKind};
    use pliron::{
        builtin::ops::ModuleOp,
        context::{Context, Ptr},
        input_error_noloc,
        operation::Operation,
        result::Result,
        utils::table::{HMap, HSet},
    };
    use thiserror::Error;

    use crate::{
        from_llvm_ir::{ConversionContext, const_llvm_value_to_attr},
        llvm_sys::core::{
            LLVMMetadata, LLVMModule, LLVMValue, llvm_get_md_kind_id_in_module,
            llvm_get_md_node_operands, llvm_get_md_string, llvm_get_metadata_kind,
            llvm_get_named_metadata_operands, llvm_get_value_kind, llvm_get_value_name,
            llvm_global_copy_all_metadata, llvm_instruction_get_all_metadata_other_than_debug_loc,
            llvm_md_node_in_module, llvm_metadata_as_value_in_module, llvm_named_metadata_names,
            llvm_print_module_to_string, llvm_print_value_to_string, llvm_value_as_metadata,
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
                match llvm_get_value_kind(val) {
                    LLVMValueKind::LLVMGlobalVariableValueKind
                    | LLVMValueKind::LLVMFunctionValueKind => {
                        // A symbol that isn't in the pliron module has nothing we could refer to.
                        match cctx.symbol_name(val) {
                            Some(name) => Ok(Some(MdOperandAttr::Global(name))),
                            None => {
                                log::warn!(
                                    "Dropping metadata operand referring to \"{}\", which has no \
                                     counterpart in the pliron module",
                                    llvm_get_value_name(val).unwrap_or_default()
                                );
                                Ok(None)
                            }
                        }
                    }
                    _ => match const_llvm_value_to_attr(ctx, cctx, val)? {
                        Some(attr) => Ok(Some(MdOperandAttr::Constant(attr))),
                        None => {
                            log::warn!(
                                "Dropping unsupported constant metadata operand {}",
                                llvm_print_value_to_string(val).unwrap_or_default()
                            );
                            Ok(None)
                        }
                    },
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
                // A debug location. pliron has its own [Location](pliron::location::Location).
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
        let entries = llvm_global_copy_all_metadata(global);
        convert_md_attachments(ctx, cctx, module, entries, m_op)
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

/// Conversion of LLVM metadata to LLVM-IR, a companion to [crate::to_llvm_ir].
pub mod to_llvm_ir {
    use alloc::{string::ToString, vec::Vec};

    use pliron::{
        attribute::attr_cast,
        builtin::ops::ModuleOp,
        context::{Context, Ptr},
        input_err_noloc, input_error_noloc,
        operation::Operation,
        printable::Printable,
        result::Result,
        utils::table::{HMap, HSet},
    };
    use thiserror::Error;

    use crate::{
        llvm_sys::core::{
            LLVMContext, LLVMMetadata, LLVMValue, llvm_add_named_metadata_operand,
            llvm_get_md_kind_id_in_context, llvm_global_set_metadata, llvm_is_a,
            llvm_md_node_in_context2, llvm_md_string_in_context2, llvm_metadata_as_value,
            llvm_metadata_replace_all_uses_with, llvm_set_metadata, llvm_temporary_md_node,
            llvm_value_as_metadata,
        },
        metadata::{
            MdNodeId, MdOperandAttr, MdTableAttr, get_attachments, get_metadata_table,
            get_named_metadata,
        },
        to_llvm_ir::{AttrToLLVMConst, ConversionContext},
    };

    /// State for converting metadata to LLVM.
    #[derive(Default)]
    pub(crate) struct MdConversionContext {
        // The module's metadata table, that metadata references resolve against.
        table: MdTableAttr,
        // Metadata nodes that have already been built.
        node_map: HMap<MdNodeId, LLVMMetadata>,
        // Temporary nodes standing in for nodes that are still being built.
        temporaries: HMap<MdNodeId, LLVMMetadata>,
        // Metadata nodes currently being built, to detect cyclic references.
        in_progress: HSet<MdNodeId>,
    }

    /// Metadata conversion errors.
    #[derive(Error, Debug)]
    pub enum MdToLLVMErr {
        #[error("Metadata node #{0} is not in the module's metadata table")]
        DanglingNodeRef(MdNodeId),
        #[error("Metadata refers to \"{0}\", which is not a global or a function in this module")]
        UndefinedSymbol(String),
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
            MdOperandAttr::Global(name) => {
                let val = cctx
                    .globals_map
                    .get(name)
                    .or_else(|| cctx.function_map.get(name))
                    .ok_or_else(|| {
                        input_error_noloc!(MdToLLVMErr::UndefinedSymbol(name.to_string()))
                    })?;
                Some(llvm_value_as_metadata(*val))
            }
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
}
