// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! IR objects that are to be printed must implement [Printable].

use alloc::{boxed::Box, rc::Rc, string::String};
use core::{
    any::Any,
    cell::{Ref, RefCell, RefMut},
    fmt::{self, Display},
};

use crate::{
    common_traits::RcShare, context::Context, identifier::Identifier, std_deps::hash::FxHashMap,
};

struct StateInner {
    // Number of spaces per indentation
    indent_width: u16,
    // Current indentation
    cur_indent: u16,
    // Aribtrary state data that different printers may want to use.
    aux_data: FxHashMap<Identifier, Box<dyn Any>>,
}

impl Default for StateInner {
    fn default() -> Self {
        Self {
            indent_width: 2,
            cur_indent: 0,
            aux_data: FxHashMap::default(),
        }
    }
}

/// A light weight reference counted wrapper around a state for [Printable].
#[derive(Default)]
pub struct State(Rc<RefCell<StateInner>>);

impl RcShare for State {
    fn share(&self) -> Self {
        State(Rc::clone(&self.0))
    }
}

impl State {
    /// Number of spaces per indentation
    pub fn indent_width(&self) -> u16 {
        self.0.as_ref().borrow().indent_width
    }

    /// Set the current indentation width
    pub fn set_indent_width(&self, indent_width: u16) {
        self.0.as_ref().borrow_mut().indent_width = indent_width;
    }

    /// What's the indentation we're at right now?
    pub fn current_indent(&self) -> u16 {
        self.0.as_ref().borrow().cur_indent
    }

    /// Increase the current indentation by [Self::indent_width]
    pub fn push_indent(&self) {
        let mut inner = self.0.as_ref().borrow_mut();
        inner.cur_indent += inner.indent_width;
    }

    /// Decrease the current indentation by [Self::indent_width].
    pub fn pop_indent(&self) {
        let mut inner = self.0.as_ref().borrow_mut();
        inner.cur_indent -= inner.indent_width;
    }

    /// Get a reference to the aux data table. The returned [Ref] is borrowed
    /// from the entire [State] object, so release it at the earliest.
    pub fn aux_data_ref(&self) -> Ref<'_, FxHashMap<Identifier, Box<dyn Any>>> {
        Ref::map(self.0.borrow(), |inner| &inner.aux_data)
    }

    /// Get a mutable reference to the aux data table. The returned [RefMut] is borrowed
    /// from the entire [State] object, so release it at the earliest.
    pub fn aux_data_mut(&self) -> RefMut<'_, FxHashMap<Identifier, Box<dyn Any>>> {
        RefMut::map(self.0.borrow_mut(), |inner| &mut inner.aux_data)
    }
}

/// All statements in the block are indented during [fmt](Printable::fmt).
/// Simply wraps the block with [State::push_indent] and [State::pop_indent].
///
/// See [Printable] for example usage.
#[macro_export]
macro_rules! indented_block {
    ($state:ident, { $($tt:tt)* }) => {
        $state.push_indent();
        $($tt)*
        $state.pop_indent();
    }
}

/// An object that implements [Display].
struct Displayable<'t, 'c, T: Printable + ?Sized> {
    t: &'t T,
    ctx: &'c Context,
    state: State,
}

impl<T: Printable + ?Sized> Display for Displayable<'_, '_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.t.fmt(self.ctx, &self.state, f)
    }
}

/// Easy printing of IR objects.
///
/// [disp](Self::disp) calls [print](Self::print) with a default [State],
/// but otherwise, are both equivalent.
///
/// Example:
/// ```
/// use pliron::{context::Context, printable::{State, Printable, ListSeparator}};
/// use core::fmt;
/// struct S {
///     i: i64,
/// }
/// impl Printable for S {
///     fn fmt(&self, _ctx: &Context, _state: &State, f: &mut fmt::Formatter<'_>)
///     -> fmt::Result
///     {
///         write!(f, "{}", self.i)
///     }
/// }
///
/// let ctx = Context::new();
/// assert!(S { i: 108 }.disp(&ctx).to_string() == "108");
/// let state = State::default();
/// assert!(S { i: 0 }.print(&ctx, &state).to_string() == "0");
/// let svec = vec![ S { i: 8 }, S { i: 16 } ];
/// use pliron::{indented_block, printable::indented_nl};
/// indented_block!(state, {
///     assert_eq!(format!("{}{}", indented_nl(&state), S { i: 108 }.print(&ctx, &state)), "\n  108");
/// });
/// ```
pub trait Printable {
    fn fmt(&self, ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result;

    /// Get a [Display]'able object from the given [Context] and default [State].
    fn disp<'t, 'c>(&'t self, ctx: &'c Context) -> Box<dyn Display + 'c>
    where
        't: 'c,
    {
        self.print(ctx, &State::default())
    }

    /// Get a [Display]'able object from the given [Context] and [State].
    fn print<'t, 'c>(&'t self, ctx: &'c Context, state: &State) -> Box<dyn Display + 'c>
    where
        't: 'c,
    {
        Box::new(Displayable {
            t: self,
            ctx,
            state: state.share(),
        })
    }
}
/// Implement [Printable] for a type that already implements [Display].
/// Example:
/// ```
///     struct MyType;
///     impl core::fmt::Display for MyType {
///         fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
///             write!(f, "MyType")
///         }
///     }
///     pliron::impl_printable_for_display!(MyType);
/// ```
#[macro_export]
macro_rules! impl_printable_for_display {
    ($ty_name:ty) => {
        impl $crate::printable::Printable for $ty_name {
            fn fmt(
                &self,
                _ctx: &pliron::context::Context,
                _state: &pliron::printable::State,
                f: &mut core::fmt::Formatter<'_>,
            ) -> core::fmt::Result {
                write!(f, "{}", self)
            }
        }
    };
}

impl_printable_for_display!(&str);
impl_printable_for_display!(String);
impl_printable_for_display!(usize);
impl_printable_for_display!(u64);
impl_printable_for_display!(u32);
impl_printable_for_display!(u16);
impl_printable_for_display!(u8);
impl_printable_for_display!(i64);
impl_printable_for_display!(i32);
impl_printable_for_display!(i16);
impl_printable_for_display!(i8);
impl_printable_for_display!(bool);
impl_printable_for_display!(char);

/// Implement [Printable] for a type that already implements [Debug].
/// Example:
/// ```
///     #[derive(Debug)]
///     struct MyType;
///     pliron::impl_printable_for_debug!(MyType);
/// ```
#[macro_export]
macro_rules! impl_printable_for_debug {
    ($ty_name:ty) => {
        impl $crate::printable::Printable for $ty_name {
            fn fmt(
                &self,
                _ctx: &pliron::context::Context,
                _state: &pliron::printable::State,
                f: &mut core::fmt::Formatter<'_>,
            ) -> core::fmt::Result {
                write!(f, "{:?}", self)
            }
        }
    };
}

impl_printable_for_debug!(*const ());
impl_printable_for_debug!(*mut ());

impl<T: Printable + ?Sized> Printable for &T {
    fn fmt(&self, ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (*self).fmt(ctx, state, f)
    }
}

#[derive(Clone, Copy)]
/// When printing lists, how must they be separated
pub enum ListSeparator {
    /// No separator
    None,
    /// Newline
    Newline,
    /// Character followed by a newline.
    CharNewline(char),
    /// Single character
    Char(char),
    /// Single character followed by a space
    CharSpace(char),
}

impl Printable for ListSeparator {
    fn fmt(&self, _ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListSeparator::None => Ok(()),
            ListSeparator::Newline => fmt_indented_newline(state, f),
            ListSeparator::CharNewline(c) => {
                write!(f, "{c}")?;
                fmt_indented_newline(state, f)
            }
            ListSeparator::Char(c) => write!(f, "{c}"),
            ListSeparator::CharSpace(c) => write!(f, "{c} "),
        }
    }
}

/// Iterate over [Item](Iterator::Item)s in an [Iterator] and print them.
pub fn fmt_iter<I>(
    mut iter: I,
    ctx: &Context,
    state: &State,
    sep: ListSeparator,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result
where
    I: Iterator,
    I::Item: Printable,
{
    if let Some(first) = iter.next() {
        first.fmt(ctx, state, f)?;
    }
    for item in iter {
        sep.fmt(ctx, state, f)?;
        item.fmt(ctx, state, f)?;
    }
    Ok(())
}

/// Print a new line followed by indentation as per current state.
pub fn fmt_indented_newline(state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let align = state.current_indent().into();
    write!(f, "\n{:>align$}", "")?;
    Ok(())
}

struct IndentedNewliner {
    state: State,
}

impl Display for IndentedNewliner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_indented_newline(&self.state, f)
    }
}

/// Print a new line followed by indentation as per current state.
pub fn indented_nl(state: &State) -> impl Display {
    IndentedNewliner {
        state: state.share(),
    }
}
