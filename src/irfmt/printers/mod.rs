// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Printers for IR objects.
//!
//! This module provides a set of reusable printers for IR objects.
//! The printers are also used by the Printable derive macro.

use core::fmt;

pub mod op;

use crate::{
    context::Context,
    printable::{ListSeparator, Printable, State},
};

/// Wrap a function to implement the Printable trait
struct PrinterFn<F>(F);

impl<F> Printable for PrinterFn<F>
where
    F: Fn(&Context, &State, &mut fmt::Formatter<'_>) -> fmt::Result,
{
    fn fmt(&self, ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0)(ctx, state, f)
    }
}

/// Create a [Printable] from a formatting function.
pub fn printable_from_fn<F>(print: F) -> impl Printable
where
    F: Fn(&Context, &State, &mut fmt::Formatter<'_>) -> fmt::Result,
{
    PrinterFn(print)
}

/// Print a string as a quoted string.
pub fn quoted(s: &str) -> impl Printable + '_ {
    printable_from_fn(
        move |_ctx: &Context, _state: &State, f: &mut fmt::Formatter<'_>| write!(f, "{s:?}"),
    )
}

/// Print a list of items separated by `sep`.
pub fn list_with_sep<T: Printable>(items: &[T], sep: ListSeparator) -> impl Printable + '_ {
    iter_with_sep(items.iter(), sep)
}

/// Print an iterator of items separated by `sep`.
pub fn iter_with_sep<I>(iter: I, sep: ListSeparator) -> impl Printable
where
    I: Iterator + Clone,
    I::Item: Printable,
{
    iter_with_sep_by(iter, sep, |item, ctx, state, f| item.fmt(ctx, state, f))
}

/// Print an iterator of items separated by `sep`, using `print_item` to print each item.
pub fn iter_with_sep_by<I, F>(iter: I, sep: ListSeparator, print_item: F) -> impl Printable
where
    I: Iterator + Clone,
    F: Fn(I::Item, &Context, &State, &mut fmt::Formatter<'_>) -> fmt::Result,
{
    printable_from_fn(
        move |ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>| {
            let mut iter = iter.clone();
            if let Some(first) = iter.next() {
                print_item(first, ctx, state, f)?;
            }
            for item in iter {
                sep.fmt(ctx, state, f)?;
                print_item(item, ctx, state, f)?;
            }
            Ok(())
        },
    )
}

/// Print `p` enclosed by `left` and `right`.
pub fn enclosed<P: Printable>(left: &'static str, right: &'static str, p: P) -> impl Printable {
    printable_from_fn(
        move |ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>| {
            write!(f, "{left}")?;
            p.fmt(ctx, state, f)?;
            write!(f, "{right}")
        },
    )
}
