//! All our `std` dependencies are used from re-exports here.
//! Dummy implementations are provided for when `std` is disabled.

#[cfg(feature = "std")]
mod r#impl {
    pub mod sync {
        pub use std::sync::LazyLock;
    }

    pub mod backtrace {
        pub use std::backtrace::{Backtrace, BacktraceStatus};
    }

    pub mod io {
        pub use std::path::PathBuf;
    }

    pub mod time {
        pub use std::time::Instant;
    }

    pub mod hash {
        pub use std::collections::{HashMap, HashSet, hash_map, hash_set};
        pub type FxHashMap<K, V> = HashMap<K, V, rustc_hash::FxBuildHasher>;
        pub type FxHashSet<V> = HashSet<V, rustc_hash::FxBuildHasher>;
    }
}

#[cfg(not(feature = "std"))]
mod r#impl {
    pub mod sync {
        pub use spin::LazyLock;
    }

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

    pub mod time {
        pub struct Instant;
        impl Instant {
            pub fn now() -> Self {
                Self
            }

            pub fn elapsed(&self) -> core::time::Duration {
                core::time::Duration::from_secs(0)
            }
        }
    }

    pub mod hash {
        pub use hashbrown::{HashMap, HashSet, hash_map, hash_set};
        pub type FxHashMap<K, V> = HashMap<K, V, rustc_hash::FxBuildHasher>;
        pub type FxHashSet<V> = HashSet<V, rustc_hash::FxBuildHasher>;
    }
}

pub use r#impl::{backtrace, hash, io, sync, time};
