#!/usr/bin/env bash

# Standalone usage:
#   bash tests/testsuite/lua.sh
#
# Run from anywhere with explicit tool paths:
#   LLVM_OPT_WRAPPER=/path/to/llvm-opt-wrapper.sh \
#   LLVM_OPT=/path/to/llvm-opt \
#   bash tests/testsuite/lua.sh
#
# What this script does:
# - creates an isolated temporary directory,
# - clones lua from GitHub,
# - checks out tag v5.5.0,
# - builds with CC set to llvm-opt-wrapper,
# - runs the upstream Lua test suite and verifies its success marker.
#
# Requirements: git, make, a working clang toolchain, llvm-opt-wrapper.sh, llvm-opt.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WRAPPER="${LLVM_OPT_WRAPPER:-$ROOT_DIR/llvm-opt-wrapper.sh}"
LLVM_OPT_BIN="${LLVM_OPT:-$ROOT_DIR/../../target/debug/llvm-opt}"

if [[ ! -x "$WRAPPER" ]]; then
  echo "[lua] wrapper not found or not executable: $WRAPPER" >&2
  echo "[lua] set LLVM_OPT_WRAPPER=/path/to/llvm-opt-wrapper.sh" >&2
  exit 1
fi

if [[ ! -x "$LLVM_OPT_BIN" ]]; then
  echo "[lua] llvm-opt binary not found or not executable: $LLVM_OPT_BIN" >&2
  echo "[lua] set LLVM_OPT=/path/to/llvm-opt" >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "[lua] git is required" >&2
  exit 1
fi

if ! command -v make >/dev/null 2>&1; then
  echo "[lua] make is required" >&2
  exit 1
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/llvm-opt-tests-lua.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT

echo "[lua] using temporary directory: $workdir"
echo "[lua] cloning lua"

git clone https://github.com/lua/lua.git "$workdir/lua" >/dev/null 2>&1
cd "$workdir/lua"

git checkout v5.5.0 >/dev/null 2>&1

echo "[lua] building with wrapper"

build_log="$workdir/lua-make.log"
if ! make \
  CC="$WRAPPER" \
  LLVM_OPT="$LLVM_OPT_BIN" \
  -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" \
  2>&1 | tee "$build_log"; then
  echo "[lua] build failed; tail of log:" >&2
  tail -n 80 "$build_log" >&2 || true
  exit 1
fi

echo "[lua] running upstream test suite"

test_log="$workdir/lua-test.log"
if ! (cd testes && "$workdir/lua/lua" -e "_U=true" all.lua) >"$test_log" 2>&1; then
  echo "[lua] test suite run failed; tail of log:" >&2
  tail -n 80 "$test_log" >&2 || true
  exit 1
fi

# The upstream test suite prints this on success.
if ! grep -F "final OK !!!" "$test_log" >/dev/null 2>&1; then
  echo "[lua] self-test success marker not found in test output" >&2
  echo "[lua] expected phrase: final OK !!!" >&2
  tail -n 120 "$test_log" >&2 || true
  exit 1
fi

echo "[lua] PASS"
