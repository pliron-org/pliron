// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! A union-find (disjoint-set) data structure.

use alloc::{vec, vec::Vec};
use core::hash::Hash;

use crate::std_deps::hash::FxHashMap;

/// A union-find (disjoint-set) structure over values of type `T`.
///
/// Elements need not be registered up front: any element passed to a method
/// is added, as a singleton set, on first sight. Uses path compression and
/// union-by-size; member lists are maintained so that enumerating a set
/// ([Self::set_members]) is O(set size).
#[derive(Clone, Debug)]
pub struct UnionFind<T: Eq + Hash + Clone> {
    parent: FxHashMap<T, T>,
    /// Members of each set, keyed by the set's representative.
    members: FxHashMap<T, Vec<T>>,
}

impl<T: Eq + Hash + Clone> Default for UnionFind<T> {
    fn default() -> Self {
        Self {
            parent: FxHashMap::default(),
            members: FxHashMap::default(),
        }
    }
}

impl<T: Eq + Hash + Clone> UnionFind<T> {
    /// Get the representative of `v`'s set, adding a singleton set if `v` is new.
    pub fn find(&mut self, v: T) -> T {
        match self.parent.get(&v) {
            None => {
                self.parent.insert(v.clone(), v.clone());
                self.members.insert(v.clone(), vec![v.clone()]);
                v
            }
            Some(p) if *p == v => v,
            Some(p) => {
                let root = self.find(p.clone());
                // Path compression.
                self.parent.insert(v, root.clone());
                root
            }
        }
    }

    /// Merge the sets of `a` and `b`.
    pub fn union(&mut self, a: T, b: T) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        // Union by size.
        let (big, small) = if self.members[&ra].len() >= self.members[&rb].len() {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent.insert(small.clone(), big.clone());
        let small_members = self
            .members
            .remove(&small)
            .expect("set root must have a members list");
        self.members
            .get_mut(&big)
            .expect("set root must have a members list")
            .extend(small_members);
    }

    /// Are `a` and `b` in the same set?
    pub fn in_same_set(&mut self, a: T, b: T) -> bool {
        self.find(a) == self.find(b)
    }

    /// All values in `v`'s set (including `v` itself), in no particular order.
    pub fn set_members(&mut self, v: T) -> &[T] {
        let root = self.find(v);
        &self.members[&root]
    }
}

#[cfg(test)]
mod tests {
    use super::UnionFind;
    use alloc::{vec, vec::Vec};

    fn sorted_members(uf: &mut UnionFind<u32>, v: u32) -> Vec<u32> {
        let mut members = uf.set_members(v).to_vec();
        members.sort();
        members
    }

    #[test]
    fn singletons_on_first_sight() {
        let mut uf = UnionFind::<u32>::default();
        assert_eq!(uf.find(7), 7);
        assert!(!uf.in_same_set(1, 2));
        assert_eq!(sorted_members(&mut uf, 7), vec![7]);
    }

    #[test]
    fn union_merges_sets() {
        let mut uf = UnionFind::<u32>::default();
        uf.union(1, 2);
        uf.union(3, 4);
        assert!(uf.in_same_set(1, 2));
        assert!(uf.in_same_set(3, 4));
        assert!(!uf.in_same_set(2, 3));

        uf.union(2, 3);
        assert!(uf.in_same_set(1, 4));
        assert_eq!(sorted_members(&mut uf, 4), vec![1, 2, 3, 4]);
    }

    #[test]
    fn union_is_idempotent() {
        let mut uf = UnionFind::<u32>::default();
        uf.union(1, 2);
        uf.union(2, 1);
        uf.union(1, 2);
        assert_eq!(sorted_members(&mut uf, 1), vec![1, 2]);
    }

    #[test]
    fn transitive_chain() {
        let mut uf = UnionFind::<u32>::default();
        for i in 0..10 {
            uf.union(i, i + 1);
        }
        assert!(uf.in_same_set(0, 10));
        assert_eq!(uf.set_members(5).len(), 11);
        // All elements resolve to the same representative.
        let root = uf.find(0);
        assert!((0..=10).all(|i| uf.find(i) == root));
    }
}
