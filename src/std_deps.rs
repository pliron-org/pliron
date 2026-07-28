// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! All our `std` dependencies are used from re-exports here.
//! Dummy implementations are provided for when `std` is disabled.

#[cfg(feature = "std")]
mod r#impl {
    /// Has pliron been built with `std`?
    pub const STD_ENABLED: bool = true;

    pub mod sync {
        pub use std::sync::{LazyLock, Mutex};
    }

    pub mod backtrace {
        pub use std::backtrace::{Backtrace, BacktraceStatus};
    }

    pub mod path {
        pub use std::path::{Path, PathBuf};
    }

    pub mod io {
        pub use std::io::BufReader;
    }

    pub mod fs {
        pub use std::fs::{File, write};
    }

    pub mod time {
        pub use std::time::Instant;
    }

    pub mod hash {
        pub use rustc_hash::FxHasher;
        pub use std::collections::{HashMap, HashSet, hash_map, hash_set};
        pub type FxHashMap<K, V> = HashMap<K, V, rustc_hash::FxBuildHasher>;
        pub type FxHashSet<V> = HashSet<V, rustc_hash::FxBuildHasher>;
    }

    pub mod utf8_chars {
        pub use utf8_chars::BufReadCharsExt;
    }
}

#[cfg(not(feature = "std"))]
mod r#impl {
    /// Has pliron been built with `std`?
    pub const STD_ENABLED: bool = false;

    pub mod sync {
        pub use spin::{LazyLock, Mutex};
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

    pub mod path {
        use alloc::string::{String, ToString};
        use core::ops::{Deref, DerefMut};

        #[derive(PartialEq, Eq, Clone, Debug, Hash)]
        pub struct PathBuf(String);

        impl PathBuf {
            pub fn join<P: AsRef<str>>(&self, path: P) -> PathBuf {
                let mut s = self.clone();
                s.push('/');
                s.push_str(path.as_ref());
                s
            }

            pub fn display(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for PathBuf {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for PathBuf {
            fn from(value: String) -> Self {
                PathBuf(value)
            }
        }

        impl From<&str> for PathBuf {
            fn from(value: &str) -> Self {
                PathBuf(value.to_string())
            }
        }

        impl From<&Path> for PathBuf {
            fn from(value: &Path) -> Self {
                PathBuf(value.0.to_string())
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

        pub struct Path(str);

        impl Path {
            pub fn display(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for Path {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    }

    pub mod io {
        pub use super::fs::File;

        pub struct BufReader<R>(core::marker::PhantomData<R>);

        impl<R> BufReader<R> {
            pub fn new(_inner: R) -> Self {
                BufReader(core::marker::PhantomData)
            }
        }
    }

    pub mod fs {
        use core::fmt;

        /// A dummy file handle: without `std` there's no filesystem to back it.
        pub struct File;

        /// Filesystem access is unavailable without the `std` feature.
        #[derive(Debug)]
        pub struct NoStdFsError;

        impl fmt::Display for NoStdFsError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("filesystem access is unavailable without the `std` feature")
            }
        }

        impl File {
            pub fn open<P: AsRef<str>>(_path: P) -> Result<File, NoStdFsError> {
                Ok(File)
            }
        }

        pub fn write<P: AsRef<str>, C: AsRef<[u8]>>(
            _path: P,
            _contents: C,
        ) -> Result<(), NoStdFsError> {
            Ok(())
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
        pub use rustc_hash::FxHasher;
        pub type FxHashMap<K, V> = HashMap<K, V, rustc_hash::FxBuildHasher>;
        pub type FxHashSet<V> = HashSet<V, rustc_hash::FxBuildHasher>;
    }

    pub mod utf8_chars {
        use core::fmt;

        #[derive(Debug)]
        pub struct Utf8CharsError;

        impl fmt::Display for Utf8CharsError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("utf8-chars is unavailable without the `std` feature")
            }
        }

        pub trait BufReadCharsExt {
            fn read_char(&mut self) -> Result<Option<char>, Utf8CharsError>;
        }

        impl<R> BufReadCharsExt for super::io::BufReader<R> {
            fn read_char(&mut self) -> Result<Option<char>, Utf8CharsError> {
                Ok(None)
            }
        }
    }
}

pub use r#impl::{STD_ENABLED, backtrace, fs, hash, io, path, sync, time, utf8_chars};
