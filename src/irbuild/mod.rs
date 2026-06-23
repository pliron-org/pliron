//! Utilities for building and modifying IR.

pub mod cloning;
pub mod dialect_conversion;
pub mod inserter;
pub mod listener;
pub mod match_rewrite;
pub mod rewriter;

use core::ops::{BitOr, BitOrAssign};

/// Indicates whether a pass changed the IR or not.
/// Implements `|` and `|=` for convenience.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IRStatus {
    #[default]
    Unchanged,
    Changed,
}

impl From<bool> for IRStatus {
    fn from(changed: bool) -> Self {
        if changed {
            IRStatus::Changed
        } else {
            IRStatus::Unchanged
        }
    }
}

impl From<IRStatus> for bool {
    fn from(status: IRStatus) -> Self {
        match status {
            IRStatus::Unchanged => false,
            IRStatus::Changed => true,
        }
    }
}

impl BitOr for IRStatus {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (IRStatus::Unchanged, IRStatus::Unchanged) => IRStatus::Unchanged,
            _ => IRStatus::Changed,
        }
    }
}

impl BitOrAssign for IRStatus {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}
