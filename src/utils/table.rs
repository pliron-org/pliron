// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Two Kinds of Hash Tables.
//!
//! * [itable] Provides [IMap] and [ISet]. They provide all standard hash table operations,
//!   with deterministic iteration.
//! * [htable] Provides [HMap] and [HSet]. They provide all standard hash table operations,
//!   except iteration.
//!
//! If iteration is not required, Use [htable] since it is faster and uses less memory.
//!
//! Currently [itable] is backed by [indexmap], and [htable] is backed by [hashbrown].
//! They both use [rustc_hash::FxBuildHasher], a fast, non-cryptographic hash function.

pub use htable::{HMap, HSet};
pub use itable::{IMap, ISet};

/// Hash table with deterministic iteration order.
pub mod itable {
    use rustc_hash::FxBuildHasher;

    /// A hash map with a fast, non-cryptographic hasher and deterministic iteration order.
    pub type IMap<K, V> = indexmap::IndexMap<K, V, FxBuildHasher>;

    /// A hash set with a fast, non-cryptographic hasher and deterministic iteration order.
    pub type ISet<T> = indexmap::IndexSet<T, FxBuildHasher>;

    /// A view into a single entry of an [IMap], obtained via [IMap::entry].
    pub use indexmap::map::Entry;
}

/// Hash table without iteration support.
pub mod htable {
    use core::{borrow::Borrow, fmt, hash::Hash};

    use rustc_hash::FxBuildHasher;

    /// A view into a single entry of an [HMap], obtained via [HMap::entry].
    pub struct Entry<'a, K, V>(hashbrown::hash_map::Entry<'a, K, V, FxBuildHasher>);

    impl<'a, K: Hash, V> Entry<'a, K, V> {
        /// Ensure a value is present, inserting `default` if it wasn't.
        pub fn or_insert(self, default: V) -> &'a mut V {
            self.0.or_insert(default)
        }

        /// Ensure a value is present, inserting the result of calling
        /// `default` if it wasn't.
        pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
            self.0.or_insert_with(default)
        }

        /// Ensure a value is present, inserting `V::default()` if it wasn't.
        pub fn or_default(self) -> &'a mut V
        where
            V: Default,
        {
            self.0.or_default()
        }

        /// Run `f` on the value if the entry is occupied, without changing
        /// whether it's occupied.
        pub fn and_modify<F: FnOnce(&mut V)>(self, f: F) -> Self {
            Entry(self.0.and_modify(f))
        }

        /// The key this entry refers to.
        pub fn key(&self) -> &K {
            self.0.key()
        }
    }

    /// A hash map with a fast, non-cryptographic hasher and no iteration support.
    #[derive(Clone)]
    pub struct HMap<K, V>(hashbrown::HashMap<K, V, FxBuildHasher>);

    impl<K, V> Default for HMap<K, V> {
        fn default() -> Self {
            HMap(hashbrown::HashMap::default())
        }
    }

    impl<K, V> HMap<K, V> {
        /// Create an empty map.
        pub fn new() -> Self {
            Self::default()
        }

        /// Create an empty map with at least the given capacity.
        pub fn with_capacity(capacity: usize) -> Self {
            HMap(hashbrown::HashMap::with_capacity_and_hasher(
                capacity,
                FxBuildHasher,
            ))
        }

        /// Number of entries in the map.
        pub fn len(&self) -> usize {
            self.0.len()
        }

        /// Is the map empty?
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        /// Remove all entries, keeping the allocated capacity.
        pub fn clear(&mut self) {
            self.0.clear()
        }

        /// Number of entries the map can hold without reallocating.
        pub fn capacity(&self) -> usize {
            self.0.capacity()
        }
    }

    impl<K: Eq + Hash, V> HMap<K, V> {
        /// Reserve capacity for at least `additional` more entries.
        pub fn reserve(&mut self, additional: usize) {
            self.0.reserve(additional)
        }

        /// Shrink the map's capacity as much as possible.
        pub fn shrink_to_fit(&mut self) {
            self.0.shrink_to_fit()
        }

        /// Insert `value` for `key`, returning the previous value if any.
        pub fn insert(&mut self, key: K, value: V) -> Option<V> {
            self.0.insert(key, value)
        }

        /// Look up the value for a key.
        pub fn get<Q>(&self, key: &Q) -> Option<&V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get(key)
        }

        /// Look up a mutable reference to the value for a key.
        pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get_mut(key)
        }

        /// Look up the stored key and value for a key.
        pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get_key_value(key)
        }

        /// Does the map contain `key`?
        pub fn contains_key<Q>(&self, key: &Q) -> bool
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.contains_key(key)
        }

        /// Remove and return the value for `key`, if present.
        pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.remove(key)
        }

        /// Remove and return the stored key and value for `key`, if present.
        pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(K, V)>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.remove_entry(key)
        }

        /// Get the entry for `key`, for in-place insert/update.
        pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
            Entry(self.0.entry(key))
        }

        /// Keep only the entries for which `f` returns `true`.
        ///
        /// `f` is run once per entry, in an unspecified, platform-dependent
        /// order; it must not rely on the order in which entries are
        /// visited.
        pub fn retain<F>(&mut self, f: F)
        where
            F: FnMut(&K, &mut V) -> bool,
        {
            self.0.retain(f)
        }
    }

    impl<K: Eq + Hash, V> Extend<(K, V)> for HMap<K, V> {
        fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
            self.0.extend(iter)
        }
    }

    impl<K: Eq + Hash, V> FromIterator<(K, V)> for HMap<K, V> {
        fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
            let mut map = HMap::default();
            map.extend(iter);
            map
        }
    }

    impl<K: Eq + Hash, V: PartialEq> PartialEq for HMap<K, V> {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }

    impl<K: Eq + Hash, V: Eq> Eq for HMap<K, V> {}

    impl<K, V> fmt::Debug for HMap<K, V> {
        /// Prints only the entry count: printing contents would expose the
        /// table's platform-dependent order.
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("HMap").field("len", &self.len()).finish()
        }
    }

    /// A hash set with a fast, non-cryptographic hasher and no iteration support.
    #[derive(Clone)]
    pub struct HSet<T>(hashbrown::HashSet<T, FxBuildHasher>);

    impl<T> Default for HSet<T> {
        fn default() -> Self {
            HSet(hashbrown::HashSet::default())
        }
    }

    impl<T> HSet<T> {
        /// Create an empty set.
        pub fn new() -> Self {
            Self::default()
        }

        /// Create an empty set with at least the given capacity.
        pub fn with_capacity(capacity: usize) -> Self {
            HSet(hashbrown::HashSet::with_capacity_and_hasher(
                capacity,
                FxBuildHasher,
            ))
        }

        /// Number of elements in the set.
        pub fn len(&self) -> usize {
            self.0.len()
        }

        /// Is the set empty?
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        /// Remove all elements, keeping the allocated capacity.
        pub fn clear(&mut self) {
            self.0.clear()
        }

        /// Number of elements the set can hold without reallocating.
        pub fn capacity(&self) -> usize {
            self.0.capacity()
        }
    }

    impl<T: Eq + Hash> HSet<T> {
        /// Reserve capacity for at least `additional` more elements.
        pub fn reserve(&mut self, additional: usize) {
            self.0.reserve(additional)
        }

        /// Shrink the set's capacity as much as possible.
        pub fn shrink_to_fit(&mut self) {
            self.0.shrink_to_fit()
        }

        /// Insert `value`, returning whether it was newly inserted.
        pub fn insert(&mut self, value: T) -> bool {
            self.0.insert(value)
        }

        /// Does the set contain `value`?
        pub fn contains<Q>(&self, value: &Q) -> bool
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.contains(value)
        }

        /// Get a reference to the stored value equal to `value`, if present.
        pub fn get<Q>(&self, value: &Q) -> Option<&T>
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get(value)
        }

        /// Remove `value`, returning whether it was present.
        pub fn remove<Q>(&mut self, value: &Q) -> bool
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.remove(value)
        }

        /// Remove and return the stored value equal to `value`, if present.
        pub fn take<Q>(&mut self, value: &Q) -> Option<T>
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.take(value)
        }

        /// Insert `value`, returning the previously-stored equal value if any.
        pub fn replace(&mut self, value: T) -> Option<T> {
            self.0.replace(value)
        }

        /// Keep only the elements for which `f` returns `true`.
        ///
        /// `f` is run once per element, in an unspecified,
        /// platform-dependent order; it must not rely on the order in
        /// which elements are visited.
        pub fn retain<F>(&mut self, f: F)
        where
            F: FnMut(&T) -> bool,
        {
            self.0.retain(f)
        }

        /// Are `self` and `other` disjoint (share no elements)?
        pub fn is_disjoint(&self, other: &Self) -> bool {
            self.0.is_disjoint(&other.0)
        }

        /// Is `self` a subset of `other`?
        pub fn is_subset(&self, other: &Self) -> bool {
            self.0.is_subset(&other.0)
        }

        /// Is `self` a superset of `other`?
        pub fn is_superset(&self, other: &Self) -> bool {
            self.0.is_superset(&other.0)
        }
    }

    impl<T: Eq + Hash> Extend<T> for HSet<T> {
        fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
            self.0.extend(iter)
        }
    }

    impl<T: Eq + Hash> FromIterator<T> for HSet<T> {
        fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
            let mut set = HSet::default();
            set.extend(iter);
            set
        }
    }

    impl<T: Eq + Hash> PartialEq for HSet<T> {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }

    impl<T: Eq + Hash> Eq for HSet<T> {}

    impl<T> fmt::Debug for HSet<T> {
        /// Prints only the element count: printing contents would expose
        /// the table's platform-dependent order.
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("HSet").field("len", &self.len()).finish()
        }
    }
}

#[cfg(test)]
mod tests {
    mod itable {
        use crate::utils::table::itable::{IMap, ISet};
        use alloc::{vec, vec::Vec};

        #[test]
        fn imap_preserves_insertion_order() {
            let mut m = IMap::default();
            m.insert("z", 1);
            m.insert("a", 2);
            m.insert("m", 3);
            let keys: Vec<_> = m.keys().copied().collect();
            assert_eq!(keys, vec!["z", "a", "m"]);
            let values: Vec<_> = m.values().copied().collect();
            assert_eq!(values, vec![1, 2, 3]);
        }

        #[test]
        fn imap_reinsertion_keeps_original_position() {
            let mut m = IMap::default();
            m.insert("z", 1);
            m.insert("a", 2);
            m.insert("z", 10);
            let keys: Vec<_> = m.keys().copied().collect();
            assert_eq!(keys, vec!["z", "a"]);
            assert_eq!(m["z"], 10);
        }

        #[test]
        fn imap_shift_remove_preserves_order_of_remainder() {
            let mut m: IMap<i32, &str> = IMap::default();
            for i in [3, 1, 4, 15, 9] {
                m.insert(i, "v");
            }
            m.shift_remove(&15);
            let keys: Vec<_> = m.keys().copied().collect();
            assert_eq!(keys, vec![3, 1, 4, 9]);
        }

        #[test]
        fn imap_get_contains_len() {
            let mut m = IMap::default();
            assert!(m.is_empty());
            m.insert(1, "one");
            m.insert(2, "two");
            assert_eq!(m.len(), 2);
            assert!(m.contains_key(&1));
            assert!(!m.contains_key(&3));
            assert_eq!(m.get(&2), Some(&"two"));
        }

        #[test]
        fn imap_entry_api() {
            let mut m: IMap<&str, i32> = IMap::default();
            *m.entry("a").or_insert(0) += 1;
            *m.entry("a").or_insert(0) += 1;
            *m.entry("b").or_insert(5) += 1;
            assert_eq!(m["a"], 2);
            assert_eq!(m["b"], 6);
        }

        #[test]
        fn imap_from_iter_and_extend() {
            let mut m: IMap<i32, i32> = [(1, 10), (2, 20)].into_iter().collect();
            assert_eq!(m.get(&1), Some(&10));
            m.extend([(3, 30)]);
            let keys: Vec<_> = m.keys().copied().collect();
            assert_eq!(keys, vec![1, 2, 3]);
        }

        #[test]
        fn iset_preserves_insertion_order() {
            let mut s = ISet::default();
            s.insert("z");
            s.insert("a");
            s.insert("m");
            s.insert("a"); // duplicate, no-op
            let items: Vec<_> = s.iter().copied().collect();
            assert_eq!(items, vec!["z", "a", "m"]);
        }

        #[test]
        fn iset_contains_len_remove() {
            let mut s: ISet<i32> = [1, 2, 3].into_iter().collect();
            assert_eq!(s.len(), 3);
            assert!(s.contains(&2));
            s.shift_remove(&2);
            assert!(!s.contains(&2));
            let items: Vec<_> = s.iter().copied().collect();
            assert_eq!(items, vec![1, 3]);
        }
    }

    mod htable {
        use crate::utils::table::htable::{HMap, HSet};

        #[test]
        fn hmap_insert_get_remove() {
            let mut m = HMap::default();
            assert!(m.is_empty());
            assert_eq!(m.insert("a", 1), None);
            assert_eq!(m.insert("a", 2), Some(1));
            assert_eq!(m.get("a"), Some(&2));
            assert_eq!(m.get("missing"), None);
            assert!(m.contains_key("a"));
            assert_eq!(m.len(), 1);
            assert_eq!(m.remove("a"), Some(2));
            assert!(m.is_empty());
            assert_eq!(m.remove("a"), None);
        }

        #[test]
        fn hmap_get_mut_and_key_value() {
            let mut m = HMap::default();
            m.insert("a", 1);
            if let Some(v) = m.get_mut("a") {
                *v += 41;
            }
            assert_eq!(m.get("a"), Some(&42));
            assert_eq!(m.get_key_value("a"), Some((&"a", &42)));
            assert_eq!(m.remove_entry("a"), Some(("a", 42)));
        }

        #[test]
        fn hmap_entry_api() {
            let mut m: HMap<&str, i32> = HMap::new();
            *m.entry("a").or_insert(0) += 1;
            *m.entry("a").or_insert(0) += 1;
            m.entry("b").or_insert_with(|| 10);
            m.entry("a").and_modify(|v| *v *= 100);
            assert_eq!(m.get("a"), Some(&200));
            assert_eq!(m.get("b"), Some(&10));

            let mut counts: HMap<&str, i32> = HMap::default();
            *counts.entry("x").or_default() += 1;
            assert_eq!(counts.get("x"), Some(&1));
        }

        #[test]
        fn hmap_clear_and_capacity() {
            let mut m = HMap::with_capacity(16);
            assert!(m.capacity() >= 16);
            m.insert(1, "a");
            m.insert(2, "b");
            assert_eq!(m.len(), 2);
            m.clear();
            assert!(m.is_empty());
            assert_eq!(m.len(), 0);
        }

        #[test]
        fn hmap_retain() {
            let mut m: HMap<i32, i32> = (0..10).map(|i| (i, i * i)).collect();
            m.retain(|k, _| k % 2 == 0);
            assert_eq!(m.len(), 5);
            for k in 0..10 {
                assert_eq!(m.contains_key(&k), k % 2 == 0);
            }
        }

        #[test]
        fn hmap_from_iter_extend_and_equality() {
            let mut m1: HMap<i32, &str> = [(1, "one"), (2, "two")].into_iter().collect();
            let mut m2 = HMap::default();
            m2.insert(1, "one");
            m2.insert(2, "two");
            assert_eq!(m1, m2);

            m1.extend([(3, "three")]);
            assert_ne!(m1, m2);
            m2.insert(3, "three");
            assert_eq!(m1, m2);
        }

        #[test]
        fn hmap_debug_does_not_panic_and_hides_contents() {
            let mut m = HMap::default();
            m.insert("secret-key", "secret-value");
            let dbg = alloc::format!("{m:?}");
            assert!(dbg.contains("len"));
            assert!(!dbg.contains("secret-key"));
            assert!(!dbg.contains("secret-value"));
        }

        #[test]
        fn hset_insert_contains_remove() {
            let mut s = HSet::default();
            assert!(s.is_empty());
            assert!(s.insert(1));
            assert!(!s.insert(1));
            assert!(s.contains(&1));
            assert_eq!(s.len(), 1);
            assert!(s.remove(&1));
            assert!(s.is_empty());
            assert!(!s.remove(&1));
        }

        #[test]
        fn hset_take_replace_get() {
            let mut s = HSet::default();
            s.insert(1);
            assert_eq!(s.get(&1), Some(&1));
            assert_eq!(s.replace(1), Some(1));
            assert_eq!(s.take(&1), Some(1));
            assert!(s.is_empty());
        }

        #[test]
        fn hset_retain() {
            let mut s: HSet<i32> = (0..10).collect();
            s.retain(|v| v % 2 == 0);
            assert_eq!(s.len(), 5);
            for v in 0..10 {
                assert_eq!(s.contains(&v), v % 2 == 0);
            }
        }

        #[test]
        fn hset_relational_ops() {
            let a: HSet<i32> = [1, 2, 3].into_iter().collect();
            let b: HSet<i32> = [2, 3, 4].into_iter().collect();
            let c: HSet<i32> = [1, 2].into_iter().collect();

            assert!(!a.is_disjoint(&b));
            assert!(a.is_disjoint(&HSet::from_iter([5, 6])));
            assert!(c.is_subset(&a));
            assert!(a.is_superset(&c));
            assert!(!a.is_subset(&c));
        }

        #[test]
        fn hset_from_iter_extend_and_equality() {
            let mut s1: HSet<i32> = [1, 2].into_iter().collect();
            let mut s2 = HSet::default();
            s2.insert(1);
            s2.insert(2);
            assert_eq!(s1, s2);

            s1.extend([3]);
            assert_ne!(s1, s2);
            s2.insert(3);
            assert_eq!(s1, s2);
        }

        #[test]
        fn hset_debug_does_not_panic_and_hides_contents() {
            let mut s = HSet::default();
            s.insert("secret-value");
            let dbg = alloc::format!("{s:?}");
            assert!(dbg.contains("len"));
            assert!(!dbg.contains("secret-value"));
        }

        #[test]
        fn hmap_clone_is_independent() {
            let mut m1 = HMap::default();
            m1.insert(1, "one");
            let mut m2 = m1.clone();
            m2.insert(2, "two");
            assert_eq!(m1.len(), 1);
            assert_eq!(m2.len(), 2);
        }

        #[test]
        fn hset_clone_is_independent() {
            let mut s1 = HSet::default();
            s1.insert(1);
            let mut s2 = s1.clone();
            s2.insert(2);
            assert_eq!(s1.len(), 1);
            assert_eq!(s2.len(), 2);
        }
    }
}
