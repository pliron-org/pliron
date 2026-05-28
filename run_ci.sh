#!/bin/bash

# This script runs the CI checks for this workspace.
#
# It differs from .github/workflows/ci.yml as follows
#   - LLVM setup is not performed.
#   - Cargo commands are not passed the additional `--verbose` flag.

set -e

# Parse command line arguments
WASM_TESTS=false
while [[ $# -gt 0 ]]; do
  case $1 in
  --wasm)
    WASM_TESTS=true
    shift
    ;;
  *)
    echo "Unknown option: $1"
    echo "Usage: $0 [--wasm]"
    exit 1
    ;;
  esac
done

set -x

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="-D warnings" cargo build --workspace
RUSTFLAGS="-D warnings" cargo build -p pliron -p pliron-derive --target wasm32-unknown-unknown
cargo test --workspace
cargo test --release --workspace
if [ "$WASM_TESTS" = true ]; then
  # Check if node is installed
  if ! command -v node &>/dev/null; then
    echo "Error: node is required for WASM tests but is not installed."
    echo "Please install Node.js from https://nodejs.org/ or using your package manager."
    exit 1
  fi

  # Check if wasm-bindgen-test-runner is installed
  if ! command -v wasm-bindgen-test-runner &>/dev/null; then
    echo "Error: wasm-bindgen-test-runner is required for WASM tests but is not installed."
    echo "Please install it by running: cargo install wasm-bindgen-cli"
    exit 1
  fi

  cargo test --target wasm32-unknown-unknown
  cargo test --release --target wasm32-unknown-unknown
fi
RUSTDOCFLAGS="-D warnings" cargo doc --workspace
RUSTFLAGS="-D warnings" cargo build -p pliron-llvm --no-default-features
