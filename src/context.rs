//! [Context] and [Ptr] together provide memory management for `pliron`.

use core::{
    any::{Any, TypeId},
    cell::{Cell, Ref, RefCell, RefMut},
    fmt::Display,
    hash::Hash,
    marker::PhantomData,
};

use crate::{
    arg_error_noloc,
    basic_block::BasicBlock,
    common_traits::Verify,
    dialect::{Dialect, DialectName},
    identifier::Identifier,
    operation::Operation,
    printable::{self, Printable},
    region::Region,
    result::Result,
    std_deps::{
        hash::{FxHashMap, FxHashSet},
        sync::LazyLock,
    },
    storage_uniquer::UniqueStore,
    r#type::TypeObj,
    uniqued_any::UniquedAny,
    verify_err_noloc,
};
use alloc::{boxed::Box, format, string::ToString, vec, vec::Vec};
use slotmap::{SlotMap, new_key_type};

new_key_type! {
    /// The index type for the [SlotMap] used to store IR objects.
    pub struct ArenaIndex;
}

new_key_type! {
    /// The index type for the [SlotMap] used to store auxiliary data.
    pub struct AuxDataIndex;
}

impl Display for ArenaIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// An arena allocation pool for IR objects.
pub type Arena<T> = SlotMap<ArenaIndex, RefCell<T>>;

/// A context stores all IR data of this compilation session.
pub struct Context {
    /// A unique number for each `Value` in the context.
    /// Serves as a generation counter against accessing invalid `Value`s.
    pub(crate) value_counter: Cell<u64>,
    /// A unique number for each `Use` in the context.
    /// Serves as a generation counter against accessing invalid `Use`s.
    pub(crate) use_counter: Cell<u64>,
    /// Allocation pool for [Operation]s.
    pub(crate) operations: Arena<Operation>,
    /// Allocation pool for [BasicBlock]s.
    pub(crate) basic_blocks: Arena<BasicBlock>,
    /// Allocation pool for [Region]s.
    pub(crate) regions: Arena<Region>,
    /// Registered [Dialect]s.
    pub(crate) dialects: FxHashMap<DialectName, Dialect>,
    /// Storage for uniqued [TypeObj]s.
    pub(crate) type_store: UniqueStore<TypeObj>,
    /// Storage for other uniqued objects.
    pub(crate) uniqued_any_store: UniqueStore<UniquedAny>,
    /// Arbitrary data storage. Use [Self::aux_data_map] for dictionary access.
    pub aux_data: SlotMap<AuxDataIndex, Box<dyn Any>>,
    /// A dictionary with keys mapping to an index in [Self::aux_data].
    pub aux_data_map: FxHashMap<Identifier, AuxDataIndex>,
}

impl Context {
    pub fn new() -> Context {
        Self::default()
    }

    /// Is the IR in this context empty?
    /// An IR is considered empty if it has no operations, basic blocks, or regions.
    /// This does not check for types, dialects, ops, or aux_data stored in the context.
    pub fn is_ir_empty(&self) -> bool {
        self.operations.is_empty() && self.basic_blocks.is_empty() && self.regions.is_empty()
    }

    /// Get a unique number for a new value.
    pub(crate) fn get_new_value_uid(&self) -> u64 {
        let uid = self.value_counter.get();
        self.value_counter.set(uid + 1);
        uid
    }

    /// Get a unique number for a new use.
    pub(crate) fn get_new_use_uid(&self) -> u64 {
        let uid = self.use_counter.get();
        self.use_counter.set(uid + 1);
        uid
    }
}

impl Default for Context {
    fn default() -> Self {
        let mut ctx = Context {
            value_counter: Cell::new(0),
            use_counter: Cell::new(0),
            operations: Arena::default(),
            basic_blocks: Arena::default(),
            regions: Arena::default(),
            dialects: FxHashMap::default(),
            type_store: UniqueStore::default(),
            uniqued_any_store: UniqueStore::default(),
            aux_data: SlotMap::with_key(),
            aux_data_map: FxHashMap::default(),
        };

        // Verify that all dictionary keys are unique.
        if let Err(err) = &*DICT_KEYS_VERIFIER {
            panic!("{}", err.err);
        }

        // Run all context registrations
        for registration in get_context_registrations() {
            registration(&mut ctx);
        }

        ctx
    }
}

pub(crate) mod private {
    use super::*;

    /// An IR object owned by Context
    pub trait ArenaObj
    where
        Self: Sized,
    {
        /// Get the arena that has allocated this object.
        fn get_arena(ctx: &Context) -> &Arena<Self>;
        /// Get the arena that has allocated this object.
        fn get_arena_mut(ctx: &mut Context) -> &mut Arena<Self>;
        /// Get a Ptr to self.
        fn get_self_ptr(&self, ctx: &Context) -> Ptr<Self>;
        /// If this object contains any ArenaObj itself, it must dealloc()
        /// all of those sub-objects. This is called when self is deallocated.
        fn dealloc_sub_objects(ptr: Ptr<Self>, ctx: &mut Context);

        /// Allocates object on the arena, given a creator function.
        fn alloc<T: FnOnce(Ptr<Self>) -> Self>(ctx: &mut Context, f: T) -> Ptr<Self> {
            let creator = |idx: ArenaIndex| {
                let t = f(Ptr::<Self> {
                    idx,
                    _dummy: PhantomData::<Self>,
                });
                RefCell::new(t)
            };
            Ptr::<Self> {
                idx: Self::get_arena_mut(ctx).insert_with_key(creator),
                _dummy: PhantomData,
            }
        }

        /// Deallocates this object from the arena.
        fn dealloc(ptr: Ptr<Self>, ctx: &mut Context) {
            Self::dealloc_sub_objects(ptr, ctx);
            Self::get_arena_mut(ctx).remove(ptr.idx);
        }
    }
}

use private::ArenaObj;

/// Pointer to an IR Object owned by Context.
pub struct Ptr<T: ArenaObj> {
    pub(crate) idx: ArenaIndex,
    pub(crate) _dummy: PhantomData<T>,
}

impl<T: ArenaObj> core::fmt::Debug for Ptr<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Ptr<{}>[{}]", core::any::type_name::<T>(), self.idx)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Attempt to dereference a dangling Ptr")]
pub struct DanglingPtrDerefError;

impl<'a, T: ArenaObj> Ptr<T> {
    /// Borrow the inner [RefCell] and return a [Ref] to the pointee.
    /// The borrow is live as long as the returned [Ref] lives.
    /// Panics on dangling [Ptr] or borrow interior mutability errors.
    /// Run `cargo` with options `+nightly -Zbuild-std -Zbuild-std-features="debug_refcell"`
    /// to enable printing the interior mutability violation locations.
    #[track_caller]
    pub fn deref(&self, ctx: &'a Context) -> Ref<'a, T> {
        T::get_arena(ctx)
            .get(self.idx)
            .expect("Dangling Ptr deref")
            .borrow()
    }

    /// Mutably borrow the inner [RefCell] and return a [RefMut] to the pointee.
    /// The borrow is live as long as the returned [RefMut] lives.
    /// Panics on dangling [Ptr] or borrow interior mutability errors.
    /// Run `cargo` with options `+nightly -Zbuild-std -Zbuild-std-features="debug_refcell"`
    /// to enable printing the interior mutability violation locations.
    #[track_caller]
    pub fn deref_mut(&self, ctx: &'a Context) -> RefMut<'a, T> {
        T::get_arena(ctx)
            .get(self.idx)
            .expect("Dangling Ptr deref_mut")
            .borrow_mut()
    }

    /// Try and borrow the inner [RefCell] and return a [Ref] to the pointee.
    /// The borrow is live as long as the returned [Ref] lives.
    /// If [Ptr] is dangling or already mutably borrowed, an [Error](crate::result::Error)
    /// with [DanglingPtrDerefError] or [BorrowError](core::cell::BorrowError) is returned.
    pub fn try_deref(&self, ctx: &'a Context) -> Result<Ref<'a, T>> {
        T::get_arena(ctx)
            .get(self.idx)
            .ok_or_else(|| arg_error_noloc!(DanglingPtrDerefError))?
            .try_borrow()
            .map_err(|err| arg_error_noloc!(err))
    }

    /// Try and mutably borrow the inner [RefCell] and return a [RefMut] to the pointee.
    /// The borrow is live as long as the returned [RefMut] lives.
    /// If [Ptr] is dangling or already borrowed, an [Error](crate::result::Error)
    /// with [DanglingPtrDerefError] or [BorrowMutError](core::cell::BorrowMutError) is returned.
    pub fn try_deref_mut(&self, ctx: &'a Context) -> Result<RefMut<'a, T>> {
        T::get_arena(ctx)
            .get(self.idx)
            .ok_or_else(|| arg_error_noloc!(DanglingPtrDerefError))?
            .try_borrow_mut()
            .map_err(|err| arg_error_noloc!(err))
    }

    /// Create a unique (to the arena) name based on the arena index.
    pub(crate) fn make_name(&self, name_base: &str) -> Identifier {
        let idx = format!("{}", self.idx);
        (name_base.to_string() + &idx).try_into().unwrap()
    }
}

impl<T: ArenaObj> Clone for Ptr<T> {
    fn clone(&self) -> Ptr<T> {
        *self
    }
}

impl<T: ArenaObj> Copy for Ptr<T> {}

impl<T: ArenaObj> PartialEq for Ptr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

impl<T: ArenaObj> Eq for Ptr<T> {}

impl<T: ArenaObj + 'static> Hash for Ptr<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        TypeId::of::<T>().hash(state);
        self.idx.hash(state);
    }
}

impl<T: ArenaObj + Printable> Printable for Ptr<T> {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        self.deref(ctx).fmt(ctx, state, f)
    }
}

impl<T: ArenaObj + Verify> Verify for Ptr<T> {
    fn verify(&self, ctx: &Context) -> Result<()> {
        self.deref(ctx).verify(ctx)
    }
}

#[doc(hidden)]
/// Declaration of a static [Identifier] for use as a dictionary key.
#[derive(Eq, PartialEq, Debug, Clone)]
pub struct DictKeyId {
    /// The [Identifier] itself.
    pub id: Identifier,
    /// The file where this key was declared.
    pub file: &'static str,
    /// The line where this key was declared.
    pub line: u32,
    /// The column where this key was declared.
    pub column: u32,
}

/// These represent registrations that happen automatically at link time.
/// Every dialect, op, type, and attribute that are linked into your code
/// will register themselves using this type.
pub type ContextRegistration = fn(&mut Context);

#[doc(hidden)]
/// `pliron` uses dictionaries indexed by static [Identifier]s in many places,
/// such as [Context::aux_data_map], and the states of [Printable](crate::printable::State)
/// and [Parsable](crate::parsable::State). To avoid collisions in these [Identifier]s,
/// we use the [crate::dict_key!] macro to verify that all such keys declared using the macro
/// are unique. The macro adds the keys to this static slice, which is then verified
/// when a [Context] is created.
#[cfg(not(target_family = "wasm"))]
pub mod statics {
    use super::*;

    #[::pliron::linkme::distributed_slice]
    #[linkme(crate = ::pliron::linkme)]
    pub static DICT_KEY_IDS: [LazyLock<DictKeyId>];

    pub fn get_dict_key_ids() -> impl Iterator<Item = &'static LazyLock<DictKeyId>> {
        DICT_KEY_IDS.iter()
    }

    #[::pliron::linkme::distributed_slice]
    #[linkme(crate = ::pliron::linkme)]
    pub static CONTEXT_REGISTRATIONS: [LazyLock<ContextRegistration>];

    pub fn get_context_registrations()
    -> impl Iterator<Item = &'static LazyLock<ContextRegistration>> {
        CONTEXT_REGISTRATIONS.iter()
    }
}

#[cfg(target_family = "wasm")]
pub mod statics {
    use super::*;
    use crate::InventoryWrapper;

    ::pliron::inventory::collect!(InventoryWrapper<LazyLock<DictKeyId>>);

    pub fn get_dict_key_ids() -> impl Iterator<Item = &'static LazyLock<DictKeyId>> {
        ::pliron::inventory::iter::<InventoryWrapper<LazyLock<DictKeyId>>>().map(|llw| llw.0)
    }

    ::pliron::inventory::collect!(InventoryWrapper<LazyLock<ContextRegistration>>);

    pub fn get_context_registrations()
    -> impl Iterator<Item = &'static LazyLock<ContextRegistration>> {
        ::pliron::inventory::iter::<InventoryWrapper<LazyLock<ContextRegistration>>>()
            .map(|llw| llw.0)
    }
}

pub use statics::*;

#[doc(hidden)]
pub static DICT_KEYS_VERIFIER: LazyLock<Result<()>> = LazyLock::new(verify_dict_keys);

#[doc(hidden)]
/// Collect `(owner, __all_verifiers)` entries into an ordered verifier map.
///
/// Each owner (op/type/attribute) can contribute verifiers through multiple interfaces.
/// This helper preserves interface dependency order (as returned by each `__all_verifiers`
/// function) while deduplicating verifier function pointers.
pub(crate) fn collect_deduped_interface_verifiers<Id, AllVerifiers, Verifier>(
    interface_verifiers: impl Iterator<Item = &'static (Id, AllVerifiers)>,
) -> FxHashMap<Id, Vec<Verifier>>
where
    Id: Eq + Hash + Clone + 'static,
    AllVerifiers: Fn() -> Vec<Verifier> + Clone + 'static,
    Verifier: Eq + Hash + Clone,
{
    let mut grouped = FxHashMap::default();
    for entry in interface_verifiers {
        let (id, all_verifiers_for_interface) = entry;
        grouped
            .entry(id.clone())
            .and_modify(|verifiers: &mut Vec<AllVerifiers>| {
                verifiers.push(all_verifiers_for_interface.clone())
            })
            .or_insert(vec![all_verifiers_for_interface.clone()]);
    }

    // Remove duplicates (best effort as rustc may inline functions, resulting in different pointers).
    // Relies on `__all_verifiers` returning the super-verifiers followed by self verifier
    // to ensure that super-interfaces are verified first.
    grouped
        .into_iter()
        .map(|(id, verifiers)| {
            let mut dedupd_verifiers = Vec::new();
            let mut seen = FxHashSet::default();
            for verifier_fn_list in verifiers {
                for verifier in verifier_fn_list() {
                    if seen.insert(verifier.clone()) {
                        dedupd_verifiers.push(verifier);
                    }
                }
            }
            (id, dedupd_verifiers)
        })
        .collect()
}

#[doc(hidden)]
/// Verify that all dictionary keys are unique. This is called when a [Context] is created.
/// If any duplicate keys are found, a panic is raised with the file, line, and column
/// information of the duplicate keys.
pub fn verify_dict_keys() -> Result<()> {
    let mut seen: FxHashMap<Identifier, (&'static str, u32, u32)> = FxHashMap::default();
    for key in get_dict_key_ids() {
        if let Some((file, line, column)) = seen.get(&key.id) {
            return verify_err_noloc!(
                "Duplicate dictionary key \"{}\" declared in {}:{}:{} and {}:{}:{}",
                key.id,
                file,
                line,
                column,
                key.file,
                key.line,
                key.column
            );
        }
        seen.insert(key.id.clone(), (key.file, key.line, key.column));
    }
    Ok(())
}

/// A macro to declare a static [Identifier] for use as a dictionary key.
///
/// Usage:
/// ```
/// # use pliron::dict_key;
/// dict_key!(MY_KEY, "my_key");
/// let mut ctx = pliron::context::Context::new();
/// let aux_data_index = ctx.aux_data.insert(Box::new(42));
/// ctx.aux_data_map.insert(MY_KEY.clone(), aux_data_index);
/// assert_eq!(ctx.aux_data[aux_data_index].downcast_ref::<i32>(), Some(&42));
/// assert_eq!(ctx.aux_data_map[&*MY_KEY], aux_data_index);
/// ```
/// Here, `MY_KEY` is the name of the static variable, and `"my_key"` is the
/// string value of the [Identifier]. The macro will create a static variable
/// of type [`LazyLock<Identifier>`](LazyLock) with the name `MY_KEY`.
#[macro_export]
macro_rules! dict_key {
    (   $(#[$outer:meta])*
        $decl:ident, $name:expr
    ) => {
        // Create a static variable linked to the DICT_KEY_IDS slice
        // to ensure that all keys are unique.
        // The static variable is created in a separate anonymous module.
        const _: () = {
            #[cfg_attr(not(target_family = "wasm"),
                ::pliron::linkme::distributed_slice(::pliron::context::DICT_KEY_IDS), linkme(crate = ::pliron::linkme))]
            pub static $decl: $crate::std_deps::sync::LazyLock<::pliron::context::DictKeyId> =
                $crate::std_deps::sync::LazyLock::new(|| ::pliron::context::DictKeyId {
                    id: $name.try_into().unwrap(),
                    file: file!(),
                    line: line!(),
                    column: column!(),
                });

            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! {
                ::pliron::InventoryWrapper(&$decl)
            }
        };
        $(#[$outer])*
        // Create a static variable with the provided name to access the identifier.
        pub static $decl: $crate::std_deps::sync::LazyLock<::pliron::identifier::Identifier> =
            $crate::std_deps::sync::LazyLock::new(|| $name.try_into().unwrap());
    };
}

/// A macro to register a [ContextRegistration]. The argument function
/// will be called with a mutable reference to the [Context] when a [Context] is created.
/// Use this outside of any function (e.g. in the module scope).
///
/// Usage:
/// ```
/// use pliron::context_registration;
/// context_registration!(my_registration_fn);
/// fn my_registration_fn(_: &mut pliron::context::Context) {}
/// ```
/// Here, `my_registration_fn` is a function or closure matching [ContextRegistration].
#[macro_export]
macro_rules! context_registration {
    (   $(#[$outer:meta])*
        $registration:expr
    ) => {
        const _: () = {
            $(#[$outer])*
            #[cfg_attr(not(target_family = "wasm"),
                ::pliron::linkme::distributed_slice(::pliron::context::CONTEXT_REGISTRATIONS), linkme(crate = ::pliron::linkme))]
            static CONTEXT_REGISTRATION: $crate::std_deps::sync::LazyLock<::pliron::context::ContextRegistration> =
                $crate::std_deps::sync::LazyLock::new(|| $registration);

            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! {
                ::pliron::InventoryWrapper(&CONTEXT_REGISTRATION)
            }
        };
    };
}
