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

#[cfg(feature = "std")]
pub mod io {
    pub use std::path::PathBuf;
}

#[cfg(not(feature = "std"))]
pub mod io {
    use alloc::string::String;
    use core::ops::{Deref, DerefMut};

    #[derive(PartialEq, Eq, Clone, Debug, Hash)]
    pub struct PathBuf(String);

    impl<T: Into<String>> From<T> for PathBuf {
        fn from(value: T) -> Self {
            PathBuf(value.into())
        }
    }

    impl Deref for PathBuf {
        type Target = String;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl DerefMut for PathBuf {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    impl PathBuf {
        pub fn display(&self) -> &str {
            &self.0
        }
    }
}

pub mod hash {
    pub type FxHashMap<K, V> = hashbrown::HashMap<K, V, rustc_hash::FxBuildHasher>;
    pub type FxHashSet<V> = hashbrown::HashSet<V, rustc_hash::FxBuildHasher>;
    pub use hashbrown::*;
}
