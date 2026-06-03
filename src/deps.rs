pub mod sync {
    #[cfg(not(feature = "std"))]
    pub use spin::LazyLock;
    #[cfg(feature = "std")]
    pub use std::sync::LazyLock;
}

#[cfg(feature = "std")]
pub mod backtrace {
    pub use std::backtrace::{Backtrace, BacktraceStatus};
}

#[cfg(not(feature = "std"))]
pub mod backtrace {
    use core::fmt::{Debug, Display};

    #[non_exhaustive]
    #[derive(Debug, PartialEq, Eq)]
    pub enum BacktraceStatus {
        Unsupported,
        Disabled,
        Captured,
    }

    pub struct Backtrace;

    impl Debug for Backtrace {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("<disabled>")
        }
    }

    impl Display for Backtrace {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("disabled backtrace")
        }
    }

    impl Backtrace {
        pub fn capture() -> Self {
            Self
        }

        pub fn status(&self) -> BacktraceStatus {
            BacktraceStatus::Disabled
        }
    }
}

pub type FxHashMap<K, V> = hashbrown::HashMap<K, V, rustc_hash::FxBuildHasher>;
pub type FxHashSet<V> = hashbrown::HashSet<V, rustc_hash::FxBuildHasher>;

use alloc::string::String;

pub trait CharIter {
    fn chars_iter(&mut self) -> impl Iterator<Item = char>;
}

impl CharIter for String {
    fn chars_iter(&mut self) -> impl Iterator<Item = char> {
        self.chars()
    }
}

// This should ideally be any `T: BufRead`, but coherence rules don't allow it because `std` may
// add an implementation of `BufRead` for `String` in the future.
#[cfg(feature = "std")]
impl<T: std::io::Read> CharIter for std::io::BufReader<T> {
    fn chars_iter(&mut self) -> impl Iterator<Item = char> {
        use utf8_chars::BufReadCharsExt;
        self.chars().map(|c| {
            c.map_err(|e| std::eprintln!("Error reading chars from file: {e}"))
                .unwrap()
        })
    }
}
