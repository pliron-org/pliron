// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Infrastructure for casting from `dyn Any` to `dyn Trait`,
//! for traits that the type contained by the [Any] object implements.
//!
//! A user must specify [type_to_trait](crate::type_to_trait) for a type that implements
//! a trait and needs to be casted to it, and then use [any_to_trait]
//! to do the actual cast. See their documentation for details and examples.

use core::any::{Any, TypeId};

use alloc::boxed::Box;

use crate::{std_deps::sync::LazyLock, utils::table::HMap};

#[doc(hidden)]
/// Input to a per-type trait-caster function.
/// Also see [TraitCastOutput].
pub enum AnyCastInput<'a> {
    Ref(&'a dyn Any),
    Owned(Box<dyn Any>),
}

#[doc(hidden)]
/// Output of a per-type trait-caster function.
/// Also see [AnyCastInput].
pub enum TraitCastOutput<'a, T: ?Sized> {
    Ref(&'a T),
    Owned(Box<T>),
}

/// Cast a [dyn Any](Any) object to a `dyn Trait` object for any
/// trait that the contained (in [Any]) type implements, and for which
/// [type_to_trait](crate::type_to_trait) has been specified.
///
/// To cast from `dyn Trait1` to `dyn Trait2` (when the underlying type implements both),
/// the user may use [downcast_rs] to easily upcast from `dyn Trait1` to [Any],
/// and then use [any_to_trait] to cast to `dyn Trait2`.
///
/// Example:
/// ```
/// # use pliron::{type_to_trait, utils::trait_cast::any_to_trait};
/// # use core::any::Any;
/// # use downcast_rs::Downcast;
///
/// trait Trait1: Downcast {}
/// trait Trait2 {}
/// trait Trait3<T> {}
///
/// struct S;
/// impl Trait1 for S {}
/// impl Trait2 for S {}
/// impl Trait3<u32> for S {}
///
/// type_to_trait!(S, Trait2);
/// type_to_trait!(S, Trait3<u32>);
///
/// let s1: &dyn Trait1 = &S;
/// any_to_trait::<dyn Trait2>(s1.as_any()).expect("Expected S to implement Trait2");
/// any_to_trait::<dyn Trait3<u32>>(s1.as_any()).expect("Expected S to implement Trait3<u32>");
/// assert!(any_to_trait::<dyn Trait3<f32>>(s1.as_any()).is_none(),
///     "S does not implement Trait3<f32>");
///
/// ```
pub fn any_to_trait<T: ?Sized + 'static>(r: &dyn Any) -> Option<&T> {
    TRAIT_CASTERS_MAP
        .get(&(r.type_id(), TypeId::of::<T>()))
        .map(|caster| {
            let caster = caster
                // The caster function is set by `type_to_trait!`, and can only be of this type.
                .downcast_ref::<for<'a> fn(AnyCastInput<'a>) -> TraitCastOutput<'a, T>>()
                .unwrap();
            match caster(AnyCastInput::Ref(r)) {
                TraitCastOutput::Ref(r) => r,
                // A `Ref` input to `cast_to_trait` always yields a `Ref` output.
                TraitCastOutput::Owned(_) => unreachable!(),
            }
        })
}

/// Cast a `Box<dyn Any>` object to a `Box<dyn Trait>` for any
/// trait that the contained (in [Any]) type implements, and for which
/// [type_to_trait](crate::type_to_trait) has been specified.
///
/// To cast from `Box<dyn Trait1>` to `Box<dyn Trait2>` (when the underlying type implements
/// both), the user may use [downcast_rs] to easily upcast from `Box<dyn Trait1>` to
/// `Box<dyn Any>`, and then use [any_to_trait_box] to cast to `Box<dyn Trait2>`.
///
/// Example:
/// ```
/// # use pliron::{type_to_trait, utils::trait_cast::any_to_trait_box};
/// # use pliron::alloc::boxed::Box;
/// # use core::any::Any;
///
/// trait Trait1 {}
/// trait Trait2 {}
///
/// struct S;
/// impl Trait1 for S {}
/// impl Trait2 for S {}
///
/// type_to_trait!(S, Trait2);
///
/// let s1: Box<dyn Any> = Box::new(S);
/// any_to_trait_box::<dyn Trait2>(s1).expect("Expected S to implement Trait2");
///
/// struct S2;
/// let s2: Box<dyn Any> = Box::new(S2);
/// assert!(any_to_trait_box::<dyn Trait2>(s2).is_none(), "S2 does not implement Trait2");
/// ```
pub fn any_to_trait_box<T: ?Sized + 'static>(r: Box<dyn Any>) -> Option<Box<T>> {
    let key = ((*r).type_id(), TypeId::of::<T>());
    TRAIT_CASTERS_MAP.get(&key).map(|caster| {
        let caster = caster
            // The caster function is set by `type_to_trait!`, and can only be of this type.
            .downcast_ref::<for<'a> fn(AnyCastInput<'a>) -> TraitCastOutput<'a, T>>()
            .unwrap();
        match caster(AnyCastInput::Owned(r)) {
            TraitCastOutput::Owned(b) => b,
            // An `Owned` input to `cast_to_trait` always yields an `Owned` output.
            TraitCastOutput::Ref(_) => unreachable!(),
        }
    })
}

/// Check if type `T` was registered to be casted to trait `I`
/// using [type_to_trait](crate::type_to_trait).
///
/// Example:
/// ```
/// # use pliron::{type_to_trait, utils::trait_cast::impls_trait_static};
/// # use core::any::Any;
/// trait Trait {}
/// struct S;
/// impl Trait for S {}
/// type_to_trait!(S, Trait);
/// assert!(impls_trait_static::<S, dyn Trait>());
/// struct S2;
/// assert!(!impls_trait_static::<S2, dyn Trait>());
/// ```
pub fn impls_trait_static<T: 'static, I: ?Sized + 'static>() -> bool {
    TRAIT_CASTERS_MAP.contains_key(&(TypeId::of::<T>(), TypeId::of::<I>()))
}

#[doc(hidden)]
/// Information to cast from a Rust type to a trait object.
pub struct TraitCasterInfo {
    /// The type from which we cast.
    pub from: TypeId,
    /// The trait to which we cast.
    pub to: TypeId,
    /// The cast function pointer.
    pub caster: &'static (dyn Any + Sync + Send),
}

#[cfg(not(target_family = "wasm"))]
pub mod statics {
    use super::*;

    #[::pliron::linkme::distributed_slice]
    pub static TRAIT_CASTERS: [TraitCasterInfo] = [..];

    pub fn get_trait_casters() -> impl Iterator<Item = &'static TraitCasterInfo> {
        TRAIT_CASTERS.iter()
    }
}

#[cfg(target_family = "wasm")]
pub mod statics {
    use super::*;

    ::pliron::inventory::collect!(&'static TraitCasterInfo);

    pub fn get_trait_casters() -> impl Iterator<Item = &'static TraitCasterInfo> {
        ::pliron::inventory::iter::<&'static TraitCasterInfo>().copied()
    }
}

pub use statics::*;

#[doc(hidden)]
/// A map of all the trait casters, indexed by the type_id of the object
/// and the type_id of the trait to cast to. The map's values hold the
/// cast function pointers.
static TRAIT_CASTERS_MAP: LazyLock<HMap<(TypeId, TypeId), &'static (dyn Any + Sync + Send)>> =
    LazyLock::new(|| {
        get_trait_casters()
            .map(|info| ((info.from, info.to), info.caster))
            .collect()
    });

/// Specify that a type may be casted to a `dyn Trait` object. Use [any_to_trait] for the actual cast.
/// Example:
/// ```
/// # use pliron::{type_to_trait, utils::trait_cast::any_to_trait};
/// # use core::any::Any;
/// trait Trait {}
/// struct S1;
/// impl Trait for S1 {}
/// type_to_trait!(S1, Trait);
///
/// let s1: &dyn Any = &S1;
/// any_to_trait::<dyn Trait>(s1).expect("Expected S1 to implement Trait");
///
/// struct S2;
/// let s2: &dyn Any = &S2;
/// assert!(
///     any_to_trait::<dyn Trait>(s2).is_none(),
///     "S2 does not implement Trait"
/// );
/// ```
#[macro_export]
macro_rules! type_to_trait {
    ($ty_name:ty, $to_trait_name:path) => {
        // The rust way to do an anonymous module.
        const _: () = {
            #[cfg_attr(
                not(target_family = "wasm"),
                ::pliron::linkme::distributed_slice
                    ($crate::utils::trait_cast::TRAIT_CASTERS), linkme(crate = ::pliron::linkme)
            )]
            static CAST_TO_TRAIT: $crate::utils::trait_cast::TraitCasterInfo =
                $crate::utils::trait_cast::TraitCasterInfo {
                    from: core::any::TypeId::of::<$ty_name>(),
                    to: core::any::TypeId::of::<dyn $to_trait_name>(),
                    caster: &(cast_to_trait
                        as for<'a> fn(
                            $crate::utils::trait_cast::AnyCastInput<'a>,
                        ) -> $crate::utils::trait_cast::TraitCastOutput<
                            'a,
                            dyn $to_trait_name + 'static,
                        >) as &'static (dyn core::any::Any + Sync + Send),
                };

            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! {
                &CAST_TO_TRAIT
            }

            fn cast_to_trait<'a>(
                r: $crate::utils::trait_cast::AnyCastInput<'a>,
            ) -> $crate::utils::trait_cast::TraitCastOutput<'a, dyn $to_trait_name + 'static>
            {
                // The downcasts below are only reached when the type contained in `r`
                // is `$ty_name`, so they must succeed. A failure indicates an internal
                // bug in `trait_cast`, not in how it's used.
                match r {
                    $crate::utils::trait_cast::AnyCastInput::Ref(r) => {
                        $crate::utils::trait_cast::TraitCastOutput::Ref(
                            r.downcast_ref::<$ty_name>().unwrap() as &dyn $to_trait_name,
                        )
                    }
                    $crate::utils::trait_cast::AnyCastInput::Owned(r) => {
                        $crate::utils::trait_cast::TraitCastOutput::Owned(r
                            .downcast::<$ty_name>()
                            .unwrap()
                            as $crate::alloc::boxed::Box<dyn $to_trait_name>)
                    }
                }
            }
        };
    };
}
