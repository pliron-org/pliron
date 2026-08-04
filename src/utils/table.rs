// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Hash Tables.
//!
//! * [itable] Provides [IMap] and [ISet]. They provide all standard hash table operations,
//!   with deterministic iteration.
//! * [htable] Provides [HMap] and [HSet]. They provide all standard hash table operations,
//!   except iteration.
//! * [smalltable] Provides [SmallMap] and [SmallSet], for tables that are usually small.
//!
//! All of these use [rustc_hash::FxBuildHasher], a fast, non-cryptographic hasher.
//!
//! If iteration is not required, use [htable] since it is faster and uses less memory.
//!
//! Currently [itable] is backed by [indexmap], and [htable] is backed by [hashbrown].

pub use htable::{HMap, HSet};
pub use itable::{IMap, ISet};
pub use rustc_hash::FxHasher;
pub use smalltable::{SmallMap, SmallSet};

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

    /// A view into a single entry in a map, which may either be vacant or
    /// occupied.
    ///
    /// This `enum` is constructed from the [entry](HMap::entry) method on
    /// [HMap].
    pub enum Entry<'a, K, V> {
        /// An occupied entry.
        Occupied(OccupiedEntry<'a, K, V>),
        /// A vacant entry.
        Vacant(VacantEntry<'a, K, V>),
    }

    impl<'a, K, V> From<hashbrown::hash_map::Entry<'a, K, V, FxBuildHasher>> for Entry<'a, K, V> {
        fn from(entry: hashbrown::hash_map::Entry<'a, K, V, FxBuildHasher>) -> Self {
            match entry {
                hashbrown::hash_map::Entry::Occupied(o) => Entry::Occupied(OccupiedEntry(o)),
                hashbrown::hash_map::Entry::Vacant(v) => Entry::Vacant(VacantEntry(v)),
            }
        }
    }

    impl<'a, K, V> From<Entry<'a, K, V>> for hashbrown::hash_map::Entry<'a, K, V, FxBuildHasher> {
        fn from(entry: Entry<'a, K, V>) -> Self {
            match entry {
                Entry::Occupied(o) => hashbrown::hash_map::Entry::Occupied(o.0),
                Entry::Vacant(v) => hashbrown::hash_map::Entry::Vacant(v.0),
            }
        }
    }

    impl<'a, K: Hash, V> Entry<'a, K, V> {
        /// Ensures a value is in the entry by inserting the default if
        /// empty, and returns a mutable reference to the value in the
        /// entry.
        pub fn or_insert(self, default: V) -> &'a mut V {
            hashbrown::hash_map::Entry::from(self).or_insert(default)
        }

        /// Ensures a value is in the entry by inserting the result of the
        /// default function if empty, and returns a mutable reference to
        /// the value in the entry.
        pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
            hashbrown::hash_map::Entry::from(self).or_insert_with(default)
        }

        /// Ensures a value is in the entry by inserting the default value
        /// if empty, and returns a mutable reference to the value in the
        /// entry.
        pub fn or_default(self) -> &'a mut V
        where
            V: Default,
        {
            hashbrown::hash_map::Entry::from(self).or_default()
        }

        /// Provides in-place mutable access to an occupied entry before
        /// any potential inserts into the map.
        pub fn and_modify<F: FnOnce(&mut V)>(self, f: F) -> Self {
            hashbrown::hash_map::Entry::from(self).and_modify(f).into()
        }

        /// Returns a reference to this entry's key.
        pub fn key(&self) -> &K {
            match self {
                Entry::Occupied(entry) => entry.key(),
                Entry::Vacant(entry) => entry.key(),
            }
        }
    }

    /// A view into an occupied entry in a [HMap].
    pub struct OccupiedEntry<'a, K, V>(hashbrown::hash_map::OccupiedEntry<'a, K, V, FxBuildHasher>);

    impl<'a, K, V> OccupiedEntry<'a, K, V> {
        /// Gets a reference to the key in the entry.
        pub fn key(&self) -> &K {
            self.0.key()
        }

        /// Gets a reference to the value in the entry.
        pub fn get(&self) -> &V {
            self.0.get()
        }

        /// Gets a mutable reference to the value in the entry.
        pub fn get_mut(&mut self) -> &mut V {
            self.0.get_mut()
        }

        /// Converts the `OccupiedEntry` into a mutable reference to the
        /// value in the entry with a lifetime bound to the map itself.
        pub fn into_mut(self) -> &'a mut V {
            self.0.into_mut()
        }

        /// Sets the value of the entry, and returns the entry's old value.
        pub fn insert(&mut self, value: V) -> V {
            self.0.insert(value)
        }

        /// Takes the value out of the entry, and returns it. Keeps the
        /// allocated memory for reuse.
        pub fn remove(self) -> V {
            self.0.remove()
        }

        /// Take the ownership of the key and value from the map. Keeps
        /// the allocated memory for reuse.
        pub fn remove_entry(self) -> (K, V) {
            self.0.remove_entry()
        }
    }

    /// A view into a vacant entry in a [HMap].
    pub struct VacantEntry<'a, K, V>(hashbrown::hash_map::VacantEntry<'a, K, V, FxBuildHasher>);

    impl<'a, K, V> VacantEntry<'a, K, V> {
        /// Gets a reference to the key that would be used when inserting a
        /// value through the `VacantEntry`.
        pub fn key(&self) -> &K {
            self.0.key()
        }

        /// Sets the value of the entry with the `VacantEntry`'s key, and
        /// returns a mutable reference to it.
        pub fn insert(self, value: V) -> &'a mut V
        where
            K: Hash,
        {
            self.0.insert(value)
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
        /// Creates an empty [HMap].
        ///
        /// The hash map is initially created with a capacity of 0, so it
        /// will not allocate until it is first inserted into.
        pub fn new() -> Self {
            Self::default()
        }

        /// Creates an empty [HMap] with the specified capacity.
        ///
        /// The hash map will be able to hold at least `capacity` elements
        /// without reallocating. If `capacity` is 0, the hash map will not
        /// allocate.
        pub fn with_capacity(capacity: usize) -> Self {
            HMap(hashbrown::HashMap::with_capacity_and_hasher(
                capacity,
                FxBuildHasher,
            ))
        }

        /// Returns the number of elements in the map.
        pub fn len(&self) -> usize {
            self.0.len()
        }

        /// Returns `true` if the map contains no elements.
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        /// Clears the map, removing all key-value pairs. Keeps the
        /// allocated memory for reuse.
        pub fn clear(&mut self) {
            self.0.clear()
        }

        /// Returns the number of elements the map can hold without
        /// reallocating.
        ///
        /// This number is a lower bound; the [HMap] might be able
        /// to hold more, but is guaranteed to be able to hold at least this
        /// many.
        pub fn capacity(&self) -> usize {
            self.0.capacity()
        }
    }

    impl<K: Eq + Hash, V> HMap<K, V> {
        /// Reserves capacity for at least `additional` more elements to be
        /// inserted in the [HMap]. The collection may reserve more space
        /// to avoid frequent reallocations.
        pub fn reserve(&mut self, additional: usize) {
            self.0.reserve(additional)
        }

        /// Shrinks the capacity of the map as much as possible. It will
        /// drop down as much as possible while maintaining the internal
        /// rules and possibly leaving some space in accordance with the
        /// resize policy.
        pub fn shrink_to_fit(&mut self) {
            self.0.shrink_to_fit()
        }

        /// Inserts a key-value pair into the map.
        ///
        /// If the map did not have this key present, [None] is returned.
        ///
        /// If the map did have this key present, the value is updated, and
        /// the old value is returned. The key is not updated, though; this
        /// matters for types that can be `==` without being identical.
        pub fn insert(&mut self, key: K, value: V) -> Option<V> {
            self.0.insert(key, value)
        }

        /// Returns a reference to the value corresponding to the key.
        ///
        /// The key may be any borrowed form of the map's key type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// key type.
        pub fn get<Q>(&self, key: &Q) -> Option<&V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get(key)
        }

        /// Returns a mutable reference to the value corresponding to the
        /// key.
        ///
        /// The key may be any borrowed form of the map's key type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// key type.
        pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get_mut(key)
        }

        /// Returns the key-value pair corresponding to the supplied key.
        ///
        /// The supplied key may be any borrowed form of the map's key type,
        /// but [Hash] and [Eq] on the borrowed form *must* match those for
        /// the key type.
        pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get_key_value(key)
        }

        /// Returns `true` if the map contains a value for the specified
        /// key.
        ///
        /// The key may be any borrowed form of the map's key type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// key type.
        pub fn contains_key<Q>(&self, key: &Q) -> bool
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.contains_key(key)
        }

        /// Removes a key from the map, returning the value at the key if
        /// the key was previously in the map. Keeps the allocated memory
        /// for reuse.
        ///
        /// The key may be any borrowed form of the map's key type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// key type.
        pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.remove(key)
        }

        /// Removes a key from the map, returning the stored key and value
        /// if the key was previously in the map. Keeps the allocated
        /// memory for reuse.
        ///
        /// The key may be any borrowed form of the map's key type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// key type.
        pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(K, V)>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.remove_entry(key)
        }

        /// Gets the given key's corresponding entry in the map for
        /// in-place manipulation.
        pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
            self.0.entry(key).into()
        }

        /// Retains only the elements specified by the predicate. Keeps the
        /// allocated memory for reuse.
        ///
        /// In other words, remove all pairs `(k, v)` such that `f(&k, &mut
        /// v)` returns `false`. The elements are visited in unsorted (and
        /// unspecified) order.
        pub fn retain<F>(&mut self, f: F)
        where
            F: FnMut(&K, &mut V) -> bool,
        {
            self.0.retain(f)
        }
    }

    impl<K: Eq + Hash, V, Q> core::ops::Index<&Q> for HMap<K, V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        type Output = V;

        /// Look up the value for a key, panicking if it isn't present.
        fn index(&self, key: &Q) -> &V {
            self.get(key).expect("no entry found for key")
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
        /// Creates an empty [HSet].
        ///
        /// The hash set is initially created with a capacity of 0, so it
        /// will not allocate until it is first inserted into.
        pub fn new() -> Self {
            Self::default()
        }

        /// Creates an empty [HSet] with the specified capacity.
        ///
        /// The hash set will be able to hold at least `capacity` elements
        /// without reallocating. If `capacity` is 0, the hash set will not
        /// allocate.
        pub fn with_capacity(capacity: usize) -> Self {
            HSet(hashbrown::HashSet::with_capacity_and_hasher(
                capacity,
                FxBuildHasher,
            ))
        }

        /// Returns the number of elements in the set.
        pub fn len(&self) -> usize {
            self.0.len()
        }

        /// Returns `true` if the set contains no elements.
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        /// Clears the set, removing all values.
        pub fn clear(&mut self) {
            self.0.clear()
        }

        /// Returns the number of elements the set can hold without
        /// reallocating.
        pub fn capacity(&self) -> usize {
            self.0.capacity()
        }
    }

    impl<T: Eq + Hash> HSet<T> {
        /// Reserves capacity for at least `additional` more elements to be
        /// inserted in the [HSet]. The collection may reserve more
        /// space to avoid frequent reallocations.
        pub fn reserve(&mut self, additional: usize) {
            self.0.reserve(additional)
        }

        /// Shrinks the capacity of the set as much as possible. It will
        /// drop down as much as possible while maintaining the internal
        /// rules and possibly leaving some space in accordance with the
        /// resize policy.
        pub fn shrink_to_fit(&mut self) {
            self.0.shrink_to_fit()
        }

        /// Adds a value to the set.
        ///
        /// If the set did not have this value present, `true` is returned.
        ///
        /// If the set did have this value present, `false` is returned.
        pub fn insert(&mut self, value: T) -> bool {
            self.0.insert(value)
        }

        /// Returns `true` if the set contains a value.
        ///
        /// The value may be any borrowed form of the set's value type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// value type.
        pub fn contains<Q>(&self, value: &Q) -> bool
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.contains(value)
        }

        /// Returns a reference to the value in the set, if any, that is
        /// equal to the given value.
        ///
        /// The value may be any borrowed form of the set's value type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// value type.
        pub fn get<Q>(&self, value: &Q) -> Option<&T>
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.get(value)
        }

        /// Removes a value from the set. Returns whether the value was
        /// present in the set.
        ///
        /// The value may be any borrowed form of the set's value type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// value type.
        pub fn remove<Q>(&mut self, value: &Q) -> bool
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.remove(value)
        }

        /// Removes and returns the value in the set, if any, that is equal
        /// to the given one.
        ///
        /// The value may be any borrowed form of the set's value type, but
        /// [Hash] and [Eq] on the borrowed form *must* match those for the
        /// value type.
        pub fn take<Q>(&mut self, value: &Q) -> Option<T>
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.take(value)
        }

        /// Adds a value to the set, replacing the existing value, if any,
        /// that is equal to the given one. Returns the replaced value.
        pub fn replace(&mut self, value: T) -> Option<T> {
            self.0.replace(value)
        }

        /// Retains only the elements specified by the predicate.
        ///
        /// In other words, remove all elements `e` such that `f(&e)`
        /// returns `false`.
        pub fn retain<F>(&mut self, f: F)
        where
            F: FnMut(&T) -> bool,
        {
            self.0.retain(f)
        }

        /// Returns `true` if `self` has no elements in common with
        /// `other`. This is equivalent to checking for an empty
        /// intersection.
        pub fn is_disjoint(&self, other: &Self) -> bool {
            self.0.is_disjoint(&other.0)
        }

        /// Returns `true` if the set is a subset of another, i.e., `other`
        /// contains at least all the values in `self`.
        pub fn is_subset(&self, other: &Self) -> bool {
            self.0.is_subset(&other.0)
        }

        /// Returns `true` if the set is a superset of another, i.e.,
        /// `self` contains at least all the values in `other`.
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

/// Hash table optimized for the common case of very few entries.
pub mod smalltable {
    use alloc::boxed::Box;
    use core::{borrow::Borrow, fmt, hash::Hash, mem};

    use rustc_hash::FxBuildHasher;
    use smallvec::SmallVec;

    use super::itable::{IMap, ISet};

    /// Backing storage for a [SmallMap]: either up to `N` entries inline in
    /// a flat, linearly-scanned array, or (once grown past `N` entries) a
    /// heap-allocated [IMap].
    enum MapRepr<K, V, const N: usize> {
        Inline(SmallVec<[(K, V); N]>),
        Spilled(Box<IMap<K, V>>),
    }

    /// A hash map, with a fast non-cryptographic hasher and deterministic
    /// iteration order, optimized for the common case of very few entries.
    ///
    /// Up to `N` entries are stored inline, in a flat, linearly-scanned
    /// array, with no heap allocation and no hashing. Once an insert
    /// grows the map past `N` entries, it transparently and permanently
    /// promotes itself to an [IMap]
    pub struct SmallMap<K, V, const N: usize>(MapRepr<K, V, N>);

    impl<K, V, const N: usize> Default for SmallMap<K, V, N> {
        fn default() -> Self {
            SmallMap(MapRepr::Inline(SmallVec::new()))
        }
    }

    impl<K, V, const N: usize> SmallMap<K, V, N> {
        /// Creates an empty [SmallMap]. Does not allocate until it grows
        /// past `N` entries.
        pub fn new() -> Self {
            Self::default()
        }

        /// Returns `true` if this map has not (yet) grown past `N`
        /// entries, i.e., is still using inline, non-heap-allocated
        /// storage.
        pub fn is_inline(&self) -> bool {
            matches!(self.0, MapRepr::Inline(_))
        }

        /// Returns the number of elements in the map.
        pub fn len(&self) -> usize {
            match &self.0 {
                MapRepr::Inline(v) => v.len(),
                MapRepr::Spilled(m) => m.len(),
            }
        }

        /// Returns `true` if the map contains no elements.
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// Clears the map, removing all key-value pairs. A previously
        /// promoted map stays promoted; see the [SmallMap] documentation.
        pub fn clear(&mut self) {
            match &mut self.0 {
                MapRepr::Inline(v) => v.clear(),
                MapRepr::Spilled(m) => m.clear(),
            }
        }

        /// Retains only the elements specified by the predicate.
        ///
        /// In other words, remove all pairs `(k, v)` such that `f(&k, &mut
        /// v)` returns `false`.
        pub fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, mut f: F) {
            match &mut self.0 {
                MapRepr::Inline(v) => v.retain(|(k, v)| f(k, v)),
                MapRepr::Spilled(m) => m.retain(f),
            }
        }

        /// Returns an iterator over the map's key-value pairs.
        pub fn iter(&self) -> MapIter<'_, K, V> {
            match &self.0 {
                MapRepr::Inline(v) => MapIter::Inline(v.iter()),
                MapRepr::Spilled(map) => MapIter::Spilled(map.iter()),
            }
        }

        /// Returns a mutable iterator over the map's key-value pairs.
        pub fn iter_mut(&mut self) -> MapIterMut<'_, K, V> {
            match &mut self.0 {
                MapRepr::Inline(v) => MapIterMut::Inline(v.iter_mut()),
                MapRepr::Spilled(map) => MapIterMut::Spilled(map.iter_mut()),
            }
        }

        /// Returns an iterator over the map's keys.
        pub fn keys(&self) -> impl Iterator<Item = &K> + '_ {
            self.iter().map(|(k, _)| k)
        }

        /// Returns an iterator over the map's values.
        pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
            self.iter().map(|(_, v)| v)
        }

        /// Returns a mutable iterator over the map's values.
        pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> + '_ {
            self.iter_mut().map(|(_, v)| v)
        }
    }

    impl<K: Hash + Eq, V, const N: usize> SmallMap<K, V, N> {
        /// Moves from inline storage to a heap-allocated [IMap].
        fn promote(inline: &mut SmallVec<[(K, V); N]>) -> Box<IMap<K, V>> {
            let mut map = IMap::with_capacity_and_hasher(inline.len() + 1, FxBuildHasher);
            for (k, v) in inline.drain(..) {
                map.insert(k, v);
            }
            Box::new(map)
        }

        /// Inserts a key-value pair into the map.
        ///
        /// If the map did not have this key present, [None] is returned.
        /// If the map did have this key present, the value is updated,
        /// and the old value is returned. The key is not updated, though;
        /// this matters for types that can be `==` without being
        /// identical.
        pub fn insert(&mut self, key: K, value: V) -> Option<V> {
            match &mut self.0 {
                MapRepr::Inline(v) => {
                    if let Some(i) = v.iter().position(|(k, _)| *k == key) {
                        return Some(mem::replace(&mut v[i].1, value));
                    }
                    if v.len() < N {
                        v.push((key, value));
                        None
                    } else {
                        let mut map = Self::promote(v);
                        let old = map.insert(key, value);
                        self.0 = MapRepr::Spilled(map);
                        old
                    }
                }
                MapRepr::Spilled(map) => map.insert(key, value),
            }
        }

        /// Returns a reference to the value corresponding to the key.
        pub fn get<Q>(&self, key: &Q) -> Option<&V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &self.0 {
                MapRepr::Inline(v) => v.iter().find(|(k, _)| k.borrow() == key).map(|(_, v)| v),
                MapRepr::Spilled(map) => map.get(key),
            }
        }

        /// Returns a mutable reference to the value corresponding to the
        /// key.
        pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &mut self.0 {
                MapRepr::Inline(v) => {
                    let i = v.iter().position(|(k, _)| k.borrow() == key)?;
                    Some(&mut v[i].1)
                }
                MapRepr::Spilled(map) => map.get_mut(key),
            }
        }

        /// Returns the key-value pair corresponding to the supplied key.
        pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &self.0 {
                MapRepr::Inline(v) => v
                    .iter()
                    .find(|(k, _)| k.borrow() == key)
                    .map(|(k, v)| (k, v)),
                MapRepr::Spilled(map) => map.get_key_value(key),
            }
        }

        /// Returns `true` if the map contains a value for the specified
        /// key.
        pub fn contains_key<Q>(&self, key: &Q) -> bool
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.get(key).is_some()
        }

        /// Removes a key from the map, returning the value at the key if
        /// the key was previously in the map.
        pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
        where
            K: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &mut self.0 {
                MapRepr::Inline(v) => v
                    .iter()
                    .position(|(k, _)| k.borrow() == key)
                    .map(|i| v.swap_remove(i).1),
                MapRepr::Spilled(map) => map.swap_remove(key),
            }
        }

        /// Gets the given key's corresponding entry in the map for
        /// in-place manipulation.
        pub fn entry(&mut self, key: K) -> Entry<'_, K, V, N> {
            if let MapRepr::Inline(v) = &self.0 {
                if let Some(index) = v.iter().position(|(k, _)| *k == key) {
                    let MapRepr::Inline(v) = &mut self.0 else {
                        unreachable!()
                    };
                    return Entry::Occupied(OccupiedEntry(OccupiedEntryRepr::Inline {
                        vec: v,
                        index,
                    }));
                }
                return Entry::Vacant(VacantEntry(VacantEntryRepr::Inline {
                    repr: &mut self.0,
                    key,
                }));
            }
            let MapRepr::Spilled(map) = &mut self.0 else {
                unreachable!()
            };
            match map.entry(key) {
                indexmap::map::Entry::Occupied(e) => {
                    Entry::Occupied(OccupiedEntry(OccupiedEntryRepr::Spilled(e)))
                }
                indexmap::map::Entry::Vacant(e) => {
                    Entry::Vacant(VacantEntry(VacantEntryRepr::Spilled(e)))
                }
            }
        }
    }

    /// A view into a single entry in a [SmallMap], which may either be
    /// vacant or occupied. Obtained via [SmallMap::entry].
    pub enum Entry<'a, K, V, const N: usize> {
        /// An occupied entry.
        Occupied(OccupiedEntry<'a, K, V, N>),
        /// A vacant entry.
        Vacant(VacantEntry<'a, K, V, N>),
    }

    impl<'a, K, V, const N: usize> Entry<'a, K, V, N> {
        /// Provides in-place mutable access to an occupied entry before
        /// any potential inserts into the map.
        pub fn and_modify<F: FnOnce(&mut V)>(self, f: F) -> Self {
            match self {
                Entry::Occupied(mut e) => {
                    f(e.get_mut());
                    Entry::Occupied(e)
                }
                Entry::Vacant(e) => Entry::Vacant(e),
            }
        }

        /// Returns a reference to this entry's key.
        pub fn key(&self) -> &K {
            match self {
                Entry::Occupied(e) => e.key(),
                Entry::Vacant(e) => e.key(),
            }
        }
    }

    impl<'a, K: Hash + Eq, V, const N: usize> Entry<'a, K, V, N> {
        /// Ensures a value is in the entry by inserting the default if
        /// empty, and returns a mutable reference to the value in the
        /// entry.
        pub fn or_insert(self, default: V) -> &'a mut V {
            match self {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => e.insert(default),
            }
        }

        /// Ensures a value is in the entry by inserting the result of the
        /// default function if empty, and returns a mutable reference to
        /// the value in the entry.
        pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
            match self {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => e.insert(default()),
            }
        }

        /// Ensures a value is in the entry by inserting the default value
        /// if empty, and returns a mutable reference to the value in the
        /// entry.
        pub fn or_default(self) -> &'a mut V
        where
            V: Default,
        {
            self.or_insert_with(V::default)
        }
    }

    enum OccupiedEntryRepr<'a, K, V, const N: usize> {
        Inline {
            vec: &'a mut SmallVec<[(K, V); N]>,
            index: usize,
        },
        Spilled(indexmap::map::OccupiedEntry<'a, K, V>),
    }

    /// A view into an occupied entry in a [SmallMap].
    pub struct OccupiedEntry<'a, K, V, const N: usize>(OccupiedEntryRepr<'a, K, V, N>);

    impl<'a, K, V, const N: usize> OccupiedEntry<'a, K, V, N> {
        /// Gets a reference to the key in the entry.
        pub fn key(&self) -> &K {
            match &self.0 {
                OccupiedEntryRepr::Inline { vec, index } => &vec[*index].0,
                OccupiedEntryRepr::Spilled(e) => e.key(),
            }
        }

        /// Gets a reference to the value in the entry.
        pub fn get(&self) -> &V {
            match &self.0 {
                OccupiedEntryRepr::Inline { vec, index } => &vec[*index].1,
                OccupiedEntryRepr::Spilled(e) => e.get(),
            }
        }

        /// Gets a mutable reference to the value in the entry.
        pub fn get_mut(&mut self) -> &mut V {
            match &mut self.0 {
                OccupiedEntryRepr::Inline { vec, index } => &mut vec[*index].1,
                OccupiedEntryRepr::Spilled(e) => e.get_mut(),
            }
        }

        /// Converts the `OccupiedEntry` into a mutable reference to the
        /// value in the entry with a lifetime bound to the map itself.
        pub fn into_mut(self) -> &'a mut V {
            match self.0 {
                OccupiedEntryRepr::Inline { vec, index } => &mut vec[index].1,
                OccupiedEntryRepr::Spilled(e) => e.into_mut(),
            }
        }

        /// Sets the value of the entry, and returns the entry's old value.
        pub fn insert(&mut self, value: V) -> V {
            mem::replace(self.get_mut(), value)
        }

        /// Takes the value out of the entry, and returns it.
        pub fn remove(self) -> V {
            match self.0 {
                OccupiedEntryRepr::Inline { vec, index } => vec.swap_remove(index).1,
                OccupiedEntryRepr::Spilled(e) => e.swap_remove(),
            }
        }
    }

    enum VacantEntryRepr<'a, K, V, const N: usize> {
        Inline {
            repr: &'a mut MapRepr<K, V, N>,
            key: K,
        },
        Spilled(indexmap::map::VacantEntry<'a, K, V>),
    }

    /// A view into a vacant entry in a [SmallMap].
    pub struct VacantEntry<'a, K, V, const N: usize>(VacantEntryRepr<'a, K, V, N>);

    impl<'a, K, V, const N: usize> VacantEntry<'a, K, V, N> {
        /// Gets a reference to the key that would be used when inserting a
        /// value through the `VacantEntry`.
        pub fn key(&self) -> &K {
            match &self.0 {
                VacantEntryRepr::Inline { key, .. } => key,
                VacantEntryRepr::Spilled(e) => e.key(),
            }
        }
    }

    impl<'a, K: Hash + Eq, V, const N: usize> VacantEntry<'a, K, V, N> {
        /// Sets the value of the entry with the `VacantEntry`'s key, and
        /// returns a mutable reference to it.
        pub fn insert(self, value: V) -> &'a mut V {
            match self.0 {
                VacantEntryRepr::Inline { repr, key } => {
                    let needs_promotion = match &*repr {
                        MapRepr::Inline(v) => v.len() >= N,
                        MapRepr::Spilled(_) => unreachable!(),
                    };
                    if !needs_promotion {
                        let MapRepr::Inline(v) = repr else {
                            unreachable!()
                        };
                        v.push((key, value));
                        &mut v.last_mut().unwrap().1
                    } else {
                        let MapRepr::Inline(v) = &mut *repr else {
                            unreachable!()
                        };
                        let mut map = SmallMap::<K, V, N>::promote(v);
                        map.insert(key, value);
                        *repr = MapRepr::Spilled(map);
                        let MapRepr::Spilled(map) = repr else {
                            unreachable!()
                        };
                        let index = map.len() - 1;
                        map.get_index_mut(index).unwrap().1
                    }
                }
                VacantEntryRepr::Spilled(e) => e.insert(value),
            }
        }
    }

    /// Iterator over the key-value pairs of a [SmallMap]. See
    /// [SmallMap::iter].
    pub enum MapIter<'a, K, V> {
        Inline(core::slice::Iter<'a, (K, V)>),
        Spilled(indexmap::map::Iter<'a, K, V>),
    }

    impl<'a, K, V> Iterator for MapIter<'a, K, V> {
        type Item = (&'a K, &'a V);

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                MapIter::Inline(it) => it.next().map(|(k, v)| (k, v)),
                MapIter::Spilled(it) => it.next(),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                MapIter::Inline(it) => it.size_hint(),
                MapIter::Spilled(it) => it.size_hint(),
            }
        }
    }

    impl<'a, K, V> Clone for MapIter<'a, K, V> {
        fn clone(&self) -> Self {
            match self {
                MapIter::Inline(it) => MapIter::Inline(it.clone()),
                MapIter::Spilled(it) => MapIter::Spilled(it.clone()),
            }
        }
    }

    /// Mutable iterator over the key-value pairs of a [SmallMap]. See
    /// [SmallMap::iter_mut].
    pub enum MapIterMut<'a, K, V> {
        Inline(core::slice::IterMut<'a, (K, V)>),
        Spilled(indexmap::map::IterMut<'a, K, V>),
    }

    impl<'a, K, V> Iterator for MapIterMut<'a, K, V> {
        type Item = (&'a K, &'a mut V);

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                MapIterMut::Inline(it) => it.next().map(|(k, v)| (&*k, v)),
                MapIterMut::Spilled(it) => it.next(),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                MapIterMut::Inline(it) => it.size_hint(),
                MapIterMut::Spilled(it) => it.size_hint(),
            }
        }
    }

    /// Owned iterator over the key-value pairs of a [SmallMap]. See
    /// `IntoIterator for SmallMap`.
    pub enum MapIntoIter<K, V, const N: usize> {
        Inline(smallvec::IntoIter<[(K, V); N]>),
        Spilled(indexmap::map::IntoIter<K, V>),
    }

    impl<K, V, const N: usize> Iterator for MapIntoIter<K, V, N> {
        type Item = (K, V);

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                MapIntoIter::Inline(it) => it.next(),
                MapIntoIter::Spilled(it) => it.next(),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                MapIntoIter::Inline(it) => it.size_hint(),
                MapIntoIter::Spilled(it) => it.size_hint(),
            }
        }
    }

    impl<K, V, const N: usize> IntoIterator for SmallMap<K, V, N> {
        type Item = (K, V);
        type IntoIter = MapIntoIter<K, V, N>;

        fn into_iter(self) -> Self::IntoIter {
            match self.0 {
                MapRepr::Inline(v) => MapIntoIter::Inline(v.into_iter()),
                MapRepr::Spilled(map) => MapIntoIter::Spilled((*map).into_iter()),
            }
        }
    }

    impl<'a, K, V, const N: usize> IntoIterator for &'a SmallMap<K, V, N> {
        type Item = (&'a K, &'a V);
        type IntoIter = MapIter<'a, K, V>;

        fn into_iter(self) -> Self::IntoIter {
            self.iter()
        }
    }

    impl<K: Hash + Eq, V, const N: usize> Extend<(K, V)> for SmallMap<K, V, N> {
        fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
            for (k, v) in iter {
                self.insert(k, v);
            }
        }
    }

    impl<K: Hash + Eq, V, const N: usize> FromIterator<(K, V)> for SmallMap<K, V, N> {
        fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
            let mut map = Self::default();
            map.extend(iter);
            map
        }
    }

    impl<K: Hash + Eq, V: PartialEq, const N: usize> PartialEq for SmallMap<K, V, N> {
        fn eq(&self, other: &Self) -> bool {
            self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
        }
    }

    impl<K: Hash + Eq, V: Eq, const N: usize> Eq for SmallMap<K, V, N> {}

    impl<K: fmt::Debug, V: fmt::Debug, const N: usize> fmt::Debug for SmallMap<K, V, N> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_map().entries(self.iter()).finish()
        }
    }

    impl<K: Clone, V: Clone, const N: usize> Clone for SmallMap<K, V, N> {
        fn clone(&self) -> Self {
            match &self.0 {
                MapRepr::Inline(v) => SmallMap(MapRepr::Inline(v.clone())),
                MapRepr::Spilled(map) => SmallMap(MapRepr::Spilled(map.clone())),
            }
        }
    }

    /// Backing storage for a [SmallSet]: either up to `N` elements inline
    /// in a flat, linearly-scanned array, or (once grown past `N`
    /// elements) a heap-allocated [ISet].
    enum SetRepr<T, const N: usize> {
        Inline(SmallVec<[T; N]>),
        Spilled(Box<ISet<T>>),
    }

    /// A hash set, with a fast non-cryptographic hasher and deterministic
    /// iteration order, optimized for the common case of very few elements.
    ///
    /// Up to `N` entries are stored inline, in a flat, linearly-scanned
    /// array, with no heap allocation and no hashing. Once an insert
    /// grows the set past `N` entries, it transparently and permanently
    /// promotes itself to an [ISet]
    pub struct SmallSet<T, const N: usize>(SetRepr<T, N>);

    impl<T, const N: usize> Default for SmallSet<T, N> {
        fn default() -> Self {
            SmallSet(SetRepr::Inline(SmallVec::new()))
        }
    }

    impl<T, const N: usize> SmallSet<T, N> {
        /// Creates an empty [SmallSet]. Does not allocate until it grows
        /// past `N` elements.
        pub fn new() -> Self {
            Self::default()
        }

        /// Returns `true` if this set has not (yet) grown past `N`
        /// elements, i.e., is still using inline, non-heap-allocated
        /// storage.
        pub fn is_inline(&self) -> bool {
            matches!(self.0, SetRepr::Inline(_))
        }

        /// Returns the number of elements in the set.
        pub fn len(&self) -> usize {
            match &self.0 {
                SetRepr::Inline(v) => v.len(),
                SetRepr::Spilled(s) => s.len(),
            }
        }

        /// Returns `true` if the set contains no elements.
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// Clears the set, removing all values. A previously promoted set
        /// stays promoted; see the [SmallMap] documentation.
        pub fn clear(&mut self) {
            match &mut self.0 {
                SetRepr::Inline(v) => v.clear(),
                SetRepr::Spilled(s) => s.clear(),
            }
        }

        /// Retains only the elements specified by the predicate.
        ///
        /// In other words, remove all values `v` such that `f(&v)` returns
        /// `false`.
        pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut f: F) {
            match &mut self.0 {
                SetRepr::Inline(v) => v.retain(|v| f(v)),
                SetRepr::Spilled(s) => s.retain(f),
            }
        }

        /// Returns an iterator over the set's elements.
        pub fn iter(&self) -> SetIter<'_, T> {
            match &self.0 {
                SetRepr::Inline(v) => SetIter::Inline(v.iter()),
                SetRepr::Spilled(set) => SetIter::Spilled(set.iter()),
            }
        }
    }

    impl<T: Hash + Eq, const N: usize> SmallSet<T, N> {
        /// Moves from inline storage to a heap-allocated [ISet].
        fn promote(inline: &mut SmallVec<[T; N]>) -> Box<ISet<T>> {
            let mut set = ISet::with_capacity_and_hasher(inline.len() + 1, FxBuildHasher);
            for v in inline.drain(..) {
                set.insert(v);
            }
            Box::new(set)
        }

        /// Adds a value to the set.
        ///
        /// If the set did not have this value present, `true` is
        /// returned. If the set did have this value present, `false` is
        /// returned.
        pub fn insert(&mut self, value: T) -> bool {
            match &mut self.0 {
                SetRepr::Inline(v) => {
                    if v.contains(&value) {
                        return false;
                    }
                    if v.len() < N {
                        v.push(value);
                        true
                    } else {
                        let mut set = Self::promote(v);
                        let inserted = set.insert(value);
                        self.0 = SetRepr::Spilled(set);
                        inserted
                    }
                }
                SetRepr::Spilled(set) => set.insert(value),
            }
        }

        /// Returns `true` if the set contains a value.
        pub fn contains<Q>(&self, value: &Q) -> bool
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &self.0 {
                SetRepr::Inline(v) => v.iter().any(|x| x.borrow() == value),
                SetRepr::Spilled(set) => set.contains(value),
            }
        }

        /// Returns a reference to the value in the set, if any, that is
        /// equal to the given value.
        pub fn get<Q>(&self, value: &Q) -> Option<&T>
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &self.0 {
                SetRepr::Inline(v) => v.iter().find(|&x| x.borrow() == value),
                SetRepr::Spilled(set) => set.get(value),
            }
        }

        /// Removes a value from the set. Returns whether the value was
        /// present.
        pub fn remove<Q>(&mut self, value: &Q) -> bool
        where
            T: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            match &mut self.0 {
                SetRepr::Inline(v) => match v.iter().position(|x| x.borrow() == value) {
                    Some(i) => {
                        v.swap_remove(i);
                        true
                    }
                    None => false,
                },
                SetRepr::Spilled(set) => set.swap_remove(value),
            }
        }
    }

    /// Iterator over the elements of a [SmallSet]. See [SmallSet::iter].
    pub enum SetIter<'a, T> {
        Inline(core::slice::Iter<'a, T>),
        Spilled(indexmap::set::Iter<'a, T>),
    }

    impl<'a, T> Iterator for SetIter<'a, T> {
        type Item = &'a T;

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                SetIter::Inline(it) => it.next(),
                SetIter::Spilled(it) => it.next(),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                SetIter::Inline(it) => it.size_hint(),
                SetIter::Spilled(it) => it.size_hint(),
            }
        }
    }

    impl<'a, T> Clone for SetIter<'a, T> {
        fn clone(&self) -> Self {
            match self {
                SetIter::Inline(it) => SetIter::Inline(it.clone()),
                SetIter::Spilled(it) => SetIter::Spilled(it.clone()),
            }
        }
    }

    /// Owned iterator over the elements of a [SmallSet]. See
    /// `IntoIterator for SmallSet`.
    pub enum SetIntoIter<T, const N: usize> {
        Inline(smallvec::IntoIter<[T; N]>),
        Spilled(indexmap::set::IntoIter<T>),
    }

    impl<T, const N: usize> Iterator for SetIntoIter<T, N> {
        type Item = T;

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                SetIntoIter::Inline(it) => it.next(),
                SetIntoIter::Spilled(it) => it.next(),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            match self {
                SetIntoIter::Inline(it) => it.size_hint(),
                SetIntoIter::Spilled(it) => it.size_hint(),
            }
        }
    }

    impl<T, const N: usize> IntoIterator for SmallSet<T, N> {
        type Item = T;
        type IntoIter = SetIntoIter<T, N>;

        fn into_iter(self) -> Self::IntoIter {
            match self.0 {
                SetRepr::Inline(v) => SetIntoIter::Inline(v.into_iter()),
                SetRepr::Spilled(set) => SetIntoIter::Spilled((*set).into_iter()),
            }
        }
    }

    impl<'a, T, const N: usize> IntoIterator for &'a SmallSet<T, N> {
        type Item = &'a T;
        type IntoIter = SetIter<'a, T>;

        fn into_iter(self) -> Self::IntoIter {
            self.iter()
        }
    }

    impl<T: Hash + Eq, const N: usize> Extend<T> for SmallSet<T, N> {
        fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
            for v in iter {
                self.insert(v);
            }
        }
    }

    impl<T: Hash + Eq, const N: usize> FromIterator<T> for SmallSet<T, N> {
        fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
            let mut set = Self::default();
            set.extend(iter);
            set
        }
    }

    impl<T: Hash + Eq, const N: usize> PartialEq for SmallSet<T, N> {
        fn eq(&self, other: &Self) -> bool {
            self.len() == other.len() && self.iter().all(|v| other.contains(v))
        }
    }

    impl<T: Hash + Eq, const N: usize> Eq for SmallSet<T, N> {}

    impl<T: fmt::Debug, const N: usize> fmt::Debug for SmallSet<T, N> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_set().entries(self.iter()).finish()
        }
    }

    impl<T: Clone, const N: usize> Clone for SmallSet<T, N> {
        fn clone(&self) -> Self {
            match &self.0 {
                SetRepr::Inline(v) => SmallSet(SetRepr::Inline(v.clone())),
                SetRepr::Spilled(set) => SmallSet(SetRepr::Spilled(set.clone())),
            }
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
            s.swap_remove(&2);
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
        fn hmap_entry_occupied_vacant_match() {
            use crate::utils::table::htable::Entry;

            let mut m: HMap<&str, i32> = HMap::default();
            match m.entry("a") {
                Entry::Occupied(_) => panic!("expected vacant entry"),
                Entry::Vacant(v) => {
                    assert_eq!(v.key(), &"a");
                    v.insert(1);
                }
            }
            match m.entry("a") {
                Entry::Occupied(mut o) => {
                    assert_eq!(o.key(), &"a");
                    assert_eq!(o.get(), &1);
                    assert_eq!(o.insert(2), 1);
                    assert_eq!(o.remove(), 2);
                }
                Entry::Vacant(_) => panic!("expected occupied entry"),
            }
            assert!(m.is_empty());
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
        fn hmap_index() {
            let mut m = HMap::default();
            m.insert("a", 1);
            assert_eq!(m["a"], 1);
        }

        #[test]
        #[should_panic]
        fn hmap_index_missing_key_panics() {
            let m: HMap<&str, i32> = HMap::default();
            let _ = m["missing"];
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

    mod smalltable {
        use crate::utils::table::smalltable::{SmallMap, SmallSet};
        use alloc::{vec, vec::Vec};

        #[test]
        fn smallmap_stays_inline_below_threshold() {
            let mut m: SmallMap<i32, &str, 2> = SmallMap::new();
            assert!(m.is_inline());
            m.insert(1, "one");
            assert!(m.is_inline());
            m.insert(2, "two");
            assert!(m.is_inline());
            assert_eq!(m.len(), 2);
        }

        #[test]
        fn smallmap_promotes_past_threshold() {
            let mut m: SmallMap<i32, &str, 2> = SmallMap::new();
            m.insert(1, "one");
            m.insert(2, "two");
            assert!(m.is_inline());
            m.insert(3, "three");
            assert!(!m.is_inline());
            assert_eq!(m.len(), 3);
            assert_eq!(m.get(&1), Some(&"one"));
            assert_eq!(m.get(&2), Some(&"two"));
            assert_eq!(m.get(&3), Some(&"three"));
        }

        #[test]
        fn smallmap_stays_promoted_after_shrinking() {
            let mut m: SmallMap<i32, i32, 2> = SmallMap::new();
            m.insert(1, 1);
            m.insert(2, 2);
            m.insert(3, 3);
            assert!(!m.is_inline());
            m.remove(&3);
            m.remove(&2);
            assert_eq!(m.len(), 1);
            assert!(!m.is_inline());
        }

        #[test]
        fn smallmap_insert_overwrites_existing_key() {
            let mut m: SmallMap<&str, i32, 2> = SmallMap::new();
            assert_eq!(m.insert("a", 1), None);
            assert_eq!(m.insert("a", 2), Some(1));
            assert_eq!(m.get("a"), Some(&2));
            assert_eq!(m.len(), 1);
        }

        #[test]
        fn smallmap_get_mut_and_key_value() {
            let mut m: SmallMap<&str, i32, 2> = SmallMap::new();
            m.insert("a", 1);
            if let Some(v) = m.get_mut("a") {
                *v += 41;
            }
            assert_eq!(m.get("a"), Some(&42));
            assert_eq!(m.get_key_value("a"), Some((&"a", &42)));
            assert!(m.contains_key("a"));
            assert!(!m.contains_key("missing"));
        }

        #[test]
        fn smallmap_entry_api_inline() {
            let mut m: SmallMap<&str, i32, 2> = SmallMap::new();
            *m.entry("a").or_insert(0) += 1;
            *m.entry("a").or_insert(0) += 1;
            *m.entry("b").or_insert(5) += 1;
            assert!(m.is_inline());
            assert_eq!(m.get("a"), Some(&2));
            assert_eq!(m.get("b"), Some(&6));

            let mut counts: SmallMap<&str, i32, 2> = SmallMap::new();
            *counts.entry("x").or_default() += 1;
            assert_eq!(counts.get("x"), Some(&1));
        }

        #[test]
        fn smallmap_entry_api_triggers_promotion() {
            let mut m: SmallMap<i32, i32, 2> = SmallMap::new();
            m.entry(1).or_insert(1);
            m.entry(2).or_insert(2);
            assert!(m.is_inline());
            // A third distinct key should promote to the spilled repr.
            *m.entry(3).or_insert(0) += 3;
            assert!(!m.is_inline());
            assert_eq!(m.get(&1), Some(&1));
            assert_eq!(m.get(&2), Some(&2));
            assert_eq!(m.get(&3), Some(&3));
        }

        #[test]
        fn smallmap_entry_api_spilled() {
            let mut m: SmallMap<i32, i32, 1> = [(1, 10), (2, 20)].into_iter().collect();
            assert!(!m.is_inline());
            *m.entry(1).or_insert(0) += 1;
            m.entry(3).or_insert(30);
            assert_eq!(m.get(&1), Some(&11));
            assert_eq!(m.get(&3), Some(&30));
        }

        #[test]
        fn smallmap_entry_and_modify() {
            let mut m: SmallMap<&str, i32, 2> = SmallMap::new();
            m.entry("a").and_modify(|v| *v += 1).or_insert(1);
            m.entry("a").and_modify(|v| *v += 1).or_insert(1);
            assert_eq!(m.get("a"), Some(&2));
        }

        #[test]
        fn smallmap_entry_occupied_vacant_match() {
            use crate::utils::table::smalltable::Entry;

            let mut m: SmallMap<&str, i32, 2> = SmallMap::new();
            match m.entry("a") {
                Entry::Occupied(_) => panic!("expected vacant entry"),
                Entry::Vacant(v) => {
                    assert_eq!(v.key(), &"a");
                    v.insert(1);
                }
            }
            match m.entry("a") {
                Entry::Occupied(mut o) => {
                    assert_eq!(o.key(), &"a");
                    assert_eq!(o.get(), &1);
                    assert_eq!(o.insert(2), 1);
                    assert_eq!(o.remove(), 2);
                }
                Entry::Vacant(_) => panic!("expected occupied entry"),
            }
            assert!(m.is_empty());
        }

        #[test]
        fn smallmap_remove_inline_and_spilled() {
            let mut m: SmallMap<i32, &str, 2> = SmallMap::new();
            for i in [1, 2, 3] {
                m.insert(i, "v");
            }
            assert!(!m.is_inline());
            assert_eq!(m.remove(&1), Some("v"));
            assert_eq!(m.remove(&1), None);
            assert_eq!(m.len(), 2);

            let mut m: SmallMap<i32, &str, 4> = SmallMap::new();
            m.insert(1, "v");
            assert!(m.is_inline());
            assert_eq!(m.remove(&1), Some("v"));
            assert!(m.is_empty());
        }

        #[test]
        fn smallmap_retain_inline_and_spilled() {
            let mut m: SmallMap<i32, i32, 4> = SmallMap::new();
            for i in 1..=3 {
                m.insert(i, i * 10);
            }
            assert!(m.is_inline());
            m.retain(|k, _| k % 2 == 1);
            assert!(m.is_inline());
            let mut keys: Vec<_> = m.keys().copied().collect();
            keys.sort();
            assert_eq!(keys, vec![1, 3]);

            let mut m: SmallMap<i32, i32, 2> = SmallMap::new();
            for i in 1..=4 {
                m.insert(i, i * 10);
            }
            assert!(!m.is_inline());
            m.retain(|k, _| k % 2 == 1);
            // A promoted map stays promoted.
            assert!(!m.is_inline());
            let mut keys: Vec<_> = m.keys().copied().collect();
            keys.sort();
            assert_eq!(keys, vec![1, 3]);
        }

        #[test]
        fn smallmap_from_iter_and_extend() {
            let mut m: SmallMap<i32, i32, 2> = [(1, 10), (2, 20)].into_iter().collect();
            assert_eq!(m.get(&1), Some(&10));
            m.extend([(3, 30)]);
            assert!(!m.is_inline());
            let mut keys: Vec<_> = m.keys().copied().collect();
            keys.sort();
            assert_eq!(keys, vec![1, 2, 3]);
        }

        #[test]
        fn smallmap_values_mut_and_into_iter() {
            let mut m: SmallMap<i32, i32, 2> = SmallMap::new();
            for i in [3, 1, 4] {
                m.insert(i, i * i);
            }
            assert!(!m.is_inline());
            for v in m.values_mut() {
                *v += 1;
            }
            let mut values: Vec<_> = m.values().copied().collect();
            values.sort();
            assert_eq!(values, vec![2, 10, 17]);

            let mut pairs: Vec<_> = (&m).into_iter().map(|(k, v)| (*k, *v)).collect();
            pairs.sort();
            assert_eq!(pairs, vec![(1, 2), (3, 10), (4, 17)]);

            let mut pairs: Vec<_> = m.into_iter().collect();
            pairs.sort();
            assert_eq!(pairs, vec![(1, 2), (3, 10), (4, 17)]);
        }

        #[test]
        fn smallmap_iter_is_clone() {
            let m: SmallMap<i32, i32, 4> = [(3, 30), (1, 10), (4, 40)].into_iter().collect();
            assert!(m.is_inline());
            let it = m.iter();
            let mut a: Vec<_> = it.clone().map(|(k, v)| (*k, *v)).collect();
            let mut b: Vec<_> = it.map(|(k, v)| (*k, *v)).collect();
            a.sort();
            b.sort();
            assert_eq!(a, vec![(1, 10), (3, 30), (4, 40)]);
            assert_eq!(a, b);

            let m: SmallMap<i32, i32, 2> = [(3, 30), (1, 10), (4, 40)].into_iter().collect();
            assert!(!m.is_inline());
            let it = m.iter();
            let mut a: Vec<_> = it.clone().map(|(k, v)| (*k, *v)).collect();
            let mut b: Vec<_> = it.map(|(k, v)| (*k, *v)).collect();
            a.sort();
            b.sort();
            assert_eq!(a, vec![(1, 10), (3, 30), (4, 40)]);
            assert_eq!(a, b);
        }

        #[test]
        fn smallmap_equality_ignores_inline_vs_spilled() {
            let m1: SmallMap<i32, i32, 1> = [(1, 1), (2, 2)].into_iter().collect();
            let m2: SmallMap<i32, i32, 4> = [(2, 2), (1, 1)].into_iter().collect();
            assert!(!m1.is_inline());
            assert!(m2.is_inline());
            assert_eq!(m1.len(), m2.len());
            assert!(m1.iter().all(|(k, v)| m2.get(k) == Some(v)));
        }

        #[test]
        fn smallmap_debug_shows_contents() {
            let mut m: SmallMap<&str, i32, 2> = SmallMap::new();
            m.insert("a", 1);
            let dbg = alloc::format!("{m:?}");
            assert!(dbg.contains('a'));
            assert!(dbg.contains('1'));
        }

        #[test]
        fn smallmap_clone_is_independent() {
            let mut m1: SmallMap<i32, i32, 2> = SmallMap::new();
            m1.insert(1, 1);
            let mut m2 = m1.clone();
            m2.insert(2, 2);
            assert_eq!(m1.len(), 1);
            assert_eq!(m2.len(), 2);
        }

        #[test]
        fn smallset_stays_inline_below_threshold() {
            let mut s: SmallSet<i32, 2> = SmallSet::new();
            assert!(s.is_inline());
            s.insert(1);
            s.insert(2);
            assert!(s.is_inline());
            assert_eq!(s.len(), 2);
        }

        #[test]
        fn smallset_promotes_past_threshold() {
            let mut s: SmallSet<i32, 2> = SmallSet::new();
            s.insert(1);
            s.insert(2);
            assert!(s.is_inline());
            s.insert(3);
            assert!(!s.is_inline());
            assert_eq!(s.len(), 3);
            assert!(s.contains(&1));
            assert!(s.contains(&2));
            assert!(s.contains(&3));
        }

        #[test]
        fn smallset_insert_duplicate_is_noop() {
            let mut s: SmallSet<i32, 2> = SmallSet::new();
            assert!(s.insert(1));
            assert!(!s.insert(1));
            assert_eq!(s.len(), 1);

            let mut s: SmallSet<i32, 1> = SmallSet::new();
            s.insert(1);
            s.insert(2); // promotes
            assert!(!s.is_inline());
            assert!(!s.insert(1));
            assert_eq!(s.len(), 2);
        }

        #[test]
        fn smallset_remove_inline_and_spilled() {
            let mut s: SmallSet<i32, 2> = [1, 2, 3].into_iter().collect();
            assert!(!s.is_inline());
            assert!(s.remove(&1));
            assert!(!s.remove(&1));
            assert_eq!(s.len(), 2);

            let mut s: SmallSet<i32, 4> = SmallSet::new();
            s.insert(1);
            assert!(s.is_inline());
            assert!(s.remove(&1));
            assert!(s.is_empty());
        }

        #[test]
        fn smallset_retain_inline_and_spilled() {
            let mut s: SmallSet<i32, 4> = [1, 2, 3].into_iter().collect();
            assert!(s.is_inline());
            s.retain(|v| v % 2 == 1);
            assert!(s.is_inline());
            let mut vals: Vec<_> = s.iter().copied().collect();
            vals.sort();
            assert_eq!(vals, vec![1, 3]);

            let mut s: SmallSet<i32, 2> = [1, 2, 3, 4].into_iter().collect();
            assert!(!s.is_inline());
            s.retain(|v| v % 2 == 1);
            // A promoted set stays promoted.
            assert!(!s.is_inline());
            let mut vals: Vec<_> = s.iter().copied().collect();
            vals.sort();
            assert_eq!(vals, vec![1, 3]);
        }

        #[test]
        fn smallset_get_and_contains() {
            let s: SmallSet<i32, 2> = [1, 2].into_iter().collect();
            assert_eq!(s.get(&1), Some(&1));
            assert_eq!(s.get(&3), None);
            assert!(s.contains(&2));
            assert!(!s.contains(&3));
        }

        #[test]
        fn smallset_from_iter_and_extend() {
            let mut s: SmallSet<i32, 2> = [1, 2].into_iter().collect();
            assert!(s.is_inline());
            s.extend([3]);
            assert!(!s.is_inline());
            let mut items: Vec<_> = s.iter().copied().collect();
            items.sort();
            assert_eq!(items, vec![1, 2, 3]);
        }

        #[test]
        fn smallset_into_iter_owned_and_borrowed() {
            let s: SmallSet<i32, 2> = [3, 1, 4].into_iter().collect();
            assert!(!s.is_inline());

            let mut items: Vec<_> = (&s).into_iter().copied().collect();
            items.sort();
            assert_eq!(items, vec![1, 3, 4]);

            let mut items: Vec<_> = s.into_iter().collect();
            items.sort();
            assert_eq!(items, vec![1, 3, 4]);
        }

        #[test]
        fn smallset_iter_is_clone() {
            let s: SmallSet<i32, 4> = [3, 1, 4].into_iter().collect();
            assert!(s.is_inline());
            let it = s.iter();
            let mut a: Vec<_> = it.clone().copied().collect();
            let mut b: Vec<_> = it.copied().collect();
            a.sort();
            b.sort();
            assert_eq!(a, vec![1, 3, 4]);
            assert_eq!(a, b);

            let s: SmallSet<i32, 2> = [3, 1, 4].into_iter().collect();
            assert!(!s.is_inline());
            let it = s.iter();
            let mut a: Vec<_> = it.clone().copied().collect();
            let mut b: Vec<_> = it.copied().collect();
            a.sort();
            b.sort();
            assert_eq!(a, vec![1, 3, 4]);
            assert_eq!(a, b);
        }

        #[test]
        fn smallset_equality_ignores_inline_vs_spilled() {
            let s1: SmallSet<i32, 1> = [1, 2].into_iter().collect();
            let s2: SmallSet<i32, 4> = [2, 1].into_iter().collect();
            assert!(!s1.is_inline());
            assert!(s2.is_inline());
            assert_eq!(s1.len(), s2.len());
            assert!(s1.iter().all(|v| s2.contains(v)));
        }

        #[test]
        fn smallset_partial_eq_same_n() {
            let s1: SmallSet<i32, 4> = [1, 2, 3].into_iter().collect();
            let mut s2: SmallSet<i32, 4> = SmallSet::new();
            s2.insert(3);
            s2.insert(2);
            s2.insert(1);
            assert_eq!(s1, s2);
            s2.insert(4);
            assert_ne!(s1, s2);
        }

        #[test]
        fn smallmap_partial_eq_same_n() {
            let m1: SmallMap<i32, i32, 4> = [(1, 1), (2, 2)].into_iter().collect();
            let mut m2: SmallMap<i32, i32, 4> = SmallMap::new();
            m2.insert(2, 2);
            m2.insert(1, 1);
            assert_eq!(m1, m2);
            m2.insert(1, 99);
            assert_ne!(m1, m2);
        }

        #[test]
        fn smallset_debug_shows_contents() {
            let mut s: SmallSet<&str, 2> = SmallSet::new();
            s.insert("hello");
            let dbg = alloc::format!("{s:?}");
            assert!(dbg.contains("hello"));
        }

        #[test]
        fn smallset_clone_is_independent() {
            let mut s1: SmallSet<i32, 2> = SmallSet::new();
            s1.insert(1);
            let mut s2 = s1.clone();
            s2.insert(2);
            assert_eq!(s1.len(), 1);
            assert_eq!(s2.len(), 2);
        }
    }
}
