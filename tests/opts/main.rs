//! Test optimizations

// We use pliron-llvm in this test, which is not supported in wasm.
#![cfg(not(target_family = "wasm"))]

mod dce;
mod mem2reg;
mod sccp;
