// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! `pliron` representation of [LLVM metadata](https://llvm.org/docs/LangRef.html#metadata).
//!
//! LLVM's metadata graph is made of `MDNode`s. The graph may be cyclic, including
//! self referential nodes (like `!0 = distinct !{!0, !"llvm.loop.unroll.disable"}`).
//! Nodes have an identity: structural equivalence does not imply identity equivalence.
//!
//! In `pliron`, every `MDNode` lives in a module level [metadata table](MdTableAttr),
//! and the index in that table [identifies](MdNodeId) the node. This is similar to LLVM's
//! textual format, where every node is printed at module level and referred to by number:
//!
//! ```llvm
//! %10 = load i32, ptr %9, !tbaa !5, !alias.scope !12
//! !5  = !{!6, !6, i64 0}
//! !12 = distinct !{!12, !"copy: argument 1"}
//! ```
//!
//! becomes, in pliron:
//!
//! ```text
//! v10 = llvm.load v9 : builtin.integer si32 !0
//!
//! outlined_attributes:
//! !0 = [llvm_metadata = llvm.md_attachments ["tbaa" = #5, "alias.scope" = #12]]
//! ```
//!
//! with the definitions of `#5` and `#12` in the module's [MdTableAttr].
//!
//! Metadata *attachments* on an [Operation] ([MdAttachmentsAttr]), and the module's
//! named metadata ([NamedMdAttr]), both refer to nodes by [MdNodeId] too.
//! All of [MdTableAttr], [MdAttachmentsAttr] and [NamedMdAttr] are [OutlinedAttr]s,
//! ensuring better readability and simpler formatters (custom printers and parsers
//! don't need to be responsible for metadata, it's automatically taken care of).
//!
//! TODO: Only generic nodes (LLVM's `MDTuple`) are supported. LLVM's specialized
//! debug info nodes (`DILocation`, `DISubprogram`, ...) need extensions in [MdNodeAttr];
//! until then, conversion from LLVM-IR drops them, with a warning.

use alloc::{string::String, vec::Vec};

use thiserror::Error;

use pliron::{
    arg_err,
    attribute::AttrObj,
    builtin::{attr_interfaces::OutlinedAttr, ops::ModuleOp},
    combine::{Parser, attempt, between, choice, not_followed_by, parser::char::spaces, token},
    context::{Context, Ptr},
    derive::{attr_interface_impl, pliron_attr},
    dict_key,
    graph::walkers::{
        IRNode, WALKCONFIG_PREORDER_FORWARD,
        interruptible::{WalkResult, immutable::walk_op, walk_advance, walk_break},
    },
    identifier::Identifier,
    indented_block,
    irfmt::{
        parsers::{delimited_list_parser, list_parser},
        printers::iter_with_sep,
    },
    location::{Located, Location},
    op::Op,
    operation::Operation,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    printable::{self, ListSeparator, Printable, indented_nl},
    result::{Error, Result},
    utils::vec_exns::VecExtns,
    verify_err, verify_error,
};

/// Index of a metadata node in the module's metadata table ([MdTableAttr]).
pub type MdNodeId = u32;

/// Write `s` as a quoted string, escaping what pliron's quoted string parser un-escapes.
///
/// [quoted](pliron::irfmt::printers::quoted) escapes via [Debug](core::fmt::Debug), which
/// the parser won't accept back.
fn write_md_quoted(f: &mut core::fmt::Formatter<'_>, s: &str) -> core::fmt::Result {
    write!(f, "\"")?;
    for c in s.chars() {
        match c {
            '\\' | '"' => write!(f, "\\{c}")?,
            _ => write!(f, "{c}")?,
        }
    }
    write!(f, "\"")
}

dict_key!(
    /// The module level [MdTableAttr] holding every metadata node's definition.
    ATTR_KEY_MD_TABLE, "llvm_metadata_defs"
);

dict_key!(
    /// The module level [NamedMdAttr], LLVM's named metadata.
    ATTR_KEY_NAMED_MD, "llvm_named_metadata"
);

dict_key!(
    /// The [MdAttachmentsAttr] on an [Operation].
    ATTR_KEY_MD_ATTACHMENTS, "llvm_metadata"
);

/// An operand of a metadata node ([MdNodeAttr]).
///
/// Contrary to its name, this isn't an [Attribute](pliron::attribute::Attribute).
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum MdOperandAttr {
    /// A null operand. Printed as `null`.
    Null,
    /// LLVM's `MDString`. Printed as `!"contents"`.
    String(String),
    /// A reference to another node in the module's metadata table. Printed as `#42`.
    Node(MdNodeId),
    /// LLVM's `ConstantAsMetadata` wrapping the address of a global or a function.
    /// Printed as `@symbol_name`.
    Global(Identifier),
    /// LLVM's `ConstantAsMetadata` wrapping a constant value, held as the attribute
    /// that an [llvm.constant](crate::ops::ConstantOp) would carry.
    Constant(AttrObj),
}

impl Printable for MdOperandAttr {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            MdOperandAttr::Null => write!(f, "null"),
            MdOperandAttr::String(s) => {
                write!(f, "!")?;
                write_md_quoted(f, s)
            }
            MdOperandAttr::Node(id) => write!(f, "#{id}"),
            MdOperandAttr::Global(name) => write!(f, "@{name}"),
            MdOperandAttr::Constant(attr) => attr.fmt(ctx, state, f),
        }
    }
}

impl Parsable for MdOperandAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        choice((
            attempt(
                pliron::combine::parser::char::string("null")
                    .skip(not_followed_by(pliron::combine::parser::char::alpha_num())),
            )
            .map(|_| MdOperandAttr::Null),
            token('!')
                .with(String::parser(()))
                .map(MdOperandAttr::String),
            token('#')
                .with(MdNodeId::parser(()))
                .map(MdOperandAttr::Node),
            token('@')
                .with(Identifier::parser(()))
                .map(MdOperandAttr::Global),
            AttrObj::parser(()).map(MdOperandAttr::Constant),
        ))
        .parse_stream(state_stream)
        .into()
    }
}

/// A metadata node: an entry in the module's metadata table ([MdTableAttr]).
///
/// TODO: Only LLVM's generic `MDTuple` is modelled. Typed support for LLVM's
/// specialized debug info nodes would be added as further variants of this enum,
/// holding the scalar fields that those nodes keep outside their operand list.
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum MdNodeAttr {
    /// LLVM's `MDTuple`: `!{ op, op, ... }`, or `distinct !{ op, op, ... }` for a node
    /// that LLVM must not unique with a structurally identical one.
    Tuple {
        distinct: bool,
        operands: Vec<MdOperandAttr>,
    },
}

impl MdNodeAttr {
    /// A uniqued `MDTuple` with the given operands.
    pub fn new_tuple(operands: Vec<MdOperandAttr>) -> Self {
        MdNodeAttr::Tuple {
            distinct: false,
            operands,
        }
    }

    /// A `distinct` `MDTuple` with the given operands.
    pub fn new_distinct_tuple(operands: Vec<MdOperandAttr>) -> Self {
        MdNodeAttr::Tuple {
            distinct: true,
            operands,
        }
    }

    /// Is this a `distinct` node?
    pub fn is_distinct(&self) -> bool {
        let MdNodeAttr::Tuple { distinct, .. } = self;
        *distinct
    }

    /// This node's operands.
    pub fn operands(&self) -> &[MdOperandAttr] {
        let MdNodeAttr::Tuple { operands, .. } = self;
        operands
    }
}

impl Printable for MdNodeAttr {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let MdNodeAttr::Tuple { distinct, operands } = self;
        if *distinct {
            write!(f, "distinct ")?;
        }
        write!(f, "!{{")?;
        iter_with_sep(operands.iter(), ListSeparator::CharSpace(',')).fmt(ctx, state, f)?;
        write!(f, "}}")
    }
}

impl Parsable for MdNodeAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        pliron::combine::optional(attempt(
            pliron::combine::parser::char::string("distinct").skip(spaces()),
        ))
        .and(token('!').with(between(
            token('{').skip(spaces()),
            spaces().with(token('}')),
            list_parser(',', MdOperandAttr::parser(())),
        )))
        .map(|(distinct, operands)| MdNodeAttr::Tuple {
            distinct: distinct.is_some(),
            operands,
        })
        .parse_stream(state_stream)
        .into()
    }
}

/// The module level table of metadata node definitions. A node is referred to,
/// from anywhere in the module, by its [index](MdNodeId) in this table.
///
/// Printed as `[#0 = !{...}, #1 = distinct !{...}]`.
#[pliron_attr(name = "llvm.md_table", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash, Default)]
pub struct MdTableAttr(Vec<MdNodeAttr>);

#[attr_interface_impl]
impl OutlinedAttr for MdTableAttr {}

impl MdTableAttr {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `node` to the table and get the [MdNodeId] it can be referred to by.
    pub fn push(&mut self, node: MdNodeAttr) -> MdNodeId {
        self.0.push_back(node) as MdNodeId
    }

    /// Add `node` to the table, unless it is a non-`distinct` node that is structurally
    /// equal to one already in the table, and get the [MdNodeId] to refer to it by.
    pub fn push_uniqued(&mut self, node: MdNodeAttr) -> MdNodeId {
        if !node.is_distinct()
            && let Some((id, _)) = self.iter().find(|(_, existing)| **existing == node)
        {
            return id;
        }
        self.push(node)
    }

    /// Reserve an id for a node whose operands aren't known yet, so that the node
    /// (or a node it refers to) can refer back to it. Must be followed by
    /// [Self::set](Self::set) for the same id.
    pub fn reserve(&mut self) -> MdNodeId {
        self.push(MdNodeAttr::new_tuple(Vec::new()))
    }

    /// Set the node at `id`, which must already be in the table.
    pub fn set(&mut self, id: MdNodeId, node: MdNodeAttr) {
        self.0[id as usize] = node;
    }

    /// The node at `id`, if there is one.
    pub fn get(&self, id: MdNodeId) -> Option<&MdNodeAttr> {
        self.0.get(id as usize)
    }

    /// Number of nodes in this table.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Is this table empty?
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over `(id, node)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (MdNodeId, &MdNodeAttr)> {
        self.0
            .iter()
            .enumerate()
            .map(|(idx, node)| (idx as MdNodeId, node))
    }
}

impl Printable for MdTableAttr {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        // A module's metadata table can be long, so print an entry per line.
        write!(f, "[")?;
        indented_block!(state, {
            for (id, node) in self.iter() {
                if id != 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}#{id} = ", indented_nl(state))?;
                node.fmt(ctx, state, f)?;
            }
        });
        if !self.is_empty() {
            write!(f, "{}", indented_nl(state))?;
        }
        write!(f, "]")
    }
}

#[derive(Debug, Error)]
#[error("Metadata table entry {0} is out of order; entries must be #0, #1, ... in order")]
pub struct MdTableParseErr(MdNodeId);

impl Parsable for MdTableAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();
        let entry = token('#')
            .with(MdNodeId::parser(()))
            .skip(spaces())
            .skip(token('='))
            .skip(spaces())
            .and(MdNodeAttr::parser(()));

        let (entries, _) = delimited_list_parser('[', ']', ',', entry)
            .parse_stream(state_stream)
            .into_result()?;

        let mut table = MdTableAttr::new();
        for (id, node) in entries {
            if id as usize != table.len() {
                return Err(pliron::input_error!(loc, MdTableParseErr(id))).into_parse_result();
            }
            table.push(node);
        }
        Ok(table).into_parse_result()
    }
}

/// Metadata attached to an [Operation], keyed by LLVM's metadata *kind* name
/// (`"tbaa"`, `"llvm.loop"`, ...), the way `%v = load ..., !tbaa !5` attaches node
/// `!5` under kind `tbaa`.
///
/// Printed as `["tbaa" = #5, "llvm.loop" = #14]`.
#[pliron_attr(name = "llvm.md_attachments", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash, Default)]
pub struct MdAttachmentsAttr(Vec<(String, MdNodeId)>);

#[attr_interface_impl]
impl OutlinedAttr for MdAttachmentsAttr {}

impl MdAttachmentsAttr {
    /// No attachments.
    pub fn new() -> Self {
        Self::default()
    }

    /// The node attached under metadata kind `kind`, if any.
    pub fn get(&self, kind: &str) -> Option<MdNodeId> {
        self.0
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, node)| *node)
    }

    /// Attach `node` under metadata kind `kind`, replacing any existing attachment
    /// for that kind.
    pub fn set(&mut self, kind: impl Into<String>, node: MdNodeId) {
        let kind = kind.into();
        match self.0.iter_mut().find(|(k, _)| *k == kind) {
            Some(entry) => entry.1 = node,
            None => self.0.push((kind, node)),
        }
    }

    /// Remove the attachment for metadata kind `kind`, if any.
    pub fn remove(&mut self, kind: &str) {
        self.0.retain(|(k, _)| k != kind);
    }

    /// Is there no attachment at all?
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over `(kind name, node)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, MdNodeId)> {
        self.0.iter().map(|(kind, node)| (kind.as_str(), *node))
    }
}

impl Printable for MdAttachmentsAttr {
    fn fmt(
        &self,
        _ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "[")?;
        for (idx, (kind, node)) in self.0.iter().enumerate() {
            if idx != 0 {
                write!(f, ", ")?;
            }
            write_md_quoted(f, kind)?;
            write!(f, " = #{node}")?;
        }
        write!(f, "]")
    }
}

impl Parsable for MdAttachmentsAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let entry = String::parser(())
            .skip(spaces())
            .skip(token('='))
            .skip(spaces())
            .and(token('#').with(MdNodeId::parser(())));

        delimited_list_parser('[', ']', ',', entry)
            .map(MdAttachmentsAttr)
            .parse_stream(state_stream)
            .into()
    }
}

/// LLVM's [named metadata](https://llvm.org/docs/LangRef.html#named-metadata-nodes):
/// a module level list of nodes under a name, such as `!llvm.module.flags = !{!0, !1}`.
///
/// Printed as `["llvm.module.flags" = [#0, #1], "llvm.ident" = [#4]]`.
#[pliron_attr(name = "llvm.named_md", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash, Default)]
pub struct NamedMdAttr(Vec<(String, Vec<MdNodeId>)>);

#[attr_interface_impl]
impl OutlinedAttr for NamedMdAttr {}

impl NamedMdAttr {
    /// No named metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// The nodes listed under `name`, if `name` is present.
    pub fn get(&self, name: &str) -> Option<&[MdNodeId]> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, nodes)| nodes.as_slice())
    }

    /// Append `node` to the list under `name`, creating the list if needed.
    pub fn push(&mut self, name: impl Into<String>, node: MdNodeId) {
        let name = name.into();
        match self.0.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1.push(node),
            None => self.0.push((name, alloc::vec![node])),
        }
    }

    /// Is there no named metadata at all?
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over `(name, nodes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[MdNodeId])> {
        self.0
            .iter()
            .map(|(name, nodes)| (name.as_str(), nodes.as_slice()))
    }
}

impl Printable for NamedMdAttr {
    fn fmt(
        &self,
        _ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "[")?;
        for (idx, (name, nodes)) in self.0.iter().enumerate() {
            if idx != 0 {
                write!(f, ", ")?;
            }
            write_md_quoted(f, name)?;
            write!(f, " = [")?;
            for (idx, node) in nodes.iter().enumerate() {
                if idx != 0 {
                    write!(f, ", ")?;
                }
                write!(f, "#{node}")?;
            }
            write!(f, "]")?;
        }
        write!(f, "]")
    }
}

impl Parsable for NamedMdAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let nodes = delimited_list_parser('[', ']', ',', token('#').with(MdNodeId::parser(())));
        let entry = String::parser(())
            .skip(spaces())
            .skip(token('='))
            .skip(spaces())
            .and(nodes);

        delimited_list_parser('[', ']', ',', entry)
            .map(NamedMdAttr)
            .parse_stream(state_stream)
            .into()
    }
}

/// Get the metadata table of `module_op`.
pub fn get_metadata_table(ctx: &Context, module_op: ModuleOp) -> Option<MdTableAttr> {
    module_op
        .get_operation()
        .deref(ctx)
        .attributes
        .get::<MdTableAttr>(&ATTR_KEY_MD_TABLE)
        .cloned()
}

/// Set the metadata table on `module_op`.
pub fn set_metadata_table(ctx: &Context, module_op: ModuleOp, table: MdTableAttr) {
    module_op
        .get_operation()
        .deref_mut(ctx)
        .attributes
        .set(ATTR_KEY_MD_TABLE.clone(), table);
}

/// Get the named metadata of `module_op`.
pub fn get_named_metadata(ctx: &Context, module_op: ModuleOp) -> Option<NamedMdAttr> {
    module_op
        .get_operation()
        .deref(ctx)
        .attributes
        .get::<NamedMdAttr>(&ATTR_KEY_NAMED_MD)
        .cloned()
}

/// Set the named metadata on `module_op`.
pub fn set_named_metadata(ctx: &Context, module_op: ModuleOp, named: NamedMdAttr) {
    module_op
        .get_operation()
        .deref_mut(ctx)
        .attributes
        .set(ATTR_KEY_NAMED_MD.clone(), named);
}

/// Get the metadata attached to `op`.
pub fn get_attachments(ctx: &Context, op: Ptr<Operation>) -> Option<MdAttachmentsAttr> {
    op.deref(ctx)
        .attributes
        .get::<MdAttachmentsAttr>(&ATTR_KEY_MD_ATTACHMENTS)
        .cloned()
}

/// Attach `attachments` to `op`, replacing whatever was attached to it.
pub fn set_attachments(ctx: &Context, op: Ptr<Operation>, attachments: MdAttachmentsAttr) {
    op.deref_mut(ctx)
        .attributes
        .set(ATTR_KEY_MD_ATTACHMENTS.clone(), attachments);
}

/// Attach the node `node` to `op` under the LLVM metadata kind `kind`.
pub fn attach_metadata(ctx: &Context, op: Ptr<Operation>, kind: impl Into<String>, node: MdNodeId) {
    let mut attachments = get_attachments(ctx, op).unwrap_or_default();
    attachments.set(kind, node);
    set_attachments(ctx, op, attachments);
}

/// Starting at `op` and walking up its ancestors, find the enclosing [ModuleOp].
pub fn find_enclosing_module(ctx: &Context, op: Ptr<Operation>) -> Option<ModuleOp> {
    let mut cur = Some(op);
    while let Some(op) = cur {
        if let Some(module_op) = Operation::get_op::<ModuleOp>(op, ctx) {
            return Some(module_op);
        }
        cur = op.deref(ctx).get_parent_op(ctx);
    }
    None
}

/// Starting at `op` and walking up its ancestors, find the [metadata table](MdTableAttr)
/// of the enclosing [ModuleOp].
pub fn find_metadata_table(ctx: &Context, op: Ptr<Operation>) -> Option<MdTableAttr> {
    find_enclosing_module(ctx, op).and_then(|module_op| get_metadata_table(ctx, module_op))
}

/// Error enum for metadata addition.
#[derive(Debug, Error)]
pub enum MdAddErr {
    #[error("Cannot add a metadata node for an operation that is not inside a module")]
    NoEnclosingModule,
}

/// Add `node` to the metadata table of the module enclosing `op`, creating the table
/// if the module doesn't have one yet, and get the [MdNodeId] to refer to it by.
///
/// A non-`distinct` node already in the table isn't added again.
pub fn add_metadata_node(ctx: &Context, op: Ptr<Operation>, node: MdNodeAttr) -> Result<MdNodeId> {
    let Some(module_op) = find_enclosing_module(ctx, op) else {
        let loc = op.deref(ctx).loc();
        return arg_err!(loc, MdAddErr::NoEnclosingModule);
    };
    let mut table = get_metadata_table(ctx, module_op).unwrap_or_default();
    let node_id = table.push_uniqued(node);
    set_metadata_table(ctx, module_op, table);
    Ok(node_id)
}

/// Add `node` to the metadata table of the module enclosing `op`,
/// and attach it to `op` under the LLVM metadata kind `kind`,
/// replacing any existing attachment for that kind.
pub fn attach_new_metadata(
    ctx: &Context,
    op: Ptr<Operation>,
    kind: impl Into<String>,
    node: MdNodeAttr,
) -> Result<MdNodeId> {
    let node_id = add_metadata_node(ctx, op, node)?;
    attach_metadata(ctx, op, kind, node_id);
    Ok(node_id)
}

#[derive(Debug, Error)]
pub enum MetadataVerifyErr {
    #[error("Metadata node #{0} is not in the module's metadata table")]
    DanglingNodeRef(MdNodeId),
    #[error("Metadata is attached here, but the module has no metadata table")]
    NoTable,
}

/// Verify that every metadata reference in the module rooted at `module_op`
/// resolves to a node in the module's metadata table.
///
/// This is not part of [Verify](pliron::common_traits::Verify) for the attributes
/// above, since a reference can only be resolved with the enclosing module in hand.
pub fn verify_metadata(ctx: &Context, module_op: ModuleOp) -> Result<()> {
    let module_op_ptr = module_op.get_operation();
    let table = get_metadata_table(ctx, module_op).unwrap_or_default();
    let num_nodes = table.len() as MdNodeId;
    let loc = module_op_ptr.deref(ctx).loc();

    let check = |node: MdNodeId, loc: Location| -> Result<()> {
        if node >= num_nodes {
            verify_err!(loc, MetadataVerifyErr::DanglingNodeRef(node))?;
        }
        Ok(())
    };

    for (_, node) in table.iter() {
        for operand in node.operands() {
            if let MdOperandAttr::Node(id) = operand {
                check(*id, loc.clone())?;
            }
        }
    }

    if let Some(named) = get_named_metadata(ctx, module_op) {
        for (_, nodes) in named.iter() {
            for node in nodes {
                check(*node, loc.clone())?;
            }
        }
    }

    let mut state = (num_nodes, get_metadata_table(ctx, module_op).is_some());
    let walk_result: WalkResult<Error> = walk_op(
        ctx,
        &mut state,
        &WALKCONFIG_PREORDER_FORWARD,
        module_op_ptr,
        |ctx: &Context,
         (num_nodes, has_table): &mut (MdNodeId, bool),
         node: IRNode|
         -> WalkResult<Error> {
            let IRNode::Operation(op) = node else {
                return walk_advance();
            };
            let Some(attachments) = get_attachments(ctx, op) else {
                return walk_advance();
            };
            let loc = op.deref(ctx).loc();
            if !*has_table && !attachments.is_empty() {
                return walk_break(verify_error!(loc, MetadataVerifyErr::NoTable));
            }
            for (_, node) in attachments.iter() {
                if node >= *num_nodes {
                    return walk_break(verify_error!(
                        loc.clone(),
                        MetadataVerifyErr::DanglingNodeRef(node)
                    ));
                }
            }
            walk_advance()
        },
    );

    match walk_result {
        WalkResult::Break(err) => Err(err),
        WalkResult::Continue(_) => Ok(()),
    }
}
