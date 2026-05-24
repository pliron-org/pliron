# Chapter 5: JIT with LLVM

Chapter 4 lowered Kaleidoscope IR into the LLVM dialect. In this chapter we
take the final step: convert that module to LLVM IR, JIT compile it, and invoke
the generated machine code directly from Rust.

The implementation for this chapter lives in `examples/kaleidoscope/jit.rs`.

## Design

The flow is intentionally small and linear:

1. Parse Kaleidoscope source into AST (`parse_program`).
2. Lower each AST function into Kaleidoscope dialect (`lower_function`).
3. Lower the module to LLVM dialect (`lower_module`).
4. Convert LLVM dialect to LLVM IR (`to_llvm_ir::convert_module`).
5. Build an LLVM ORC JIT instance, add the module, look up a symbol, execute.

The one extra safeguard in this implementation is a runtime signature check.
Before JIT invocation, we verify that the target function really has signature
`fn(i64) -> i64`. This limitation is intentionally set for simplicity.

## Step 1-4: Lower source all the way to LLVM IR

`lower_to_llvm_ir` is a helper that runs the complete frontend and lowering
pipeline and returns an `LLVMModule`:

```rust
{{#include ../../examples/kaleidoscope/jit.rs:lower_to_llvm_ir}}
```

Notes:

- We build a fresh `Context` and `ModuleOp` per call.
- Every parsed function is lowered and appended to the module.
- `lower_module` mutates the module in place from Kaleidoscope dialect to LLVM dialect.
- `to_llvm_ir::convert_module` produces real LLVM IR (`LLVMModule`) consumable by JIT.

## Step 5: JIT compile and execute

The public API for execution is `exec_fn`:

```rust
{{#include ../../examples/kaleidoscope/jit.rs:exec_fn}}
```

Key parts of `exec_fn`:

- `initialize_native()` sets up the host target backend once per process.
- `llvm_get_named_function` checks that the requested symbol exists in the generated module.
- `llvm_get_param_types` and `llvm_get_return_type` are used to validate the function ABI as exactly one `i64` argument and an `i64` return value.
- `LLVMLLJIT::new_with_default_builder()` creates an ORC JIT instance.
- `add_module` loads the generated LLVM module into JIT.
- `lookup_symbol(name)` resolves the function entry address.
- `std::mem::transmute` casts the raw symbol address to `extern "C" fn(i64) -> i64`, then calls it.

If the function is missing, or has a different signature, `exec_fn` returns a
structured `pliron::result::Error` instead of panicking.

## Tests

The file includes three focused tests that exercise the end-to-end JIT path.

Fibonacci from source file:

```rust
{{#include ../../examples/kaleidoscope/jit.rs:fibonacci_jit_test}}
```

Factorial from source file:

```rust
{{#include ../../examples/kaleidoscope/jit.rs:factorial_jit_test}}
```

Inline `if/else` program:

```rust
{{#include ../../examples/kaleidoscope/jit.rs:if_else_jit_test}}
```

These tests verify that:

- file-backed programs run correctly through JIT,
- recursive calls work (`fib`, `factorial`),
- structured control flow (`if/else`) lowers and executes correctly.

## Try it out

Run individual JIT tests:

```sh
cargo test --example kaleidoscope -- --show-output fibonacci_jit
cargo test --example kaleidoscope -- --show-output factorial_jit
cargo test --example kaleidoscope -- --show-output if_else_jit
```

You can also run the example binary in this workspace and execute a
chosen function from a `.kal` source file.

## Running via the example CLI

The example binary in `examples/kaleidoscope/main.rs` is a thin wrapper around
`jit::exec_fn`: it reads source from `--input`, selects a function with
`--fn`, passes an integer argument via `--arg`, and prints the result.

Run Fibonacci's `main(5)`:

```sh
cargo run --example kaleidoscope -- --input examples/kaleidoscope/fibonacci.kal --fn main --arg 5
```

Run Factorial's `main(5)`:

```sh
cargo run --example kaleidoscope -- --input examples/kaleidoscope/factorial.kal --fn main --arg 5
```

If the function name is missing from the module, or does not match the
expected `fn(i64) -> i64` signature, the CLI reports the error returned by
`exec_fn`.

## Recap

At this point, the tutorial pipeline is complete:

1. Parse source text into AST.
2. Lower AST to a custom high-level dialect.
3. Lower to LLVM dialect.
4. Convert to LLVM IR.
5. JIT and execute native code.

This is the minimal but complete compiler path from source language to running
machine code using `pliron` + `pliron-llvm`.
