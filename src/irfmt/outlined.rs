//! Printer and parser for [outlined](OutlinedAttr) attributes.
//! Outlined attributes are printed in a separate section of the
//! IR, after the top level operation is printed.

use core::{borrow::Borrow, hash::Hash};

use alloc::{boxed::Box, vec::Vec};
use rustc_hash::FxHashMap;

use crate::{
    attribute::{AttrObj, Attribute, AttributeDict, attr_cast, attr_impls},
    basic_block::BasicBlock,
    builtin::attr_interfaces::{OutlinedAttr, PrintOnceAttr},
    combine::{Parser, between, optional, parser::char::spaces, token},
    context::{Context, Ptr},
    dict_key,
    identifier::Identifier,
    input_err, input_error,
    location::{Located, Location},
    operation::Operation,
    parsable::{Parsable, StateStream},
    printable::{self, Printable},
    result::Result,
    utils::vec_exns::VecExtns,
};

use super::parsers::{delimited_list_parser, location, spaced, zero_or_more_parser};

/// An item (operation or block) that has something to record in the outlined section.
enum OutlinedItem {
    Op(Ptr<Operation>),
    Block(Ptr<BasicBlock>),
}

// Implement `Hash`, `PartialEq`, `Eq` for `Box<dyn PrintOnceAttr>`
// so that we can use it as a key in `print_once_attrs map`.
struct PrintOnceAttrWrapper(Box<dyn PrintOnceAttr>);

impl core::hash::Hash for PrintOnceAttrWrapper {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash_attr().hash(state);
    }
}

impl PartialEq for PrintOnceAttrWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_attr(&*other.0)
    }
}

impl Eq for PrintOnceAttrWrapper {}

// To enable looking up `PrintOnceAttrWrapper` in the map using a `&dyn PrintOnceAttr`.
impl Borrow<dyn PrintOnceAttr> for PrintOnceAttrWrapper {
    fn borrow(&self) -> &dyn PrintOnceAttr {
        &*self.0
    }
}

// To enable looking up `PrintOnceAttrWrapper` in the map using a `&dyn PrintOnceAttr`.
impl Hash for dyn PrintOnceAttr {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.hash_attr().hash(state);
    }
}

// To enable looking up `PrintOnceAttrWrapper` in the map using a `&dyn PrintOnceAttr`.
impl PartialEq for dyn PrintOnceAttr {
    fn eq(&self, other: &Self) -> bool {
        self.eq_attr(other)
    }
}

impl Eq for dyn PrintOnceAttr {}

#[derive(Default)]
struct OutlinePrintState {
    /// Items (operations or blocks) that have some outline item to be printed.
    outlined_items: Vec<OutlinedItem>,
    /// [PrintOnceAttr]s, mapped to their outindex.
    print_once_attrs: FxHashMap<PrintOnceAttrWrapper, usize>,
}

dict_key!(OUTLINED_STATE, "outlined_state");

/// An [Operation] was just printed, and we now print a future reference to the
/// outlined attributes (if any) or location (if any) that will be printed later.
pub(crate) fn preprint_outline_operation(
    ctx: &Context,
    opr: Ptr<Operation>,
    print_state: printable::State,
    f: &mut core::fmt::Formatter<'_>,
) -> core::fmt::Result {
    let mut aux_print_data = print_state.aux_data_mut();
    let print_state = aux_print_data
        .entry(OUTLINED_STATE.clone())
        .or_insert(Box::new(OutlinePrintState::default()))
        .downcast_mut::<OutlinePrintState>()
        .expect("failed to downcast outline print state");

    let op = opr.deref(ctx);

    // If it has a location, we need to outline the location.
    if !op.loc().is_unknown() {
        let outindex = print_state.outlined_items.push_back(OutlinedItem::Op(opr));
        return write!(f, " !{outindex}");
    }

    // Check if there's any attribute that's outlined.
    if op
        .attributes
        .0
        .iter()
        .any(|(_, attr)| attr_impls::<dyn OutlinedAttr>(&**attr))
    {
        let outindex = print_state.outlined_items.push_back(OutlinedItem::Op(opr));
        return write!(f, " !{outindex}");
    }

    Ok(())
}

/// A [BasicBlock] was just printed (label + args), and we now print a future reference to the
/// outlined attributes (if any) or location (if any) that will be printed later.
pub(crate) fn preprint_outline_block(
    ctx: &Context,
    block: Ptr<BasicBlock>,
    print_state: printable::State,
    f: &mut core::fmt::Formatter<'_>,
) -> core::fmt::Result {
    let mut aux_print_data = print_state.aux_data_mut();
    let print_state = aux_print_data
        .entry(OUTLINED_STATE.clone())
        .or_insert(Box::new(OutlinePrintState::default()))
        .downcast_mut::<OutlinePrintState>()
        .expect("failed to downcast outline print state");

    let bl = block.deref(ctx);

    // If it has a location, we need to outline the location.
    if !bl.loc().is_unknown() {
        let outindex = print_state
            .outlined_items
            .push_back(OutlinedItem::Block(block));
        return write!(f, " !{outindex}");
    }

    // Check if there's any attribute that's outlined.
    if bl
        .attributes
        .0
        .iter()
        .any(|(_, attr)| attr_impls::<dyn OutlinedAttr>(&**attr))
    {
        let outindex = print_state
            .outlined_items
            .push_back(OutlinedItem::Block(block));
        return write!(f, " !{outindex}");
    }

    Ok(())
}

/// Print the outlined attributes and locations.
/// This is called after all operations have been printed.
pub(crate) fn print_outlines(
    ctx: &Context,
    print_state: printable::State,
    f: &mut core::fmt::Formatter<'_>,
) -> core::fmt::Result {
    let Some(print_state) = print_state.aux_data_mut().remove(&*OUTLINED_STATE) else {
        return Ok(());
    };

    let mut print_state = *print_state
        .downcast::<OutlinePrintState>()
        .expect("failed to downcast outline print state");

    if print_state.outlined_items.is_empty() {
        return Ok(());
    }

    writeln!(f, "\n\noutlined_attributes:")?;
    let mut print_once_attr_indices = print_state.outlined_items.len();

    // A helper function so we don't duplicate the per-attribute printing logic.
    fn print_outlined_attrs_for(
        ctx: &Context,
        f: &mut core::fmt::Formatter<'_>,
        print_once_attrs: &mut FxHashMap<PrintOnceAttrWrapper, usize>,
        print_once_attr_indices: &mut usize,
        attributes: &AttributeDict,
        loc: Location,
    ) -> core::fmt::Result {
        if !loc.is_unknown() {
            write!(f, "@[{}], ", loc.disp(ctx))?;
        }
        write!(f, "[")?;
        let mut first = true;
        for (attr_name, attr) in attributes.0.iter() {
            if attr_impls::<dyn OutlinedAttr>(&**attr) {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                if let Some(print_once_attr) = attr_cast::<dyn PrintOnceAttr>(&**attr) {
                    if let Some(outindex) = print_once_attrs.get(print_once_attr) {
                        write!(f, "{attr_name} = !{outindex}")?;
                    } else {
                        // If this is the first time we see this PrintOnceAttr,
                        // we need to store it for later.
                        print_once_attrs.insert(
                            PrintOnceAttrWrapper(dyn_clone::clone_box(print_once_attr)),
                            *print_once_attr_indices,
                        );
                        write!(f, "{attr_name} = !{print_once_attr_indices}")?;
                        *print_once_attr_indices += 1;
                    }
                } else {
                    write!(f, "{} = {}", attr_name, attr.disp(ctx))?;
                }
            }
        }
        Ok(())
    }

    for (outidx, item) in print_state.outlined_items.iter().enumerate() {
        write!(f, "!{outidx} = ")?;
        match item {
            OutlinedItem::Op(op) => {
                let opr = op.deref(ctx);
                print_outlined_attrs_for(
                    ctx,
                    f,
                    &mut print_state.print_once_attrs,
                    &mut print_once_attr_indices,
                    &opr.attributes,
                    opr.loc(),
                )?;
            }
            OutlinedItem::Block(block) => {
                let bl = block.deref(ctx);
                print_outlined_attrs_for(
                    ctx,
                    f,
                    &mut print_state.print_once_attrs,
                    &mut print_once_attr_indices,
                    &bl.attributes,
                    bl.loc(),
                )?;
            }
        }
        writeln!(f, "]")?;
    }

    // Now print the PrintOnceAttrs, if any.
    if !print_state.print_once_attrs.is_empty() {
        for (attr, outindex) in print_state.print_once_attrs {
            let attr = attr.0 as Box<dyn Attribute>;
            writeln!(f, "!{} = {}", outindex, attr.disp(ctx))?;
        }
    }

    Ok(())
}

/// The state used to parse outlined attributes and locations.
#[derive(Default)]
struct OutlineParseState {
    /// Map an outline item number to the [Operation] it refers to.
    outindex_op_map: FxHashMap<usize, Ptr<Operation>>,
    /// Map an outline item number to the [BasicBlock] it refers to.
    outindex_block_map: FxHashMap<usize, Ptr<BasicBlock>>,
}

/// Each [Operation] is associated with an optional [Location] and
/// a number of outlined attributes or references to them.
enum AttrOrOutlineEntryRef {
    Attr(AttrObj),
    /// The location here is where the entry ref (`!<outindex>`) is,
    /// mainly for reporting errors.
    OutlineEntryRef((Location, usize)),
}

enum OutlineEntry {
    /// A [PrintOnceAttr] is an entry by itself.
    PrintOnceAttr(AttrObj),
    /// The outline entry for an [Operation] or [BasicBlock]:
    /// A location and a list of attributes or references to attributes.
    LocAndOutlinedAttrs(Option<Location>, Vec<(Identifier, AttrOrOutlineEntryRef)>),
}

/// An [Operation] was just parsed, see if it has any outlined item number and note that down.
pub(crate) fn postparse_outline(state_stream: &mut StateStream, op: Ptr<Operation>) -> Result<()> {
    let mut outindex_parser = spaces().with(optional(combine::token('!').with(usize::parser(()))));

    let loc = state_stream.loc();
    let outindex = match outindex_parser.parse_stream(state_stream).into_result() {
        Ok((Some(outindex), _)) => outindex,
        Ok((None, _)) => {
            // No outline index, nothing to do.
            return Ok(());
        }
        Err(e) => {
            return input_err!(
                loc,
                "Error parsing outline index for operation: {}",
                e.into_inner().error
            );
        }
    };

    let parse_state = state_stream
        .state
        .aux_data
        .entry(OUTLINED_STATE.clone())
        .or_insert(Box::new(OutlineParseState::default()))
        .downcast_mut::<OutlineParseState>()
        .expect("failed to downcast outline parse state");

    if parse_state.outindex_op_map.insert(outindex, op).is_some() {
        return input_err!(loc, "Duplicate outline index: {}", outindex);
    }

    Ok(())
}

/// Register a [BasicBlock] with a given outline index in the parse state.
/// Called from [BasicBlock](crate::basic_block::BasicBlock) parsing after the block is
/// created, with the outline index that was already parsed from the stream.
pub(crate) fn register_block_for_outline(
    state_stream: &mut StateStream,
    outindex: usize,
    block: Ptr<BasicBlock>,
    loc: crate::location::Location,
) -> Result<()> {
    let parse_state = state_stream
        .state
        .aux_data
        .entry(OUTLINED_STATE.clone())
        .or_insert(Box::new(OutlineParseState::default()))
        .downcast_mut::<OutlineParseState>()
        .expect("failed to downcast outline parse state");

    if parse_state
        .outindex_block_map
        .insert(outindex, block)
        .is_some()
    {
        return input_err!(loc, "Duplicate outline index: {}", outindex);
    }

    Ok(())
}

/// Parse the outlined attributes and locations.
pub(crate) fn parse_outlines(state_stream: &mut StateStream) -> Result<()> {
    let Some(parse_state) = state_stream.state.aux_data.remove(&*OUTLINED_STATE) else {
        return Ok(());
    };

    let mut parse_state = *parse_state
        .downcast::<OutlineParseState>()
        .expect("failed to downcast outline parse state");

    if parse_state.outindex_op_map.is_empty() && parse_state.outindex_block_map.is_empty() {
        return Ok(());
    }

    let outindex_parser = || (location(), token('!').with(usize::parser(())));

    // We'll first try to parse `OutlineEntry::LocAndOutlinedAttrs` entries.
    let optional_loc_parser = optional(
        token('@')
            .with(between(
                token('['),
                token(']'),
                spaced(Location::parser(())),
            ))
            .skip(spaced(token(','))),
    );
    let name_attr_parser = (
        Identifier::parser(()).skip(spaced(token('='))),
        AttrObj::parser(())
            .map(AttrOrOutlineEntryRef::Attr)
            .or(outindex_parser().map(AttrOrOutlineEntryRef::OutlineEntryRef)),
    );
    let name_attrs_parser = delimited_list_parser('[', ']', ',', name_attr_parser);
    let loc_and_outlined_attrs_parser = (optional_loc_parser, spaces().with(name_attrs_parser))
        .map(|(loc, name_attrs)| OutlineEntry::LocAndOutlinedAttrs(loc, name_attrs));

    let print_once_attr_parser = AttrObj::parser(()).map(OutlineEntry::PrintOnceAttr);

    let outline_entry_parser = (
        outindex_parser().skip(spaced(token('='))),
        loc_and_outlined_attrs_parser.or(print_once_attr_parser),
    );

    let start_loc = state_stream.loc();
    let (mut entries, _): (Vec<((Location, usize), OutlineEntry)>, _) =
        spaced(combine::parser::char::string("outlined_attributes:"))
            .with(zero_or_more_parser(outline_entry_parser))
            .parse_stream(state_stream)
            .into_result()
            .map_err(|e| {
                let e = e.into_inner().error;
                let loc = if let Some(src) = start_loc.source() {
                    // There's a source, use it to create a more precise location.
                    Location::SrcPos {
                        src,
                        pos: e.position,
                    }
                } else {
                    // No source, use the start location.
                    start_loc
                };
                input_error!(loc, "Error parsing outline entries: {}", e)
            })?;

    // Separate the two kinds of entries we have.
    let print_once_entries: FxHashMap<_, _> = entries
        .extract_if(0..entries.len(), |e| {
            matches!(e.1, OutlineEntry::PrintOnceAttr(_))
        })
        .map(|(outindex, entry)| {
            if let OutlineEntry::PrintOnceAttr(attr) = entry {
                (outindex.1, attr)
            } else {
                unreachable!("print_once_entries should only contain PrintOnceAttr entries")
            }
        })
        .collect();

    let loc_and_outline_entries = entries
        .into_iter()
        .map(|(outindex, entry)| {
            let OutlineEntry::LocAndOutlinedAttrs(loc, attrs) = entry else {
                unreachable!(
                    "loc_and_outline_entries should only contain LocAndOutlinedAttrs entries"
                );
            };
            (outindex, (loc, attrs))
        })
        .collect::<Vec<_>>();

    for ((outindex_loc, outindex), (loc_opt, named_attrs)) in loc_and_outline_entries {
        // Check if this index refers to an operation or a block.
        if let Some(opr) = parse_state.outindex_op_map.remove(&outindex) {
            if let Some(loc) = loc_opt {
                opr.deref_mut(state_stream.state.ctx).set_loc(loc);
            }
            for (name, attr_or_ref) in named_attrs {
                match attr_or_ref {
                    AttrOrOutlineEntryRef::Attr(attr) => {
                        opr.deref_mut(state_stream.state.ctx)
                            .attributes
                            .0
                            .insert(name, attr);
                    }
                    AttrOrOutlineEntryRef::OutlineEntryRef((ref_outindex_loc, ref_outindex)) => {
                        if let Some(attr) = print_once_entries.get(&ref_outindex) {
                            opr.deref_mut(state_stream.state.ctx)
                                .attributes
                                .0
                                .insert(name, attr.clone());
                        } else {
                            return input_err!(
                                ref_outindex_loc,
                                "No PrintOnceAttr found for outline index: {}",
                                ref_outindex
                            );
                        }
                    }
                }
            }
        } else if let Some(block) = parse_state.outindex_block_map.remove(&outindex) {
            if let Some(loc) = loc_opt {
                block.deref_mut(state_stream.state.ctx).set_loc(loc);
            }
            for (name, attr_or_ref) in named_attrs {
                match attr_or_ref {
                    AttrOrOutlineEntryRef::Attr(attr) => {
                        block
                            .deref_mut(state_stream.state.ctx)
                            .attributes
                            .0
                            .insert(name, attr);
                    }
                    AttrOrOutlineEntryRef::OutlineEntryRef((ref_outindex_loc, ref_outindex)) => {
                        if let Some(attr) = print_once_entries.get(&ref_outindex) {
                            block
                                .deref_mut(state_stream.state.ctx)
                                .attributes
                                .0
                                .insert(name, attr.clone());
                        } else {
                            return input_err!(
                                ref_outindex_loc,
                                "No PrintOnceAttr found for outline index: {}",
                                ref_outindex
                            );
                        }
                    }
                }
            }
        } else {
            return input_err!(
                outindex_loc,
                "No operation or block found for outline index: {}",
                outindex
            );
        }
    }

    Ok(())
}
