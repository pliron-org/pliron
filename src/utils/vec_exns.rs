// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Extend functionality of Rust vectors.

use alloc::vec::Vec;

/// This trait provides additionaly functionality for Rust vectors
pub trait VecExtns<T> {
    /// Insert a new element and get back its index in the container.
    fn push_back(&mut self, t: T) -> usize;
    /// Insert a new element, constructed by calling `ctor`,
    /// and get back its index in the container. `ctor` will
    /// receve the index as a parameter. This is useful when the
    /// element inserted needs to know its index.
    fn push_back_with(&mut self, ctor: impl FnMut(usize) -> T) -> usize;

    /// Create and initialize a new vector.
    fn new_init<U: FnMut(usize) -> T>(size: usize, initf: U) -> Vec<T>;

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

    fn new_init<U: FnMut(usize) -> T>(size: usize, mut initf: U) -> Vec<T> {
        let mut v = Vec::<T>::with_capacity(size);
        for i in 0..size {
            v.push(initf(i));
        }
        v
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
