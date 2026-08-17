// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! "De-contextualization" is separating an entity from the [Context] that owns it.
//! As a baseline, this should always be doable by [printing](Printable) it.
//!
//! Why? (some examples):
//! 1. A [Type] is handled through its [TypeHandle] which is essentially an index
//!    into an array in [Context]. The [Hash] of a [TypeHandle] is just the hash of this
//!    index. Such a hash is meaningful only within the [Context]. It cannot be used as
//!    a key for disk caching, for example.
//! 2. If we want to clone IR entities into another [Context] (say, for parallel processing),
//!    we cannot have the cloned IR contain [TypeHandle]s that reference the source [Context].
//! 3. The above examples extend to [Attribute]s too, for the simple reason that they
//!    may contain [TypeHandle]s.
//! 4. Similarly, [Source](crate::location::Source) internalizes `PathBuf`s in a [Context],
//!    thus requiring special handling for stable hashing, cloning into a different [Context].

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::hash::{Hash, Hasher};
use pliron_derive::{attr_interface, type_interface};

use crate::{
    attribute::{AttrObj, Attribute, attr_cast},
    context::Context,
    parsable::{Parsable, parse_from_str},
    printable::Printable,
    result::Result,
    r#type::{Type, TypeHandle, type_cast},
    utils::trait_cast::any_to_trait,
};

/// A stable hash is deterministic across builds, platforms and [Context]s,
/// making it suitable for use as a cache key.
///
/// `impl`s must be marked with [`type_to_trait!`](crate::type_to_trait).
///
/// Use [`#[derive(StableHash)]`](pliron::derive::StableHash) when every field
/// already implements [StableHash].
///
/// [`impl_stable_hash_for_hash!`](crate::impl_stable_hash_for_hash) can be used to
/// delegate to [Hash], when the type is not [Context] dependent.
pub trait StableHash {
    /// Compute a stable hash for [self].
    fn stable_hash(&self, ctx: &Context, state: &mut dyn Hasher);
}

/// Clone a value that lives in one [Context] into another.
///
/// Use [`#[derive(CloneIntoContext)]`](pliron::derive::StableHash) when every field
/// already implements [CloneIntoContext].
///
/// [`impl_clone_into_context_for_clone!`](crate::impl_clone_into_context_for_clone) can be used
/// to delegate to [Clone], when the type is not [Context] dependent.
pub trait CloneIntoContext {
    /// Clone `self` from `src_ctx` into `dst_ctx`.
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Self;
}

/// The [Attribute] interface version of [CloneIntoContext].
///
/// Typical usage would involve using [`#[derive(CloneIntoContext)]`](pliron::derive::CloneIntoContext)
/// or [`impl_clone_into_context_for_clone!`](pliron::impl_clone_into_context_for_clone) and then
/// [`#[derive(CloneAttributeIntoContext)]`](pliron::derive::CloneAttributeIntoContext).
#[attr_interface]
pub trait CloneAttributeIntoContext {
    /// Clone `self` from `src_ctx` into `dst_ctx`.
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> AttrObj;

    fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// The [Type] interface version of [CloneIntoContext].
///
/// Typical usage would involve using [`#[derive(CloneIntoContext)]`](pliron::derive::CloneIntoContext)
/// or [`impl_clone_into_context_for_clone!`](pliron::impl_clone_into_context_for_clone) and then
/// [`#[derive(CloneTypeIntoContext)]`](pliron::derive::CloneTypeIntoContext).
#[type_interface]
pub trait CloneTypeIntoContext {
    /// Clone `self` from `src_ctx` into `dst_ctx`.
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> TypeHandle;

    fn verify(_type: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

impl StableHash for AttrObj {
    fn stable_hash(&self, ctx: &Context, mut state: &mut dyn Hasher) {
        match any_to_trait::<dyn StableHash>(self.as_any()) {
            Some(h) => h.stable_hash(ctx, state),
            None => {
                // No registered impl: fall back to hashing the printed form.
                self.disp(ctx).to_string().hash(&mut state)
            }
        }
    }
}

impl CloneIntoContext for AttrObj {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> AttrObj {
        let attr: &dyn Attribute = &**self;
        if let Some(cloner) = attr_cast::<dyn CloneAttributeIntoContext>(attr) {
            cloner.clone_into_context(src_ctx, dst_ctx)
        } else {
            // No registered impl: fall back to printing in `src_ctx` and
            // parsing back in `dst_ctx`.
            let printed = self.disp(src_ctx).to_string();
            parse_from_str(AttrObj::parser(()), dst_ctx, &printed).expect("Attribute failed parse")
        }
    }
}

impl StableHash for TypeHandle {
    fn stable_hash(&self, ctx: &Context, mut state: &mut dyn Hasher) {
        match any_to_trait::<dyn StableHash>(self.deref(ctx).as_any()) {
            Some(h) => h.stable_hash(ctx, state),
            None => {
                // No registered impl: fall back to hashing the printed form.
                self.disp(ctx).to_string().hash(&mut state);
            }
        }
    }
}

impl CloneIntoContext for TypeHandle {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> TypeHandle {
        if let Some(cloner) = type_cast::<dyn CloneTypeIntoContext>(&*self.deref(src_ctx)) {
            cloner.clone_into_context(src_ctx, dst_ctx)
        } else {
            // No registered impl: fall back to printing in `src_ctx` and
            // parsing back in `dst_ctx`.
            let printed = self.disp(src_ctx).to_string();
            parse_from_str(TypeHandle::parser(()), dst_ctx, &printed).expect("Type failed to parse")
        }
    }
}

/// Implement [StableHash] for `$ty` by delegating to its own [Hash] impl.
///
/// The user guarantees that nothing in `$ty` depends on a [Context].
///
/// [`type_to_trait!`](crate::type_to_trait) registration is not performed.
///
/// Example:
/// ```
/// use pliron::{
///     context::Context, impl_stable_hash_for_hash, irbuild::decontext::StableHash,
///     utils::table::FxHasher,
/// };
///
/// #[derive(Hash)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
/// impl_stable_hash_for_hash!(Point);
///
/// let ctx = Context::new();
/// let mut state = FxHasher::default();
/// // No `Context`-bound state in `Point`, so this just forwards to `Hash::hash`.
/// Point { x: 1, y: 2 }.stable_hash(&ctx, &mut state);
/// ```
#[macro_export]
macro_rules! impl_stable_hash_for_hash {
    ($($ty:ty),* $(,)?) => {
        $(
            impl $crate::irbuild::decontext::StableHash for $ty {
                fn stable_hash(
                    &self,
                    _ctx: &$crate::context::Context,
                    mut state: &mut dyn ::core::hash::Hasher,
                ) {
                    // Hash the type's name first, so that two distinct types
                    // delegating to an identically-shaped `Hash` impl don't collide.
                    ::core::hash::Hash::hash(
                        ::core::concat!(::core::module_path!(), "::", ::core::stringify!($ty)),
                        &mut state,
                    );
                    ::core::hash::Hash::hash(self, &mut state);
                }
            }
        )*
    };
}

/// Implement [CloneIntoContext] for `$ty` by delegating to its own [Clone] impl.
///
/// The user guarantees that nothing in `$ty` depends on a [Context].
///
/// Example:
/// ```
/// use pliron::{
///     context::Context, impl_clone_into_context_for_clone,
///     irbuild::decontext::CloneIntoContext,
/// };
///
/// #[derive(Clone, Debug, PartialEq)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
/// impl_clone_into_context_for_clone!(Point);
///
/// let src_ctx = Context::new();
/// let mut dst_ctx = Context::new();
/// let p = Point { x: 1, y: 2 };
/// // No `Context`-bound state in `Point`, so this just forwards to `Clone::clone`.
/// let cloned = p.clone_into_context(&src_ctx, &mut dst_ctx);
/// assert_eq!(cloned, p);
/// ```
#[macro_export]
macro_rules! impl_clone_into_context_for_clone {
    ($($ty:ty),* $(,)?) => {
        $(
            impl $crate::irbuild::decontext::CloneIntoContext for $ty {
                fn clone_into_context(
                    &self,
                    _src_ctx: &$crate::context::Context,
                    _dst_ctx: &mut $crate::context::Context,
                ) -> $ty {
                    ::core::clone::Clone::clone(self)
                }
            }
        )*
    };
}

impl_stable_hash_for_hash!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, bool, char, String
);
impl_clone_into_context_for_clone!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, bool, char, String
);

impl<T: StableHash> StableHash for Option<T> {
    fn stable_hash(&self, ctx: &Context, mut state: &mut dyn Hasher) {
        match self {
            Some(v) => {
                1u8.hash(&mut state);
                v.stable_hash(ctx, state);
            }
            None => 0u8.hash(&mut state),
        }
    }
}

impl<T: CloneIntoContext> CloneIntoContext for Option<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Option<T> {
        self.as_ref()
            .map(|v| v.clone_into_context(src_ctx, dst_ctx))
    }
}

impl<T: StableHash> StableHash for Vec<T> {
    fn stable_hash(&self, ctx: &Context, mut state: &mut dyn Hasher) {
        self.len().hash(&mut state);
        for v in self {
            v.stable_hash(ctx, state);
        }
    }
}

impl<T: CloneIntoContext> CloneIntoContext for Vec<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Vec<T> {
        self.iter()
            .map(|v| v.clone_into_context(src_ctx, dst_ctx))
            .collect()
    }
}

impl<T: StableHash> StableHash for Box<T> {
    fn stable_hash(&self, ctx: &Context, state: &mut dyn Hasher) {
        (**self).stable_hash(ctx, state);
    }
}

impl<T: CloneIntoContext> CloneIntoContext for Box<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<T> {
        Box::new(self.as_ref().clone_into_context(src_ctx, dst_ctx))
    }
}

#[cfg(test)]
mod tests {
    use pliron::derive::{
        CloneAttributeIntoContext, CloneIntoContext, CloneTypeIntoContext, StableHash, pliron_attr,
        pliron_type,
    };

    use super::*;
    use crate::{
        combine::stream::position::SourcePosition,
        context::Context,
        location::{Location, Source},
        parsable::{ParseResult, StateStream},
        printable,
        r#type::TypedHandle,
        type_to_trait,
        utils::table::FxHasher,
    };

    fn stable_hash_of(ctx: &Context, v: &impl StableHash) -> u64 {
        let mut state = FxHasher::default();
        v.stable_hash(ctx, &mut state);
        state.finish()
    }

    /// An attribute that impls `CloneIntoContext`.
    #[pliron_attr(
        name = "test.decontext_attr",
        format = "`<` $val `>`",
        verifier = "succ"
    )]
    #[derive(PartialEq, Eq, Clone, Debug, Hash, CloneAttributeIntoContext)]
    struct TestAttr {
        val: u64,
    }
    impl_clone_into_context_for_clone!(TestAttr);
    impl StableHash for TestAttr {
        fn stable_hash(&self, _ctx: &Context, mut state: &mut dyn Hasher) {
            self.val.hash(&mut state);
        }
    }
    type_to_trait!(TestAttr, StableHash);

    /// Same as [TestAttr], but with derived impls,
    /// to validate the derive macros end-to-end.
    #[pliron_attr(name = "test.decontext_derived_attr", verifier = "succ")]
    #[derive(
        PartialEq, Eq, Clone, Debug, Hash, StableHash, CloneIntoContext, CloneAttributeIntoContext,
    )]
    struct TestDerivedAttr {
        val: u64,
    }

    impl Printable for TestDerivedAttr {
        fn fmt(
            &self,
            _ctx: &Context,
            _state: &printable::State,
            f: &mut core::fmt::Formatter<'_>,
        ) -> core::fmt::Result {
            write!(f, "<{}>", self.val)
        }
    }

    impl Parsable for TestDerivedAttr {
        type Arg = ();
        type Parsed = Self;

        fn parse<'a>(
            _state_stream: &mut StateStream<'a>,
            _arg: Self::Arg,
        ) -> ParseResult<'a, Self::Parsed>
        where
            Self: Sized,
        {
            unreachable!("TestDerivedAttr must never be parsed")
        }
    }

    #[test]
    fn attr_derived_impls() {
        let ctx = Context::new();
        let mut dst_ctx = Context::new();

        let a1: AttrObj = Box::new(TestDerivedAttr { val: 10 });
        let a2: AttrObj = Box::new(TestDerivedAttr { val: 10 });
        let a3: AttrObj = Box::new(TestDerivedAttr { val: 11 });
        assert_eq!(stable_hash_of(&ctx, &a1), stable_hash_of(&ctx, &a2));
        assert_ne!(stable_hash_of(&ctx, &a1), stable_hash_of(&ctx, &a3));

        let a1_2 = a1.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(a1.disp(&ctx).to_string(), a1_2.disp(&dst_ctx).to_string());
    }

    /// An attribute that doesn't impl `CloneIntoContext`.
    #[pliron_attr(name = "test.decontext_attr_unregistered", format, verifier = "succ")]
    #[derive(PartialEq, Eq, Clone, Debug, Hash)]
    struct TestNoCloneIntoContextAttr;

    #[test]
    fn attr_clone_into() {
        let ctx = Context::new();
        let mut dst_ctx = Context::new();

        let attr: AttrObj = Box::new(TestAttr { val: 10 });
        let attr_2 = attr.clone_into_context(&ctx, &mut dst_ctx);
        assert!(attr.disp(&ctx).to_string() == attr_2.disp(&dst_ctx).to_string());

        // Falls back to the print/parse path.
        let attr: AttrObj = Box::new(TestNoCloneIntoContextAttr);
        let attr_2 = attr.clone_into_context(&ctx, &mut dst_ctx);
        assert!(attr.disp(&ctx).to_string() == attr_2.disp(&dst_ctx).to_string());
    }

    #[test]
    fn attr_stable_hash() {
        let ctx = Context::new();

        let a1: AttrObj = Box::new(TestAttr { val: 10 });
        let a2: AttrObj = Box::new(TestAttr { val: 10 });
        let a3: AttrObj = Box::new(TestAttr { val: 11 });
        assert_eq!(stable_hash_of(&ctx, &a1), stable_hash_of(&ctx, &a2));
        assert_ne!(stable_hash_of(&ctx, &a1), stable_hash_of(&ctx, &a3));

        // Falls back to hashing the printed form.
        let u1: AttrObj = Box::new(TestNoCloneIntoContextAttr);
        let u2: AttrObj = Box::new(TestNoCloneIntoContextAttr);
        assert_eq!(stable_hash_of(&ctx, &u1), stable_hash_of(&ctx, &u2));
    }

    /// A type that impls `CloneIntoContext`.
    #[pliron_type(
        name = "test.decontext_type",
        format = "`<` $val `>`",
        generate_get = true,
        verifier = "succ"
    )]
    #[derive(PartialEq, Eq, Clone, Debug, Hash, CloneTypeIntoContext)]
    struct TestType {
        val: u32,
    }
    impl_clone_into_context_for_clone!(TestType);
    impl StableHash for TestType {
        fn stable_hash(&self, _ctx: &Context, mut state: &mut dyn Hasher) {
            self.val.hash(&mut state);
        }
    }
    type_to_trait!(TestType, StableHash);

    /// Same as [TestType], but with derived impls,
    /// to validate the derive macros end-to-end.
    #[pliron_type(
        name = "test.decontext_derived_type",
        generate_get = true,
        verifier = "succ"
    )]
    #[derive(
        PartialEq, Eq, Clone, Debug, Hash, StableHash, CloneIntoContext, CloneTypeIntoContext,
    )]
    struct TestDerivedType {
        val: u32,
    }

    impl Printable for TestDerivedType {
        fn fmt(
            &self,
            _ctx: &Context,
            _state: &printable::State,
            f: &mut core::fmt::Formatter<'_>,
        ) -> core::fmt::Result {
            write!(f, "<{}>", self.val)
        }
    }

    impl Parsable for TestDerivedType {
        type Arg = ();
        type Parsed = TypedHandle<Self>;

        fn parse<'a>(
            _state_stream: &mut StateStream<'a>,
            _arg: Self::Arg,
        ) -> ParseResult<'a, Self::Parsed>
        where
            Self: Sized,
        {
            unreachable!("TestDerivedType must never be parsed")
        }
    }

    #[test]
    fn type_derived_impls() {
        let ctx = Context::new();
        let mut dst_ctx = Context::new();

        let t1 = TestDerivedType::get(&ctx, 32).to_handle();
        let t2 = TestDerivedType::get(&ctx, 32).to_handle();
        let t3 = TestDerivedType::get(&ctx, 33).to_handle();
        assert_eq!(stable_hash_of(&ctx, &t1), stable_hash_of(&ctx, &t2));
        assert_ne!(stable_hash_of(&ctx, &t1), stable_hash_of(&ctx, &t3));

        let t1_2 = t1.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(t1.disp(&ctx).to_string(), t1_2.disp(&dst_ctx).to_string());
    }

    /// A type that doesn't impl `CloneIntoContext`.
    #[pliron_type(
        name = "test.decontext_type_unregistered",
        format,
        generate_get = true,
        verifier = "succ"
    )]
    #[derive(PartialEq, Eq, Clone, Debug, Hash)]
    struct TestNoCloneIntoContextType;

    #[test]
    fn type_clone_into() {
        let ctx = Context::new();
        let mut dst_ctx = Context::new();

        let ty = TestType::get(&ctx, 32).to_handle();
        let ty_2 = ty.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(ty.disp(&ctx).to_string(), ty_2.disp(&dst_ctx).to_string());

        // Falls back to the print/parse path.
        let unreg_ty = TestNoCloneIntoContextType::get(&ctx).to_handle();
        let unreg_ty_2 = unreg_ty.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(
            unreg_ty.disp(&ctx).to_string(),
            unreg_ty_2.disp(&dst_ctx).to_string()
        );
    }

    #[test]
    fn type_stable_hash() {
        let ctx = Context::new();

        let t1 = TestType::get(&ctx, 32).to_handle();
        let t3 = TestType::get(&ctx, 33).to_handle();
        assert_ne!(stable_hash_of(&ctx, &t1), stable_hash_of(&ctx, &t3));

        // Falls back to hashing the printed form.
        let u1 = TestNoCloneIntoContextType::get(&ctx).to_handle();
        let u2 = TestNoCloneIntoContextType::get(&ctx).to_handle();
        assert_eq!(stable_hash_of(&ctx, &u1), stable_hash_of(&ctx, &u2));
    }

    #[test]
    fn source_stable_hash() {
        let mut ctx = Context::new();

        let s1 = Source::new_from_file(&mut ctx, "foo.mlir");
        let s2 = Source::new_from_file(&mut ctx, "foo.mlir");
        let s3 = Source::new_from_file(&mut ctx, "bar.mlir");
        assert_eq!(stable_hash_of(&ctx, &s1), stable_hash_of(&ctx, &s2));
        assert_ne!(stable_hash_of(&ctx, &s1), stable_hash_of(&ctx, &s3));
        assert_ne!(
            stable_hash_of(&ctx, &s1),
            stable_hash_of(&ctx, &Source::InMemory)
        );
    }

    #[test]
    fn source_clone_into_context() {
        let mut ctx = Context::new();
        let mut dst_ctx = Context::new();

        let file_src = Source::new_from_file(&mut ctx, "foo.mlir");
        let cloned_src = file_src.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(
            file_src.disp(&ctx).to_string(),
            cloned_src.disp(&dst_ctx).to_string()
        );

        let cloned = Source::InMemory.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(cloned, Source::InMemory);
    }

    #[test]
    fn location_stable_hash() {
        let mut ctx = Context::new();
        let src = Source::new_from_file(&mut ctx, "foo.mlir");

        let loc1 = Location::SrcPos {
            src,
            pos: SourcePosition { line: 1, column: 2 },
        };
        let loc2 = Location::SrcPos {
            src,
            pos: SourcePosition { line: 1, column: 2 },
        };
        let loc3 = Location::SrcPos {
            src,
            pos: SourcePosition { line: 1, column: 3 },
        };
        assert_eq!(stable_hash_of(&ctx, &loc1), stable_hash_of(&ctx, &loc2));
        assert_ne!(stable_hash_of(&ctx, &loc1), stable_hash_of(&ctx, &loc3));

        let named1 = Location::Named {
            name: "foo".to_string(),
            child_loc: Box::new(Location::Unknown),
        };
        let named2 = Location::Named {
            name: "bar".to_string(),
            child_loc: Box::new(Location::Unknown),
        };
        assert_ne!(stable_hash_of(&ctx, &named1), stable_hash_of(&ctx, &named2));
        assert_ne!(
            stable_hash_of(&ctx, &named1),
            stable_hash_of(&ctx, &Location::Unknown)
        );
    }

    #[test]
    fn location_clone_into_context() {
        let mut ctx = Context::new();
        let mut dst_ctx = Context::new();
        let src = Source::new_from_file(&mut ctx, "foo.mlir");

        // A location nesting a `Source` (via `SrcPos`) inside a `Named`, to
        // exercise the recursive clone.
        let loc = Location::Named {
            name: "foo".to_string(),
            child_loc: Box::new(Location::SrcPos {
                src,
                pos: SourcePosition { line: 5, column: 6 },
            }),
        };
        let cloned_loc = loc.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(
            loc.disp(&ctx).to_string(),
            cloned_loc.disp(&dst_ctx).to_string()
        );
    }
}
