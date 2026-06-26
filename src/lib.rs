//! # pliron: Programming Languages Intermediate RepresentatiON
//!
//! `pliron` is an extensible compiler IR framework in Rust, inspired by MLIR.

#![no_std]

// Allow proc-macros to find this crate
extern crate self as pliron;

// Export pliron_derive as pliron::derive for procedural macros.
pub use pliron_derive as derive;

pub extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

// Export linkme as pliron::linkme for procedural macros.
// This re-export is tricky, and we use the workaround here:
// https://github.com/dtolnay/linkme/issues/108#issuecomment-3308031385
#[cfg(not(target_family = "wasm"))]
pub use linkme;
// Export combine as pliron::combine for procedural macros.
pub use combine;
// Export log as pliron::log for procedural macros.
pub use log;
// Export dyn_clone as pliron::dyn_clone for procedural macros.
pub use dyn_clone;
// Export inventory as pliron::inventory for procedural macros.
#[cfg(target_family = "wasm")]
pub use inventory;

pub mod analyses;
pub mod attribute;
pub mod basic_block;
pub mod builtin;
pub mod common_traits;
pub mod context;
pub mod debug_info;
pub mod dialect;
pub mod graph;
pub mod identifier;
pub mod irbuild;
pub mod irfmt;
pub mod linked_list;
pub mod location;
pub mod op;
pub mod operation;
pub mod opts;
pub mod parsable;
pub mod pass;
pub mod printable;
pub mod region;
pub mod result;
pub mod storage_uniquer;
pub mod symbol_table;
pub mod r#type;
pub mod uniqued_any;
pub mod utils;
pub mod value;

pub mod deps;

/// A macro to initialize `env_logger` for tests. Default logger level is set to "off".
/// It sets `env_logger`'s test mode so that logs are captured by the test framework.
///
/// This is a macro because we don't want [pliron] to depend on `env_logger` directly,
/// and we want to allow users to choose their own logging framework if they want.
#[macro_export]
macro_rules! init_env_logger_for_tests {
    () => {{
        // The default logger level is "off".
        let mut builder =
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("off"));
        // WASM target doesn't support timestamps.
        if cfg!(target_family = "wasm") {
            let _ = builder.is_test(true).format_timestamp(None).try_init();
        } else {
            let _ = builder.is_test(true).try_init();
        }
    }};
}
