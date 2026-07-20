// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! A utility to enforce statically that a value is less than a given constant `N`.

/// Statically enforced less than `N` value. Use [I] to construct.
///
/// Example:
///
/// ```rust
/// # use pliron::{i, utils::const_bound_n::{LessThanN, I}};
/// let x: LessThanN<5> = I::<3>.into();
///
/// // Alternatively, you can use the `i!` macro to construct a
/// // `LessThanN` value from an integer literal:
/// let y: LessThanN<5> = i!(3);
/// ```
pub struct LessThanN<const N: usize> {
    i: usize,
}

impl<const N: usize> LessThanN<N> {
    /// Get the contained [LessThanN] value.
    pub fn i(&self) -> usize {
        self.i
    }
}

/// Build a [LessThanN] value with only static checks.
///
/// Example:
///
/// ```rust
/// # use pliron::{i, utils::const_bound_n::{LessThanN, I}};
/// let x: LessThanN<5> = I::<3>.into();
/// // Alternatively, you can use the `i!` macro to construct a
/// // `LessThanN` value from an integer literal:
/// let y: LessThanN<5> = i!(3);
/// ```
pub struct I<const VALUE: usize>;

impl<const N: usize, const VALUE: usize> From<I<VALUE>> for LessThanN<N> {
    fn from(_i: I<VALUE>) -> Self {
        const {
            assert!(
                VALUE < N,
                "Error constructing LessThanN<N> from I::<VALUE>: VALUE must be less than N"
            );
        }
        Self { i: VALUE }
    }
}

/// Build a [LessThanN] value from an integer literal using static checks.
///
/// Example:
///
/// ```rust
/// # use pliron::{i, utils::const_bound_n::LessThanN};
/// let x: LessThanN<5> = i!(3);
/// ```
#[macro_export]
macro_rules! i {
    ($value:literal) => {
        $crate::utils::const_bound_n::I::<$value>.into()
    };
}
