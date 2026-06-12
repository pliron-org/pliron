//! Store, in [Context], a single unique copy of any object.

use core::{any::Any, cell::Ref, hash::Hash, marker::PhantomData};

use alloc::boxed::Box;

use crate::{
    context::{ArenaIndex, Context},
    storage_uniquer::TypeValueHash,
};

/// [Box]ed [Any], used for unique storage.
pub(crate) struct UniquedAny(Box<dyn Any>);

/// A handle to the stored unique copy of an object.
#[derive(PartialEq, Eq, Debug)]
pub struct UniquedKey<T> {
    index: ArenaIndex,
    _dummy: PhantomData<T>,
}

impl<T> Clone for UniquedKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for UniquedKey<T> {}

impl<T: 'static> Hash for UniquedKey<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        core::any::TypeId::of::<T>().hash(state);
    }
}

/// Save a unique copy of an object and get a handle to the saved copy.
pub fn save<T: Any + Hash + Eq>(ctx: &mut Context, t: T) -> UniquedKey<T> {
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
pub fn get<T: Any + Hash + Eq>(ctx: &Context, key: UniquedKey<T>) -> Ref<'_, T> {
    let r = ctx
        .uniqued_any_store
        .unique_store
        .get(key.index)
        .expect("Key not found in uniqued store")
        .borrow();
    Ref::map(r, |ua| {
        ua.0.downcast_ref::<T>()
            .expect("Type mismatch in UniquedAny retrieval")
    })
}

#[cfg(test)]
mod tests {
    use crate::context::Context;
    use alloc::string::String;

    use super::{get, save};

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
}
