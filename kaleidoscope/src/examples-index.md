# Examples Index

This index maps tutorial chapters to runnable examples.

## Chapter mapping

| Chapter | Focus | Tests |
|---|---|---|
| Chapter 1 | Parser | `ast::tests::test_ast_fibonacci`<br>`ast::tests::test_ast_factorial` |
| Chapter 2 | Dialect definitions and manual IR construction | `dialect::tests::build_fib_example` |
| Chapter 3 | AST to Kaleidoscope dialect lowering | `from_ast::tests::fibonacci_from_ast`<br>`from_ast::tests::factorial_from_ast`<br>`from_ast::tests::inline_fibonacci_from_ast` |
| Chapter 4 | Kaleidoscope dialect to LLVM dialect lowering | `to_llvm::tests::fibonacci_to_llvm`<br>`to_llvm::tests::factorial_to_llvm`<br>`to_llvm::tests::if_else_to_llvm` |
| Chapter 5 | JIT execution | `jit::tests::fibonacci_jit`<br>`jit::tests::factorial_jit`<br>`jit::tests::if_else_jit` |

## Running chapter tests

```sh
cargo test --example kaleidoscope -- --list
cargo test --example kaleidoscope -- --show-output
```

Run a single test by name:

```sh
cargo test --example kaleidoscope -- fibonacci_to_llvm --show-output
```

Run the end-to-end CLI example:

```sh
cargo run --example kaleidoscope -- --input examples/kaleidoscope/fibonacci.kal --fn main --arg 5
```

## Testing strategy

As examples become implementation-heavy, mirror key behaviors in integration tests under `tests/` so chapter claims remain verifiable in CI.

## Suggested learning paths

- Parser-focused: start at Chapter 1, then Chapter 3.
- IR design-focused: start at Chapter 2, then Chapter 3 and Chapter 4.
- Execution-focused: skim Chapters 1-2, then Chapter 5.
