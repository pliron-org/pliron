//! Arrayna is an array with the following properties:
//! * It can only grow, elements once inserted cannot be removed.
//! * Elements inserted cannot be modified.
//!   (use interior mutability if you want to modify the elements)
//! * Does not require a mutable reference for any operation.
//! * Memory allocation happens in steps of powers of two.
//! * The total size is limited `1 + 2 + 4 + ... + 2^(N-1) = 2^N - 1`
//! * Written entirely in safe rust.

use alloc::boxed::Box;
use core::cell::{Cell, OnceCell};

/// A grow-only array with a maximum size of `2^N - 1`.
/// See the module-level documentation for more details.
pub struct Arrayna<T, const N: usize = 32> {
    data: [OnceCell<Box<[OnceCell<T>]>>; N],
    len: Cell<usize>,
}

impl<T, const N: usize> Default for Arrayna<T, N> {
    fn default() -> Self {
        Self {
            data: core::array::from_fn(|_| OnceCell::new()),
            len: Cell::new(0),
        }
    }
}

impl<T, const N: usize> Arrayna<T, N> {
    /// The maximum length of the arrayna, determined by the number of chunks `N`.
    const fn max_len() -> usize {
        if N >= usize::BITS as usize {
            usize::MAX
        } else {
            (1usize << N) - 1
        }
    }

    /// Current length of the arrayna.
    pub fn len(&self) -> usize {
        self.len.get()
    }

    /// Returns true if the arrayna is empty.
    pub fn is_empty(&self) -> bool {
        self.len.get() == 0
    }

    // Map a flat zero-based index into the chunk index and the element offset
    // within that chunk. Chunk sizes grow as 1, 2, 4, ... so the chunk is the
    // highest set bit of index + 1, and the offset is the remaining distance
    // from the start of that chunk.
    fn locate(index: usize) -> (usize, usize) {
        let one_based = index + 1;
        let chunk = (usize::BITS - 1 - one_based.leading_zeros()) as usize;
        let chunk_start = (1usize << chunk) - 1;
        (chunk, index - chunk_start)
    }

    // Insert a new element into the arrayna,
    // returning the index of the inserted element.
    pub fn push(&self, value: T) -> usize {
        let index = self.len.get();
        assert!(index < Self::max_len(), "Arrayna is full");

        let (chunk_idx, offset) = Self::locate(index);
        let chunk = self.data[chunk_idx].get_or_init(|| {
            core::iter::repeat_with(OnceCell::new)
                .take(1usize << chunk_idx)
                .collect::<Box<[OnceCell<T>]>>()
        });

        assert!(
            chunk[offset].set(value).is_ok(),
            "Arrayna internal error: slot already initialized"
        );
        self.len.set(index + 1);
        index
    }

    // Get a reference to the element at the given index.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len.get() {
            return None;
        }

        let (chunk_idx, offset) = Self::locate(index);
        self.data[chunk_idx]
            .get()
            .and_then(|chunk| chunk[offset].get())
    }

    /// Iterate over all elements in the arrayna in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        (0..self.len.get()).map(move |i| self.get(i).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::Arrayna;
    use alloc::{
        string::{String, ToString},
        vec::Vec,
    };

    #[test]
    fn empty_arrayna_is_empty() {
        let arrayna = Arrayna::<i32, 4>::default();

        assert_eq!(arrayna.len(), 0);
        assert_eq!(arrayna.get(0), None);
        assert!(arrayna.iter().next().is_none());
    }

    #[test]
    fn push_returns_indices_and_get_round_trips_values() {
        let arrayna = Arrayna::<String, 4>::default();

        let first = arrayna.push("alpha".to_string());
        let second = arrayna.push("beta".to_string());
        let third = arrayna.push("gamma".to_string());

        assert_eq!(first, 0);
        assert_eq!(second, 1);
        assert_eq!(third, 2);
        assert_eq!(arrayna.len(), 3);
        assert_eq!(arrayna.get(0).map(String::as_str), Some("alpha"));
        assert_eq!(arrayna.get(1).map(String::as_str), Some("beta"));
        assert_eq!(arrayna.get(2).map(String::as_str), Some("gamma"));
        assert_eq!(arrayna.get(3), None);
    }

    #[test]
    fn iterates_in_insertion_order_across_chunk_boundaries() {
        let arrayna = Arrayna::<usize, 4>::default();
        let expected: Vec<_> = (0..10).collect();

        for value in &expected {
            arrayna.push(*value);
        }

        let collected: Vec<_> = arrayna.iter().copied().collect();
        assert_eq!(collected, expected);
    }

    #[test]
    fn supports_full_capacity_for_the_declared_chunk_count() {
        let arrayna = Arrayna::<usize, 4>::default();

        for value in 0..15 {
            assert_eq!(arrayna.push(value), value);
        }

        assert_eq!(arrayna.len(), 15);
        assert_eq!(arrayna.get(0), Some(&0));
        assert_eq!(arrayna.get(6), Some(&6));
        assert_eq!(arrayna.get(14), Some(&14));
        assert_eq!(arrayna.get(15), None);
    }

    #[test]
    #[should_panic(expected = "Arrayna is full")]
    fn panics_when_capacity_is_exceeded() {
        let arrayna = Arrayna::<usize, 4>::default();

        for value in 0..15 {
            arrayna.push(value);
        }

        arrayna.push(15);
    }
}
