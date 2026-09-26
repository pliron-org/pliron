// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Store, in [Context], a single unique copy of any object.
//!
//! Use [save] to store a value and [get] to access it through a [UniquedKey].
//!
//! Use [Uniqued] when a value-like interface is more convenient.

use core::{
    any::Any,
    fmt::{self, Debug},
    hash::{Hash, Hasher},
    marker::PhantomData,
};

use alloc::boxed::Box;

use crate::{
    combine::Parser,
    context::Context,
    irbuild::decontext::{CloneIntoContext, StableHash},
    parsable::{Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    storage_uniquer::TypeValueHash,
};

/// [Box]ed [Any], used for unique storage.
pub(crate) struct UniquedAny(Box<dyn Any + Send>);

/// A handle to the stored unique copy of an object.
///
/// ```
/// use pliron::{context::Context, uniqued_any::{get, save}};
///
/// let ctx = &mut Context::new();
/// let key = save(ctx, String::from("value"));
/// assert_eq!(get(ctx, key), "value");
/// ```
///
/// Also see [`Uniqued<T>`] for a value-like interface.
#[derive(Debug)]
pub struct UniquedKey<T> {
    index: usize,
    _dummy: PhantomData<T>,
}

impl<T> Clone for UniquedKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for UniquedKey<T> {}

impl<T> PartialEq for UniquedKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl<T> Eq for UniquedKey<T> {}

impl<T: 'static> Hash for UniquedKey<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        core::any::TypeId::of::<T>().hash(state);
    }
}

/// Save a unique copy of an object and get a handle to the saved copy.
pub fn save<T: Any + Hash + Eq + Send>(ctx: &mut Context, t: T) -> UniquedKey<T> {
    let hash = TypeValueHash::new(&t);
    let t = UniquedAny(Box::new(t));
    let eq = |t1: &UniquedAny, t2: &UniquedAny| -> bool {
        t1.0.downcast_ref::<T>() == t2.0.downcast_ref::<T>()
    };
    UniquedKey {
        index: ctx.uniqued_any_store.get_or_create_unique(t, hash, &eq),
        _dummy: PhantomData,
    }
}

/// Given a handle to a stored unique copy of an object, get a reference to the object itself.
pub fn get<T: Any + Hash + Eq>(ctx: &Context, key: UniquedKey<T>) -> &T {
    ctx.uniqued_any_store
        .unique_store
        .get(key.index)
        .expect("Key not found in uniqued store")
        .0
        .downcast_ref::<T>()
        .expect("Type mismatch in uniqued store")
}

/// A value stored once (uniqued) in a [Context].
///
/// This combines a [UniquedKey] with the [save] and [get] operations.
///
/// ```
/// use pliron::{context::Context, uniqued_any::Uniqued};
///
/// let ctx = &mut Context::new();
/// let a = Uniqued::new(ctx, String::from("x + 1"));
/// let b = Uniqued::new(ctx, String::from("x + 1"));
/// let c = Uniqued::new(ctx, String::from("x + 2"));
/// assert_eq!(a, b);
/// assert_ne!(a, c);
/// assert_eq!(a.get(ctx), "x + 1");
/// ```
pub struct Uniqued<T>(UniquedKey<T>);

impl<T: Any + Hash + Eq + Send> Uniqued<T> {
    /// Unique and store the given `value`.
    pub fn new(ctx: &mut Context, value: T) -> Self {
        Self(save(ctx, value))
    }

    /// Get a reference to the stored value.
    pub fn get<'c>(&self, ctx: &'c Context) -> &'c T {
        get(ctx, self.0)
    }

    /// Get the underlying store key.
    pub fn key(&self) -> UniquedKey<T> {
        self.0
    }
}

impl<T> Clone for Uniqued<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Uniqued<T> {}

impl<T> PartialEq for Uniqued<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T> Eq for Uniqued<T> {}

impl<T: 'static> Hash for Uniqued<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T> Debug for Uniqued<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Uniqued").field(&self.0.index).finish()
    }
}

impl<T: Any + Hash + Eq + Send + Printable> Printable for Uniqued<T> {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        self.get(ctx).fmt(ctx, state, f)
    }
}

impl<T> Parsable for Uniqued<T>
where
    T: Any + Hash + Eq + Send + Parsable<Arg = (), Parsed = T>,
{
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        T::parser(())
            .parse_stream(state_stream)
            .map(|value| Self::new(state_stream.state.ctx, value))
            .into_result()
    }
}

impl<T: Any + Hash + Eq + Send + StableHash> StableHash for Uniqued<T> {
    fn stable_hash(&self, ctx: &Context, state: &mut dyn Hasher) {
        self.get(ctx).stable_hash(ctx, state);
    }
}

impl<T: Any + Hash + Eq + Send + CloneIntoContext> CloneIntoContext for Uniqued<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Self {
        let value = self.get(src_ctx).clone_into_context(src_ctx, dst_ctx);
        Self::new(dst_ctx, value)
    }
}

#[cfg(test)]
mod tests {
    use crate::context::Context;
    use alloc::string::String;

    use super::{Uniqued, get, save};

    #[test]
    fn test_uniqued_any() {
        let ctx = &mut Context::new();

        let s1 = String::from("Hello");
        let s1_handle = save(ctx, s1);
        assert!(*get(ctx, s1_handle) == "Hello");

        let s2 = String::from("Hello");
        let s2_handle = save(ctx, s2);
        assert!(s1_handle == s2_handle);

        let s3 = String::from("World");
        let s3_handle = save(ctx, s3);
        assert!(s1_handle != s3_handle);

        let i1 = 71i64;
        let i1_handle = save(ctx, i1);
        assert!(*get(ctx, i1_handle) == i1);
    }

    #[test]
    fn test_uniqued() {
        #[derive(Hash, PartialEq, Eq)]
        struct Meters(u64);
        #[derive(Hash, PartialEq, Eq)]
        struct Feet(u64);

        let ctx = &mut Context::new();

        let a = Uniqued::new(ctx, String::from("Hello"));
        let b = Uniqued::new(ctx, String::from("Hello"));
        let c = Uniqued::new(ctx, String::from("World"));
        assert_eq!(a, b);
        assert_eq!(a.key(), b.key());
        assert_ne!(a, c);
        assert_eq!(a.get(ctx), "Hello");
        assert_eq!(c.get(ctx), "World");

        // Values of different types but identical hashes.
        let u = Uniqued::new(ctx, 0u64);
        let i = Uniqued::new(ctx, 0i64);
        assert_ne!(u.key().index, i.key().index);
        assert_eq!(*u.get(ctx), 0u64);
        assert_eq!(*i.get(ctx), 0i64);

        // Same for two newtypes that wrap an equal value.
        let m = Uniqued::new(ctx, Meters(3));
        let f = Uniqued::new(ctx, Feet(3));
        assert_ne!(m.key().index, f.key().index);
        assert_eq!(m.get(ctx).0, 3);
        assert_eq!(f.get(ctx).0, 3);
    }
}
