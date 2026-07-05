//! Every SSA value, such as operation results or block arguments
//! has a type defined by the type system.
//!
//! The type system is open, with no fixed list of types,
//! and there are no restrictions on the abstractions they represent.
//!
//! See [MLIR Types](https://mlir.llvm.org/docs/DefiningDialects/TypesAndTypes/#types)
//!
//! The [pliron_type](pliron::derive::pliron_type) proc macro from [pliron-derive]
//! can be used to implement [Type] for a rust type.
//!
//! Common semantics, API and behaviour of [Type]s are
//! abstracted into interfaces. Interfaces in pliron capture MLIR
//! functionality of both [Traits](https://mlir.llvm.org/docs/Traits/)
//! and [Interfaces](https://mlir.llvm.org/docs/Interfaces/).
//! Interfaces must all implement an associated function named `verify` with
//! the type [TypeInterfaceVerifier].
//!
//! Interfaces are rust Trait definitions annotated with the attribute macro
//! [type_interface](pliron::derive::type_interface). The attribute ensures that any
//! verifiers of super-interfaces are run prior to the verifier of this interface.
//! Note: Super-interface verifiers *may* run multiple times for the same type.
//!
//! [Type]s that implement an interface must annotate the implementation with
//! [type_interface_impl](pliron::derive::type_interface_impl) macro to ensure that
//! the interface verifier is automatically called during verification
//! and that a `&dyn Type` object can be [cast](type_cast) into an interface object,
//! (or that it can be checked if the interface is [implemented](type_impls))
//! with ease.
//!
//! Use [verify_type] to verify a [Type] object.
//! This function verifies all interfaces implemented by the type, and then the type itself.
//! The type's verifier must explicitly invoke verifiers on any sub-objects it contains.
//!
//! [TypeHandle]s can be [TypeHandle::deref]'d and downcasted to their concrete types using
//! [downcast_rs](https://docs.rs/downcast-rs/latest/downcast_rs/#example-without-generics).

use crate::{
    arg_err_noloc,
    combine::{Parser, parser},
    common_traits::Verify,
    context::{Context, collect_deduped_interface_verifiers},
    dialect::{Dialect, DialectName},
    identifier::Identifier,
    impl_printable_for_display, input_err,
    irfmt::parsers::spaced,
    location::Located,
    parsable::{Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    result::Result,
    std_deps::{hash::FxHashMap, sync::LazyLock},
    storage_uniquer::TypeValueHash,
};

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    cell::{Ref, RefCell, RefMut},
    fmt::{Debug, Display},
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::Deref,
};
use downcast_rs::{Downcast, impl_downcast};
use pliron_derive::format;
use thiserror::Error;

/// Basic functionality that every type in the IR must implement.
/// Type objects (instances of a Type) are (mostly) immutable once created,
/// and are uniqued globally. Uniquing is based on the type name (i.e.,
/// the rust type being defined) and its contents.
///
/// So, for example, if we have
/// ```rust
///     # use pliron::derive::pliron_type;
///     #[pliron_type(
///         name = "test.intty",
///         format,
///         verifier = "succ"
///     )]
///     #[derive(Debug, PartialEq, Eq, Hash)]
///     struct IntType {
///         width: u64
///     }
/// ```
/// the uniquing will include
///   - [`core::any::TypeId::of::<IntType>()`](core::any::TypeId)
///   - `width`
///
/// Types *can* have mutable contents that can be modified *after*
/// the type is created. This enables creation of recursive types.
/// In such a case, it is up to the type definition to ensure that
///   1. It manually implements Hash, ignoring these mutable fields.
///   2. A proper distinguisher content (such as a string), that is part
///      of the hash, is used so that uniquing still works.
pub trait Type: Printable + Verify + Downcast + Sync + Send + Debug {
    /// Compute and get the hash for this instance of Self.
    /// Hash collisions can be a possibility.
    fn hash_type(&self) -> TypeValueHash;
    /// Is self equal to an other Type?
    fn eq_type(&self, other: &dyn Type) -> bool;

    /// Get a copyable handle to this type.
    // Unlike in [ArenaObj]s, we do not store a self handle inside the object itself
    // because that can upset taking automatic hashes of the object.
    fn get_self_handle(&self, ctx: &Context) -> TypeHandle {
        let is = |other: &TypeObj| self.eq_type(&**other.0.borrow());
        let idx = ctx
            .type_store
            .get(self.hash_type(), &is)
            .expect("Unregistered type object in existence");
        TypeHandle(idx)
    }

    /// Register an instance of a type in the provided [Context]
    /// Returns [TypeHandle] to self. If the type was already registered,
    /// the existing handle is returned.
    fn register_instance(t: Self, ctx: &Context) -> TypedHandle<Self>
    where
        Self: Sized,
    {
        let hash = t.hash_type();
        let idx = ctx.type_store.get_or_create_unique(
            TypeObj(RefCell::new(Box::new(t))),
            hash,
            &TypeObj::eq,
        );
        TypedHandle(TypeHandle(idx), PhantomData::<Self>)
    }

    /// If an instance of `t` already exists, get a [TypeHandle] to it.
    /// Consumes `t` either way.
    fn get_instance(t: Self, ctx: &Context) -> Option<TypedHandle<Self>>
    where
        Self: Sized,
    {
        let is = |other: &TypeObj| t.eq_type(&**other.0.borrow());
        ctx.type_store
            .get(t.hash_type(), &is)
            .map(|idx| TypedHandle(TypeHandle(idx), PhantomData::<Self>))
    }

    /// Get a Type's static name. This is *not* per instantiation of the type.
    /// It is mostly useful for printing and parsing the type.
    /// Uniquing does *not* use this, but instead uses [core::any::TypeId].
    fn get_type_id(&self) -> TypeId;

    /// Same as [get_type_id](Self::get_type_id), but without the self reference.
    fn get_type_id_static() -> TypeId
    where
        Self: Sized;

    #[doc(hidden)]
    /// Verify all interfaces implemented by this Type.
    fn verify_interfaces(&self, ctx: &Context) -> Result<()>;

    /// Register this Type's [TypeId] in the dialect it belongs to.
    fn register(ctx: &mut Context)
    where
        Self: Sized + Parsable<Arg = (), Parsed = TypedHandle<Self>>,
    {
        let ptr_parser: TypeParserFn = Box::new(|&()| {
            combine::parser(move |parsable_state: &mut StateStream<'_>| {
                Self::parse(parsable_state, ())
                    .map(|(typtr, r)| -> (TypeHandle, _) { (typtr.to_handle(), r) })
            })
            .boxed()
        });
        let typeid = Self::get_type_id_static();
        Dialect::register(ctx, &typeid.dialect.clone()).add_type(typeid, ptr_parser);
    }
}
impl_downcast!(Type);

/// A storable function pointer to parse a specific [Type].
/// The [Type]'s [Dialect] maps a [TypeId] to such a parser.
pub(crate) type TypeParserFn = Box<
    for<'a> fn(
        &'a (),
    )
        -> Box<dyn Parser<StateStream<'a>, Output = TypeHandle, PartialState = ()> + 'a>,
>;

/// Trait for IR entities that have a direct type.
pub trait Typed {
    /// Get the [Type] of the current entity.
    fn get_type(&self, ctx: &Context) -> TypeHandle;
}

impl Typed for TypeHandle {
    fn get_type(&self, _ctx: &Context) -> TypeHandle {
        *self
    }
}

impl Typed for dyn Type {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        self.get_self_handle(ctx)
    }
}

impl<T: Typed + ?Sized> Typed for &T {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        (*self).get_type(ctx)
    }
}

impl<T: Typed + ?Sized> Typed for &mut T {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        (**self).get_type(ctx)
    }
}

impl<T: Typed + ?Sized> Typed for Box<T> {
    fn get_type(&self, ctx: &Context) -> TypeHandle {
        (**self).get_type(ctx)
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
/// A Type's name (not including it's dialect).
pub struct TypeName(Identifier);

impl TypeName {
    /// Create a new TypeName.
    pub fn new(name: &str) -> TypeName {
        TypeName(name.try_into().expect("Invalid Identifier for TypeName"))
    }
}

impl Deref for TypeName {
    type Target = Identifier;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl_printable_for_display!(TypeName);

impl Display for TypeName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Parsable for TypeName {
    type Arg = ();
    type Parsed = TypeName;

    fn parse<'a>(
        state_stream: &mut crate::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>
    where
        Self: Sized,
    {
        Identifier::parser(())
            .map(|name| TypeName::new(&name))
            .parse_stream(state_stream)
            .into()
    }
}

/// A combination of a Type's name and its dialect.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct TypeId {
    pub dialect: DialectName,
    pub name: TypeName,
}

impl Parsable for TypeId {
    type Arg = ();
    type Parsed = TypeId;

    // Parses (but does not validate) a TypeId.
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>
    where
        Self: Sized,
    {
        let mut parser = DialectName::parser(())
            .skip(parser::char::char('.'))
            .and(TypeName::parser(()))
            .map(|(dialect, name)| TypeId { dialect, name });
        parser.parse_stream(state_stream).into()
    }
}

impl_printable_for_display!(TypeId);

impl Display for TypeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}", self.dialect, self.name)
    }
}

/// An instance of a [Type] stored in the [Context]'s type store.
pub(crate) struct TypeObj(RefCell<Box<dyn Type>>);

impl PartialEq for TypeObj {
    fn eq(&self, other: &Self) -> bool {
        self.0.borrow().eq_type(&**other.0.borrow())
    }
}

impl Eq for TypeObj {}

impl Hash for TypeObj {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&u64::from(self.0.borrow().hash_type()).to_ne_bytes())
    }
}

/// A handle to the uniqued [Type] objects stored in the [Context].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeHandle(usize);

impl TypeHandle {
    /// Get a reference to the underlying [Type] object.
    pub fn deref<'a>(&self, ctx: &'a Context) -> Ref<'a, dyn Type> {
        Ref::map(
            ctx.type_store.unique_store.get(self.0).unwrap().0.borrow(),
            |t| &**t,
        )
    }

    /// Get a mutable reference to the underlying [Type] object.
    /// This is useful when building recursive types, and the caller must ensure
    /// that the mutation does not change the hash-relevant contents of the type.
    pub fn deref_mut<'a>(&self, ctx: &'a Context) -> RefMut<'a, dyn Type> {
        RefMut::map(
            ctx.type_store
                .unique_store
                .get(self.0)
                .unwrap()
                .0
                .borrow_mut(),
            |t| &mut **t,
        )
    }
}

impl Printable for TypeHandle {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{} ", self.deref(ctx).get_type_id())?;
        Printable::fmt(&*self.deref(ctx), ctx, state, f)
    }
}

impl Verify for TypeHandle {
    fn verify(&self, ctx: &Context) -> Result<()> {
        verify_type(&*self.deref(ctx), ctx)
    }
}

impl Parsable for TypeHandle {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();
        let type_id_parser = spaced(TypeId::parser(()));

        let mut type_parser = type_id_parser.then(move |type_id: TypeId| {
            // This clone is to satify the borrow checker.
            let loc = loc.clone();
            combine::parser(move |parsable_state: &mut StateStream<'a>| {
                let state = &parsable_state.state;
                let dialect = state
                    .ctx
                    .dialects
                    .get(&type_id.dialect)
                    .expect("Dialect name parsed but dialect isn't registered");
                let Some(type_parser) = dialect.types.get(&type_id) else {
                    input_err!(loc.clone(), "Unregistered type {}", type_id.disp(state.ctx))?
                };
                type_parser(&()).parse_stream(parsable_state).into()
            })
        });

        type_parser.parse_stream(state_stream).into_result()
    }
}

/// Verify a [Type] object:
/// 1. All interfaces it implements are verified
/// 2. The type itself is verified.
pub fn verify_type(ty: &dyn Type, ctx: &Context) -> Result<()> {
    // Verify all interfaces implemented by this Type.
    ty.verify_interfaces(ctx)?;

    // Verify the type itself.
    Verify::verify(ty, ctx)
}

impl Verify for TypeObj {
    fn verify(&self, ctx: &Context) -> Result<()> {
        verify_type(self.0.borrow().as_ref(), ctx)
    }
}

/// A wrapper around [TypeHandle] with the underlying [Type] statically marked.
#[derive(Debug)]
pub struct TypedHandle<T: Type>(TypeHandle, PhantomData<T>);

#[derive(Error, Debug)]
#[error("TypedHandle mismatch: Constructing {expected} but provided {provided}")]
pub struct TypedHandleErr {
    pub expected: String,
    pub provided: String,
}

impl<T: Type> TypedHandle<T> {
    /// Return a [Ref] to the [Type]
    /// This borrows from a RefCell and the borrow is live
    /// as long as the returned [Ref] lives.
    pub fn deref<'a>(&self, ctx: &'a Context) -> Ref<'a, T> {
        Ref::map(self.0.deref(ctx), |t| {
            t.downcast_ref::<T>()
                .expect("Type mistmatch, inconsistent TypedHandle")
        })
    }

    /// Create a new [TypedHandle] from a [TypeHandle].
    pub fn from_handle(handle: TypeHandle, ctx: &Context) -> Result<TypedHandle<T>> {
        if handle.deref(ctx).is::<T>() {
            Ok(TypedHandle(handle, PhantomData::<T>))
        } else {
            arg_err_noloc!(TypedHandleErr {
                expected: T::get_type_id_static().disp(ctx).to_string(),
                provided: handle.disp(ctx).to_string()
            })
        }
    }

    /// Erase the static Rust type and return the underlying [TypeHandle].
    pub fn to_handle(&self) -> TypeHandle {
        self.0
    }
}

impl<T: Type> From<TypedHandle<T>> for TypeHandle {
    fn from(value: TypedHandle<T>) -> Self {
        value.to_handle()
    }
}

impl<T: Type> Clone for TypedHandle<T> {
    fn clone(&self) -> TypedHandle<T> {
        *self
    }
}

impl<T: Type> Copy for TypedHandle<T> {}

impl<T: Type> PartialEq for TypedHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Type> Eq for TypedHandle<T> {}

impl<T: Type> Hash for TypedHandle<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T: Type> Printable for TypedHandle<T> {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        Printable::fmt(&self.0, ctx, state, f)
    }
}

impl<T: Type + Parsable<Arg = (), Parsed = TypedHandle<T>>> Parsable for TypedHandle<T> {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();
        spaced(TypeId::parser(()))
            .then(move |type_id| {
                let loc = loc.clone();
                combine::parser(move |parsable_state: &mut StateStream<'a>| {
                    if type_id != T::get_type_id_static() {
                        input_err!(
                            loc.clone(),
                            "Expected type {}, but found {}",
                            T::get_type_id_static().disp(parsable_state.state.ctx),
                            type_id.disp(parsable_state.state.ctx)
                        )?
                    }
                    T::parser(arg).parse_stream(parsable_state).into()
                })
            })
            .parse_stream(state_stream)
            .into_result()
    }
}

impl<T: Type> Verify for TypedHandle<T> {
    fn verify(&self, ctx: &Context) -> Result<()> {
        self.0.deref(ctx).verify(ctx)
    }
}

/// Marker trait for type interface trait objects.
///
/// This is auto-implemented by the `#[type_interface]` macro for `dyn Interface`
/// objects and is used to restrict [type_cast] and [type_impls] to interface casts.
#[diagnostic::on_unimplemented(
    message = "`{Self}` not a type interface.",
    label = "If `{Self}` is a trait, annotate it with #[type_interface] to be able to cast to it from a `&dyn Type`",
    note = "If you want to cast to a concrete `Type`, use `downcast_ref` instead."
)]
pub trait TypeInterfaceMarker {}

/// Cast reference to a [Type] object to an interface reference.
///
/// Right usage: cast to an interface trait object.
/// ```
/// use pliron::builtin::type_interfaces::FunctionTypeInterface;
/// use pliron::r#type::{Type, type_cast};
///
/// fn right_cast(ty: &dyn Type) {
///     let _ = type_cast::<dyn FunctionTypeInterface>(ty);
/// }
/// ```
///
/// Casting to concrete [Type] types are intentionally rejected.
/// ```compile_fail
/// use pliron::builtin::types::IntegerType;
/// use pliron::r#type::{Type, type_cast};
///
/// fn wrong_cast(ty: &dyn Type) {
///     let _ = type_cast::<IntegerType>(ty);
/// }
/// ```
/// Use [downcast_rs](https://docs.rs/downcast-rs/latest/downcast_rs/#example-without-generics)
/// to cast to concrete [Type] types.
pub fn type_cast<T: ?Sized + TypeInterfaceMarker + 'static>(ty: &dyn Type) -> Option<&T> {
    crate::utils::trait_cast::any_to_trait::<T>(ty.as_any())
}

/// Does this [Type] object implement interface `T`?
///
/// Right usage: query using an interface trait object.
/// ```
/// use pliron::builtin::type_interfaces::FunctionTypeInterface;
/// use pliron::r#type::{Type, type_impls};
///
/// fn right_query(ty: &dyn Type) {
///     let _ = type_impls::<dyn FunctionTypeInterface>(ty);
/// }
/// ```
///
/// Querying with a concrete [Type] type is intentionally rejected.
/// ```compile_fail
/// use pliron::builtin::types::IntegerType;
/// use pliron::r#type::{Type, type_impls};
///
/// fn wrong_query(ty: &dyn Type) {
///     let _ = type_impls::<IntegerType>(ty);
/// }
/// ```
pub fn type_impls<T: ?Sized + TypeInterfaceMarker + 'static>(ty: &dyn Type) -> bool {
    type_cast::<T>(ty).is_some()
}

/// Every type interface must have a function named `verify` with this type.
pub type TypeInterfaceVerifier = fn(&dyn Type, &Context) -> Result<()>;
/// Function returns the list of super verifiers, followed by a self verifier, for an interface.
pub type TypeInterfaceAllVerifiers = fn() -> Vec<TypeInterfaceVerifier>;

#[doc(hidden)]
/// A [Type] paired with an interface it implements
/// (specifically the verifiers (including super verifiers) for that interface).
type TypeInterfaceVerifierInfo = (core::any::TypeId, TypeInterfaceAllVerifiers);

#[doc(hidden)]
#[cfg(not(target_family = "wasm"))]
pub mod statics {
    use super::*;

    #[::pliron::linkme::distributed_slice]
    pub static TYPE_INTERFACE_VERIFIERS: [TypeInterfaceVerifierInfo] = [..];

    pub(super) fn get_type_interface_verifiers()
    -> impl Iterator<Item = &'static TypeInterfaceVerifierInfo> {
        TYPE_INTERFACE_VERIFIERS.iter()
    }
}
#[doc(hidden)]
#[cfg(not(target_family = "wasm"))]
pub use statics::TYPE_INTERFACE_VERIFIERS;

#[doc(hidden)]
#[cfg(target_family = "wasm")]
pub mod statics {
    use super::*;
    use crate::InventoryWrapper;

    ::pliron::inventory::collect!(InventoryWrapper<TypeInterfaceVerifierInfo>);

    pub(super) fn get_type_interface_verifiers()
    -> impl Iterator<Item = &'static TypeInterfaceVerifierInfo> {
        ::pliron::inventory::iter::<InventoryWrapper<TypeInterfaceVerifierInfo>>().map(|llw| llw.0)
    }
}

#[doc(hidden)]
/// A map from every [Type] to its ordered (as per interface deps) list of interface verifiers.
/// An interface's super-interfaces are to be verified before it itself is.
pub static TYPE_INTERFACE_VERIFIERS_MAP: LazyLock<
    FxHashMap<core::any::TypeId, Vec<TypeInterfaceVerifier>>,
> = LazyLock::new(|| collect_deduped_interface_verifiers(statics::get_type_interface_verifiers()));

/// A convenient struct to hold a type signature.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[format("`(` vec($arguments, CharSpace(`,`)) `)` ` -> ` `(`vec($results, CharSpace(`,`)) `)`")]
pub struct TypeSig {
    pub arguments: Vec<TypeHandle>,
    pub results: Vec<TypeHandle>,
}
