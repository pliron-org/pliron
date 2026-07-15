#!/bin/bash

# Pre-CI tests to run locally before pushing to GitHub

set -ex

# 1. Format check (Run first: costs zero build time, fails instantly if messy)
cargo fmt --check

# 2. Combined linting and compilation
# (Saves time by compiling your dependencies exactly once for both tools)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Test execution under strict warning flags
# (Reuses the dependencies compiled by clippy because the profile flags match)
RUSTFLAGS="-D warnings" cargo test --workspace

# 4. Fast documentation check
# (Skips external dependency docs entirely to prevent a massive compile cycle)
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
