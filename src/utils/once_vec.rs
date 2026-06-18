//! [OnceVec] is a [OnceCell]-based grow-only vector.
//!
//! It has the following properties:
//! * It can only grow, elements once pushed cannot be removed.
//! * Elements once pushed cannot be modified.
//!   (use interior mutability if you want to modify the elements)
//! * Pushing new elements does not require a mutable reference.
//! * Memory allocation happens in steps of powers of two.
//! * The total size is limited `1 + 2 + 4 + ... + 2^(N-1) = 2^N - 1`
//! * Written entirely in safe rust.

use alloc::boxed::Box;
use core::cell::{Cell, OnceCell};

/// A grow-only array with a maximum size of `2^N - 1`.
/// See the module-level documentation for more details.
pub struct OnceVec<T, const N: usize = 32> {
    data: [OnceCell<Box<[OnceCell<T>]>>; N],
    len: Cell<usize>,
}

impl<T, const N: usize> Default for OnceVec<T, N> {
    fn default() -> Self {
        Self {
            data: core::array::from_fn(|_| OnceCell::new()),
            len: Cell::new(0),
        }
    }
}

impl<T, const N: usize> OnceVec<T, N> {
    /// The maximum length of the OnceVec, determined by the number of chunks `N`.
    const fn max_len() -> usize {
        if N >= usize::BITS as usize {
            usize::MAX
        } else {
            (1usize << N) - 1
        }
    }

    /// Current length of the OnceVec.
    pub fn len(&self) -> usize {
        self.len.get()
    }

    /// Returns true if the OnceVec is empty.
    pub fn is_empty(&self) -> bool {
        self.len.get() == 0
    }

    /// Clear all elements from the OnceVec, resetting it to an empty state.
    /// Note: This will drop all existing elements and reset the length to zero.
    pub fn clear(&mut self) {
        for cell in &mut self.data {
            cell.take();
        }
        self.len.set(0);
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

    // Insert a new element into the OnceVec,
    // returning the index of the inserted element.
    pub fn push(&self, value: T) -> usize {
        let index = self.len.get();
        assert!(index < Self::max_len(), "OnceVec is full");

        let (chunk_idx, offset) = Self::locate(index);
        let chunk = self.data[chunk_idx].get_or_init(|| {
            core::iter::repeat_with(OnceCell::new)
                .take(1usize << chunk_idx)
                .collect::<Box<[OnceCell<T>]>>()
        });

        assert!(
            chunk[offset].set(value).is_ok(),
            "OnceVec internal error: slot already initialized"
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

    /// Iterate over all elements in the OnceVec in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        (0..self.len.get()).map(move |i| self.get(i).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::OnceVec;
    use alloc::{
        string::{String, ToString},
        vec::Vec,
    };

    #[test]
    fn empty_once_vec_is_empty() {
        let once_vec = OnceVec::<i32, 4>::default();

        assert!(once_vec.is_empty());
        assert_eq!(once_vec.len(), 0);
        assert_eq!(once_vec.get(0), None);
        assert!(once_vec.iter().next().is_none());
    }

    #[test]
    fn is_empty_tracks_state_transitions() {
        let mut once_vec = OnceVec::<usize, 4>::default();

        assert!(once_vec.is_empty());
        once_vec.push(10);
        assert!(!once_vec.is_empty());

        once_vec.clear();
        assert!(once_vec.is_empty());
    }

    #[test]
    fn push_returns_indices_and_get_round_trips_values() {
        let once_vec = OnceVec::<String, 4>::default();

        let first = once_vec.push("alpha".to_string());
        let second = once_vec.push("beta".to_string());
        let third = once_vec.push("gamma".to_string());

        assert_eq!(first, 0);
        assert_eq!(second, 1);
        assert_eq!(third, 2);
        assert_eq!(once_vec.len(), 3);
        assert_eq!(once_vec.get(0).map(String::as_str), Some("alpha"));
        assert_eq!(once_vec.get(1).map(String::as_str), Some("beta"));
        assert_eq!(once_vec.get(2).map(String::as_str), Some("gamma"));
        assert_eq!(once_vec.get(3), None);
    }

    #[test]
    fn iterates_in_insertion_order_across_chunk_boundaries() {
        let once_vec = OnceVec::<usize, 4>::default();
        let expected: Vec<_> = (0..10).collect();

        for value in &expected {
            once_vec.push(*value);
        }

        let collected: Vec<_> = once_vec.iter().copied().collect();
        assert_eq!(collected, expected);
    }

    #[test]
    fn supports_full_capacity_for_the_declared_chunk_count() {
        let once_vec = OnceVec::<usize, 4>::default();

        for value in 0..15 {
            assert_eq!(once_vec.push(value), value);
        }

        assert_eq!(once_vec.len(), 15);
        assert_eq!(once_vec.get(0), Some(&0));
        assert_eq!(once_vec.get(6), Some(&6));
        assert_eq!(once_vec.get(14), Some(&14));
        assert_eq!(once_vec.get(15), None);
    }

    #[test]
    #[should_panic(expected = "OnceVec is full")]
    fn panics_when_capacity_is_exceeded() {
        let once_vec = OnceVec::<usize, 4>::default();

        for value in 0..15 {
            once_vec.push(value);
        }

        once_vec.push(15);
    }

    #[test]
    fn clear_removes_elements_and_allows_reinsertion() {
        let mut once_vec = OnceVec::<usize, 4>::default();

        for value in 0..7 {
            once_vec.push(value);
        }
        assert_eq!(once_vec.len(), 7);
        assert!(!once_vec.is_empty());

        once_vec.clear();

        assert_eq!(once_vec.len(), 0);
        assert!(once_vec.is_empty());
        assert_eq!(once_vec.get(0), None);
        assert!(once_vec.iter().next().is_none());

        let idx0 = once_vec.push(100);
        let idx1 = once_vec.push(200);
        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(once_vec.len(), 2);
        assert_eq!(once_vec.get(0), Some(&100));
        assert_eq!(once_vec.get(1), Some(&200));
    }
}
