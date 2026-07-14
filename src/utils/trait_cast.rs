//! Infrastructure for casting from `dyn Any` to `dyn Trait`,
//! for traits that the type contained by the [Any] object implements.
//!
//! A user must specify [type_to_trait](crate::type_to_trait) for a type that implements
//! a trait and needs to be casted to it, and then use [any_to_trait]
//! to do the actual cast. See their documentation for details and examples.

use core::any::{Any, TypeId};

use crate::std_deps::{hash::FxHashMap, sync::LazyLock};

/// Cast a [dyn Any](Any) object to a `dyn Trait` object for any
/// trait that the contained (in [Any]) type implements, and for which
/// [type_to_trait](crate::type_to_trait) has been specified.
///
/// To cast from `dyn Trait1` to `dyn Trait2` (when the underlying type implements both),
/// the user may use [downcast_rs] to easily upcast from `dyn Trait1` to [Any],
/// and then use [any_to_trait] to cast to `dyn Trait2`.
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
            ((**caster)
                // The caster function is set by `type_to_trait!`, and can only be of this type.
                .downcast_ref::<for<'a> fn(&'a (dyn Any + 'static)) -> &'a T>()
                .unwrap())(r)
        })
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

    pub fn get_trait_casters() -> impl Iterator<Item = &'static &'static TraitCasterInfo> {
        ::pliron::inventory::iter::<&'static TraitCasterInfo>()
    }
}

pub use statics::*;

#[doc(hidden)]
/// A map of all the trait casters, indexed by the type_id of the object
/// and the type_id of the trait to cast to. The map's values are
/// the cast function pointers. This is used to avoid having to search
/// through the distributed slice every time we want to cast an object.
static TRAIT_CASTERS_MAP: LazyLock<FxHashMap<(TypeId, TypeId), &'static (dyn Any + Sync + Send)>> =
    LazyLock::new(|| {
        get_trait_casters()
            .map(|lazy| ((lazy.from, lazy.to), lazy.caster))
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
                            &'a (dyn core::any::Any + 'static),
                        ) -> &'a (dyn $to_trait_name + 'static))
                        as &'static (dyn core::any::Any + Sync + Send),
                };

            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! {
                &CAST_TO_TRAIT
            }

            fn cast_to_trait<'a>(
                r: &'a (dyn core::any::Any + 'static),
            ) -> &'a (dyn $to_trait_name + 'static) {
                // This function is only called when the type of `r` is `$ty_name`,
                // so the downcast must succeed. A failure indicates an internal bug
                // in `type_to_trait!` or `any_to_trait`, not their usage.
                r.downcast_ref::<$ty_name>().unwrap() as &dyn $to_trait_name
            }
        };
    };
}
