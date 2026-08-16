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
//! 4. Similarly, [Source] internalizes `PathBuf`s in a [Context],
//!    thus requiring special handling for stable hashing, cloning into a different [Context].

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    any::Any,
    hash::{Hash, Hasher},
};

use crate::{
    attribute::{AttrObj, Attribute},
    context::Context,
    location::{Location, Source},
    parsable::{Parsable, parse_from_str},
    printable::Printable,
    r#type::{Type, TypeHandle, instantiate_boxed_type},
    uniqued_any,
    utils::trait_cast::{any_to_trait, any_to_trait_box},
};

/// A stable hash is deterministic across builds, platforms and [Context]s,
/// making it suitable for use as a cache key.
///
/// `impl`s must be marked with [type_to_trait](crate::type_to_trait).
///
/// Use `#[derive(StableHash)]` when every field already implements [StableHash].
///
/// [impl_stable_hash_for_hash](crate::impl_stable_hash_for_hash) can be used to
/// delegate to [Hash], when the type is not [Context] dependent.
pub trait StableHash {
    /// Compute a stable hash for [self].
    fn stable_hash(&self, ctx: &Context, state: &mut dyn Hasher);
}

/// Clone a value that lives in one [Context] into another.
///
/// `impl`s must be marked with [type_to_trait](crate::type_to_trait).
///
/// Use `#[derive(CloneIntoContext)]` when every field already implements [CloneIntoContext].
///
/// [impl_clone_into_context_for_clone](crate::impl_clone_into_context_for_clone) can be used
/// to delegate to [Clone], when the type is not [Context] dependent.
pub trait CloneIntoContext {
    /// Clone `self` from `src_ctx` into `dst_ctx`.
    /// The type of the value inside the returned [Box] must be `Self`.
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any>;
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
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let attr: &dyn Attribute = &**self;
        let inner: Box<dyn Attribute> = if let Some(cloner) =
            any_to_trait::<dyn CloneIntoContext>(attr.as_any())
        {
            let cloned_inner = cloner.clone_into_context(src_ctx, dst_ctx);
            any_to_trait_box::<dyn Attribute>(cloned_inner)
                .expect("Unable to cast Attribute instance to dyn Attribute")
        } else {
            // No registered impl: fall back to printing in `src_ctx` and
            // parsing back in `dst_ctx`.
            let printed = self.disp(src_ctx).to_string();
            parse_from_str(AttrObj::parser(()), dst_ctx, &printed).expect("Attribute failed parse")
        };
        Box::new(inner)
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
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let ty_handle: TypeHandle = if let Some(cloner) =
            any_to_trait::<dyn CloneIntoContext>(self.deref(src_ctx).as_any())
        {
            let cloned_inner = cloner.clone_into_context(src_ctx, dst_ctx);
            let inner = any_to_trait_box::<dyn Type>(cloned_inner)
                .expect("Unable to cast Type instance to dyn Type");
            instantiate_boxed_type(inner, dst_ctx)
        } else {
            // No registered impl: fall back to printing in `src_ctx` and
            // parsing back in `dst_ctx`.
            let printed = self.disp(src_ctx).to_string();
            parse_from_str(TypeHandle::parser(()), dst_ctx, &printed).expect("Type failed to parse")
        };
        Box::new(ty_handle)
    }
}

impl StableHash for Source {
    fn stable_hash(&self, ctx: &Context, mut state: &mut dyn Hasher) {
        core::mem::discriminant(self).hash(&mut state);
        if let Source::File(key) = self {
            // The key itself is just an index into `ctx`'s store; hash the
            // path it refers to instead.
            uniqued_any::get(ctx, *key).hash(&mut state);
        }
    }
}

impl CloneIntoContext for Source {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let cloned = match self {
            Source::File(key) => {
                let path = uniqued_any::get(src_ctx, *key).clone();
                Source::File(uniqued_any::save(dst_ctx, path))
            }
            Source::InMemory => Source::InMemory,
        };
        Box::new(cloned)
    }
}

impl StableHash for Location {
    fn stable_hash(&self, ctx: &Context, mut state: &mut dyn Hasher) {
        core::mem::discriminant(self).hash(&mut state);
        match self {
            Location::SrcPos { src, pos } => {
                src.stable_hash(ctx, state);
                pos.line.hash(&mut state);
                pos.column.hash(&mut state);
            }
            Location::Fused {
                metadata,
                locations,
            } => {
                match metadata {
                    Some(metadata) => metadata.stable_hash(ctx, state),
                    None => 0u8.hash(&mut state),
                }
                locations.len().hash(&mut state);
                for loc in locations {
                    loc.stable_hash(ctx, state);
                }
            }
            Location::Named { name, child_loc } => {
                name.hash(&mut state);
                child_loc.stable_hash(ctx, state);
            }
            Location::CallSite { callee, caller } => {
                callee.stable_hash(ctx, state);
                caller.stable_hash(ctx, state);
            }
            Location::Unknown => {}
        }
    }
}

/// Clone `v` into `dst_ctx` and downcast the result back to `T`.
///
/// A convenience wrapper around [CloneIntoContext::clone_into_context]
/// for callers that already know the concrete type they're cloning.
pub fn clone_into_typed<T: CloneIntoContext + 'static>(
    v: &T,
    src_ctx: &Context,
    dst_ctx: &mut Context,
) -> T {
    *v.clone_into_context(src_ctx, dst_ctx)
        .downcast::<T>()
        .expect("CloneIntoContext must box the same type it was called on")
}

impl CloneIntoContext for Location {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let cloned = match self {
            Location::SrcPos { src, pos } => Location::SrcPos {
                src: clone_into_typed(src, src_ctx, dst_ctx),
                pos: *pos,
            },
            Location::Fused {
                metadata,
                locations,
            } => Location::Fused {
                metadata: metadata
                    .as_ref()
                    .map(|metadata| clone_into_typed(metadata, src_ctx, dst_ctx)),
                locations: locations
                    .iter()
                    .map(|loc| clone_into_typed(loc, src_ctx, dst_ctx))
                    .collect(),
            },
            Location::Named { name, child_loc } => Location::Named {
                name: name.clone(),
                child_loc: clone_into_typed(child_loc, src_ctx, dst_ctx),
            },
            Location::CallSite { callee, caller } => Location::CallSite {
                callee: clone_into_typed(callee, src_ctx, dst_ctx),
                caller: clone_into_typed(caller, src_ctx, dst_ctx),
            },
            Location::Unknown => Location::Unknown,
        };
        Box::new(cloned)
    }
}

/// Implement [StableHash] for `$ty` by delegating to its own [Hash] impl.
///
/// The user guarantees that nothing in `$ty` depends on a [Context].
///
/// [type_to_trait](crate::type_to_trait) registration is not performed.
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
/// [type_to_trait](crate::type_to_trait) registration is not performed.
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
/// assert_eq!(*cloned.downcast::<Point>().unwrap(), p);
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
                ) -> $crate::alloc::boxed::Box<dyn ::core::any::Any> {
                    $crate::alloc::boxed::Box::new(::core::clone::Clone::clone(self))
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

impl<T: CloneIntoContext + 'static> CloneIntoContext for Option<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let cloned: Option<T> = self.as_ref().map(|v| clone_into_typed(v, src_ctx, dst_ctx));
        Box::new(cloned)
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

impl<T: CloneIntoContext + 'static> CloneIntoContext for Vec<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let cloned: Vec<T> = self
            .iter()
            .map(|v| clone_into_typed(v, src_ctx, dst_ctx))
            .collect();
        Box::new(cloned)
    }
}

impl<T: StableHash> StableHash for Box<T> {
    fn stable_hash(&self, ctx: &Context, state: &mut dyn Hasher) {
        (**self).stable_hash(ctx, state);
    }
}

impl<T: CloneIntoContext + 'static> CloneIntoContext for Box<T> {
    fn clone_into_context(&self, src_ctx: &Context, dst_ctx: &mut Context) -> Box<dyn Any> {
        let cloned: Box<T> = Box::new(clone_into_typed(self.as_ref(), src_ctx, dst_ctx));
        Box::new(cloned)
    }
}

#[cfg(test)]
mod tests {
    use pliron::derive::{CloneIntoContext, StableHash, pliron_attr, pliron_type};

    use super::*;
    use crate::{
        combine::stream::position::SourcePosition, context::Context, type_to_trait,
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
    #[derive(PartialEq, Eq, Clone, Debug, Hash)]
    struct TestAttr {
        val: u64,
    }
    impl CloneIntoContext for TestAttr {
        fn clone_into_context(&self, _src_ctx: &Context, _dst_ctx: &mut Context) -> Box<dyn Any> {
            Box::new(self.clone())
        }
    }
    type_to_trait!(TestAttr, CloneIntoContext);
    impl StableHash for TestAttr {
        fn stable_hash(&self, _ctx: &Context, mut state: &mut dyn Hasher) {
            self.val.hash(&mut state);
        }
    }
    type_to_trait!(TestAttr, StableHash);

    /// Same as [TestAttr], but with derived impls,
    /// to validate the derive macros end-to-end.
    #[pliron_attr(
        name = "test.decontext_derived_attr",
        format = "`<` $val `>`",
        verifier = "succ"
    )]
    #[derive(PartialEq, Eq, Clone, Debug, Hash, StableHash, CloneIntoContext)]
    struct TestDerivedAttr {
        val: u64,
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

        let cloned = a1.clone_into_context(&ctx, &mut dst_ctx);
        let a1_2 = *cloned.downcast::<AttrObj>().unwrap();
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
        let cloned = attr.clone_into_context(&ctx, &mut dst_ctx);
        let attr_2 = *cloned.downcast::<AttrObj>().unwrap();
        assert!(attr.disp(&ctx).to_string() == attr_2.disp(&dst_ctx).to_string());

        // Falls back to the print/parse path.
        let attr: AttrObj = Box::new(TestNoCloneIntoContextAttr);
        let cloned = attr.clone_into_context(&ctx, &mut dst_ctx);
        let attr_2 = *cloned.downcast::<AttrObj>().unwrap();
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
    #[derive(PartialEq, Eq, Clone, Debug, Hash)]
    struct TestType {
        val: u32,
    }
    impl CloneIntoContext for TestType {
        fn clone_into_context(&self, _src_ctx: &Context, _dst_ctx: &mut Context) -> Box<dyn Any> {
            Box::new(self.clone())
        }
    }
    type_to_trait!(TestType, CloneIntoContext);
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
        format = "`<` $val `>`",
        generate_get = true,
        verifier = "succ"
    )]
    #[derive(PartialEq, Eq, Clone, Debug, Hash, StableHash, CloneIntoContext)]
    struct TestDerivedType {
        val: u32,
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

        let cloned = t1.clone_into_context(&ctx, &mut dst_ctx);
        let t1_2 = *cloned.downcast::<TypeHandle>().unwrap();
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
        let cloned = ty.clone_into_context(&ctx, &mut dst_ctx);
        let ty_2 = *cloned.downcast::<TypeHandle>().unwrap();
        assert_eq!(ty.disp(&ctx).to_string(), ty_2.disp(&dst_ctx).to_string());

        // Falls back to the print/parse path.
        let unreg_ty = TestNoCloneIntoContextType::get(&ctx).to_handle();
        let cloned = unreg_ty.clone_into_context(&ctx, &mut dst_ctx);
        let unreg_ty_2 = *cloned.downcast::<TypeHandle>().unwrap();
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
        let cloned = file_src.clone_into_context(&ctx, &mut dst_ctx);
        let cloned_src = *cloned.downcast::<Source>().unwrap();
        assert_eq!(
            file_src.disp(&ctx).to_string(),
            cloned_src.disp(&dst_ctx).to_string()
        );

        let cloned = Source::InMemory.clone_into_context(&ctx, &mut dst_ctx);
        assert_eq!(*cloned.downcast::<Source>().unwrap(), Source::InMemory);
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
        let cloned = loc.clone_into_context(&ctx, &mut dst_ctx);
        let cloned_loc = *cloned.downcast::<Location>().unwrap();
        assert_eq!(
            loc.disp(&ctx).to_string(),
            cloned_loc.disp(&dst_ctx).to_string()
        );
    }
}
