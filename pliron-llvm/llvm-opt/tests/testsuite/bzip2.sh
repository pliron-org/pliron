#!/usr/bin/env bash

# Standalone usage:
#   bash tests/testsuite/bzip2.sh
#
# Run from anywhere with explicit tool paths:
#   LLVM_OPT_WRAPPER=/path/to/llvm-opt-wrapper.sh \
#   LLVM_OPT=/path/to/llvm-opt \
#   bash tests/testsuite/bzip2.sh
#
# What this script does:
# - creates an isolated temporary directory,
# - clones bzip2 from sourceware,
# - checks out tag bzip2-1.0.8,
# - builds with CC set to llvm-opt-wrapper,
# - verifies upstream self-test success marker in build output.
#
# Requirements: git, make, a working clang toolchain, llvm-opt-wrapper.sh, llvm-opt.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WRAPPER="${LLVM_OPT_WRAPPER:-$ROOT_DIR/llvm-opt-wrapper.sh}"
LLVM_OPT_BIN="${LLVM_OPT:-$ROOT_DIR/../../target/debug/llvm-opt}"

if [[ ! -x "$WRAPPER" ]]; then
  echo "[bzip2] wrapper not found or not executable: $WRAPPER" >&2
  echo "[bzip2] set LLVM_OPT_WRAPPER=/path/to/llvm-opt-wrapper.sh" >&2
  exit 1
fi

if [[ ! -x "$LLVM_OPT_BIN" ]]; then
  echo "[bzip2] llvm-opt binary not found or not executable: $LLVM_OPT_BIN" >&2
  echo "[bzip2] set LLVM_OPT=/path/to/llvm-opt" >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "[bzip2] git is required" >&2
  exit 1
fi

if ! command -v make >/dev/null 2>&1; then
  echo "[bzip2] make is required" >&2
  exit 1
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/llvm-opt-tests-bzip2.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT

echo "[bzip2] using temporary directory: $workdir"
echo "[bzip2] cloning sourceware bzip2"

git clone git://sourceware.org/git/bzip2.git "$workdir/bzip2" >/dev/null 2>&1
cd "$workdir/bzip2"

git checkout bzip2-1.0.8 >/dev/null 2>&1

echo "[bzip2] building with wrapper"

build_log="$workdir/bzip2-make.log"
if ! make \
  CC="$WRAPPER" \
  LLVM_OPT="$LLVM_OPT_BIN" \
  -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" \
  2>&1 | tee "$build_log"; then
  echo "[bzip2] build failed; tail of log:" >&2
  tail -n 80 "$build_log" >&2 || true
  exit 1
fi

# The upstream Makefile runs compression self-tests and prints this on success.
if ! grep -F "you're in business" "$build_log" >/dev/null 2>&1; then
  echo "[bzip2] self-test success marker not found in build output" >&2
  echo "[bzip2] expected phrase: you're in business" >&2
  tail -n 120 "$build_log" >&2 || true
  exit 1
fi

echo "[bzip2] PASS"
