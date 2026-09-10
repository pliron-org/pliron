# Real-world wrapper tests

Each script:
- runs in an isolated temporary directory,
- builds a real C program using llvm-opt-wrapper.sh as CC,
- validates an expected success marker,
- returns non-zero on failure.

## Currently available tests:
- bzip2.sh: clones bzip2, checks out bzip2-1.0.8, runs make with wrapper,
  and verifies upstream self-tests succeeded.
- lua.sh: clones lua, checks out v5.5.0, runs make with wrapper,
  and verifies the upstream test suite succeeded.

## Runner
- `../testsuite.sh` discovers and executes all *.sh scripts in this directory.

## Environment variables:
- LLVM_OPT_WRAPPER: path to llvm-opt-wrapper.sh
  default: pliron-llvm/llvm-opt/llvm-opt-wrapper.sh
- LLVM_OPT: path to llvm-opt binary
  default: target/debug/llvm-opt

## Liveness benchmark corpus

Set `PLIRON_LIVENESS_BENCH_DIR` while running a real-world wrapper test to preserve
the unoptimized LLVM modules emitted by clang. The liveness benchmark consumes all
`.ll` files found recursively in that directory. It performs one exhaustive
value/program-point query sweep per module and reports tab-separated timing data.
A module that triggers a liveness panic is reported and skipped so the remaining
corpus can still be measured.

Example using bzip2:

```bash
rm -rf /tmp/pliron-liveness-bzip2
mkdir -p /tmp/pliron-liveness-bzip2
cargo build -p llvm-opt
PLIRON_LIVENESS_BENCH_DIR=/tmp/pliron-liveness-bzip2 \
  bash pliron-llvm/llvm-opt/tests/testsuite/bzip2.sh
PLIRON_LIVENESS_BENCH_DIR=/tmp/pliron-liveness-bzip2 \
  cargo bench -p llvm-opt --bench liveness
```
