#!/bin/bash

# Pre-CI tests to run locally before pushing to GitHub

set -ex

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p pliron-llvm --all-targets --features debug-info -- -D warnings
RUSTFLAGS="-D warnings" cargo test --workspace
RUSTFLAGS="-D warnings" cargo test -p pliron-llvm --features debug-info --test debug_info
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --features pliron-llvm/debug-info
