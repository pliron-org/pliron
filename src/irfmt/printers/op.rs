// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Object specific printers and ir_format directives.

use core::fmt;

use crate::{
    builtin::op_interfaces::{OneRegionInterface, SymbolOpInterface},
    context::Context,
    op::Op,
    printable::{Printable, State},
    r#type::Typed,
};

use super::PrinterFn;

/// Print the operation name and associated symbol of the Op. The Op must implement [SymbolOpInterface].
/// The common pattern is `<opid> @<symbol_name>`. For example a function call would be printed as
/// `call @my_func`.
pub fn symb_op_header<T: Op + SymbolOpInterface>(op: &T) -> impl Printable + '_ {
    PrinterFn(
        move |ctx: &Context, _state: &State, f: &mut fmt::Formatter<'_>| {
            write!(f, "{} @{}", op.get_opid(), op.get_symbol_name(ctx))
        },
    )
}

/// Print the operation name, associated symbol and type of the Op. The Op must implement [SymbolOpInterface] and [Typed].
/// The common pattern is `<opid> @<symbol_name>: <type>`. For example a function header would
/// become `func @my_func: (i32, i32) -> i32`.
pub fn typed_symb_op_header<T: Op + SymbolOpInterface + Typed>(op: &T) -> impl Printable + '_ {
    PrinterFn(
        move |ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>| {
            symb_op_header(op).fmt(ctx, state, f)?;
            write!(f, ": ")?;
            op.get_type(ctx).fmt(ctx, state, f)?;
            Ok(())
        },
    )
}

/// Print the region of an IR object that implements the [OneRegionInterface].
pub fn region<T: Op + OneRegionInterface>(op: &T) -> impl Printable + '_ {
    PrinterFn(
        move |ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>| {
            op.get_region(ctx).fmt(ctx, state, f)
        },
    )
}
