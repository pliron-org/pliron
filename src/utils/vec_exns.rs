// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Extensions for library types

use alloc::vec::Vec;
use smallvec::{Array, SmallVec};

/// This trait provides additionaly functionality for Rust vectors
pub trait VecExtns<T> {
    /// Insert `t` and get back its index in the container.
    fn push_back(&mut self, t: T) -> usize;

    /// Insert a new element, constructed by calling `ctor` with the element's index,
    /// and get back its index in the container.
    fn push_back_with(&mut self, ctor: impl FnMut(usize) -> T) -> usize;

    /// Insert `t` if it isn't already in the container.
    /// Get back the index of the existing element or of the new element.
    fn push_unique(&mut self, t: T) -> usize
    where
        T: PartialEq;

    /// Grow the vector to the new size, initializing new elements with `initf`.
    fn grow_to(&mut self, new_size: usize, initf: impl FnMut(usize) -> T);
}

impl<T> VecExtns<T> for Vec<T> {
    fn push_back(&mut self, t: T) -> usize {
        self.push(t);
        self.len() - 1
    }

    fn push_back_with(&mut self, mut ctor: impl FnMut(usize) -> T) -> usize {
        let idx = self.len();
        self.push(ctor(idx));
        idx
    }

    fn push_unique(&mut self, t: T) -> usize
    where
        T: PartialEq,
    {
        self.iter()
            .position(|e| *e == t)
            .unwrap_or_else(|| self.push_back(t))
    }

    fn grow_to(&mut self, new_size: usize, mut initf: impl FnMut(usize) -> T) {
        let current_size = self.len();
        if new_size > current_size {
            self.reserve(new_size - current_size);
            for i in current_size..new_size {
                self.push(initf(i));
            }
        }
    }
}

impl<A: Array> VecExtns<A::Item> for SmallVec<A> {
    fn push_back(&mut self, t: A::Item) -> usize {
        self.push(t);
        self.len() - 1
    }

    fn push_back_with(&mut self, mut ctor: impl FnMut(usize) -> A::Item) -> usize {
        let idx = self.len();
        self.push(ctor(idx));
        idx
    }

    fn push_unique(&mut self, t: A::Item) -> usize
    where
        A::Item: PartialEq,
    {
        self.iter()
            .position(|e| *e == t)
            .unwrap_or_else(|| self.push_back(t))
    }

    fn grow_to(&mut self, new_size: usize, mut initf: impl FnMut(usize) -> A::Item) {
        let current_size = self.len();
        if new_size > current_size {
            self.reserve(new_size - current_size);
            for i in current_size..new_size {
                self.push(initf(i));
            }
        }
    }
}

/// This trait provides additional functionality for Rust iterators.
pub trait IterExtns: Iterator {
    /// Collect the items into `C`, skipping items equal to one already collected.
    fn collect_unique<C>(self) -> C
    where
        Self: Sized,
        Self::Item: PartialEq,
        C: VecExtns<Self::Item> + Default,
    {
        let mut collected = C::default();
        for item in self {
            collected.push_unique(item);
        }
        collected
    }
}

impl<I: Iterator> IterExtns for I {}
