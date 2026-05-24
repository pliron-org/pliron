# Chapter 3: Lowering the AST to IR

In Chapter 2 we hand-built IR for a specific Fibonacci function. This chapter
shows how to do that systematically for *any* Kaleidoscope program by lowering
the AST produced in Chapter 1 into the dialect ops defined in Chapter 2.

The implementation for this chapter lives in `examples/kaleidoscope/from_ast.rs`

## Design

Every Kaleidoscope value is a 64-bit signless integer (`i64`). The key
design decisions are:

- **All variables are memory-backed.** Every local variable and every function
  parameter is backed by a `DeclOp` slot (analogous to LLVM's `alloca`).
  Reads become `LoadOp`s; writes become `StoreOp`s. This avoids having to
  worry about SSA form.
- **Control flow uses regions.** `IfOp` owns two regions (then / else) and
  `WhileOp` owns one region (the loop body). Each region gets its own new
  `BasicBlock`.
- **`WhileOp` holds a condition pointer.** Rather than threading a boolean
  value as an argument, the condition is stored in a dedicated `DeclOp` slot
  in the outer block and updated at the end of each loop iteration.

## Entry point

The public API is a single function. It takes an AST `Function` node and
produces a `FuncOp` inside the provided `Context`:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_function}}
```

Two type aliases keep the rest of the code concise:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:type_aliases}}
```

`OpInserter` is an `IRInserter` with a no-op listener. It tracks a cursor
position inside a block and appends ops there via `append_op`. `VarMap` maps
source-level variable names to the `Value` (result of a `DeclOp`) that holds
the variable's storage slot.

### Creating the `FuncOp`

`lower_function` first creates an empty `FuncOp` with the right type, then
obtains the entry block and initialises the inserter at its end:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_function}}
```

*(Only the setup and fallback parts are discussed in detail below.)*

### Spilling parameters

Every function parameter arrives as a basic-block argument. To allow
mutation later, each argument is immediately spilled into a `DeclOp` slot:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_function_params}}
```

`entry.deref(ctx).get_argument(idx)` returns the `Value` of the `idx`-th block
argument. `DeclOp::new_with_type` allocates a pointer-typed slot; `StoreOp`
writes the initial value into it.

### Fallback terminator

After lowering the body, the entry block might still lack a terminator. This
can happen when the source function ends with an `if` statement where both
branches contain a `return`. In that case a fallback `return 0` is appended:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_function_fallback}}
```

## Lowering statements

Statements are lowered by two helpers. `lower_stmts` iterates a slice and
delegates each item to `lower_stmt`, short-circuiting on the first error:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmts}}
```

Both functions return `Result<bool>` where `true` means the last op emitted
was a block terminator (`ReturnOp`). This signal is used to suppress
redundant `YieldOp` terminators in `if` / `while` regions.

### Variable declaration

`var name;` or `var name = expr;` allocates a slot, registers it in the
`VarMap`, and optionally stores an initial value:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmt_vardecl}}
```

### Assignment

`name = expr;` looks up the existing slot in `VarMap` and emits a `StoreOp`.
If the variable was never declared, `input_error!` returns a structured error
instead of panicking:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmt_assign}}
```

### Return

`return expr;` lowers the expression, emits `ReturnOp`, and signals that the
block is now terminated:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmt_return}}
```

### If statement

`if cond { then } else { else }` emits an `IfOp`, then builds two new blocks —
one for each region. Each branch is lowered recursively. A `YieldOp` is
appended only when the branch did not end with a `return`:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmt_if}}
```

Note that `IfOp` itself is *not* a block terminator in the outer block, so
`lower_stmt_if` returns `Ok(false)`.

### While loop

`while cond { body }` uses the memory-backed condition pattern described in
the design section above. Before entering the loop, the condition is evaluated
and stored in a fresh `DeclOp` slot. At the end of each iteration, the
condition is re-evaluated and the slot is updated. A `YieldOp` closes the
body region (if it doesn't already end with a `return`):

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmt_while}}
```

### Expression statement

A bare `expr;` lowers the expression for its side effects and discards the
resulting value:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_stmt_expr}}
```

## Lowering expressions

`lower_expr` matches on the `Expr` variant and returns the `Value` that holds
the computed result:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_expr}}
```

### Integer literal

A literal is wrapped in a `ConstantOp`. The result value is returned:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_expr_integer}}
```

### Variable reference

A variable reference looks up the slot in `VarMap` and emits a `LoadOp` to
read its current value. An undeclared variable is reported as an
`InvalidInput` error using the `input_error!` macro:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_expr_variable}}
```

### Binary operation

Both operands are lowered recursively, then a `BinOp` is created with the
appropriate `BinOpKind` attribute:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_expr_binop}}
```

### Function call

Arguments are lowered in order and collected into a `Vec`. A `CallOp` is then
created with the callee name as an `IdentifierAttr`:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_expr_call}}
```

Note the reborrow trick: `call_op.get_operation().deref(ctx)` borrows `ctx`
immutably, so the result `val` must be captured before passing `call_op` to
`append_op`, which borrows `ctx` mutably.

**Note**: The general guideline around calling `deref` or `deref_mut` on `Ptr<T>` is to
keep the borrowed value (`Ref<T>` or `RefMut<T>`) around for as little time as possible,
and to avoid passing it to any function that might call back into the context.

## Error handling

All lowering functions return `pliron::result::Result<T>`. Errors are
propagated with `?`. The `input_error!` macro creates a
`pliron::result::Error` with kind `ErrorKind::InvalidInput` and an optional
source location. Because the Kaleidoscope AST does not carry source positions,
`Location::Unknown` is used throughout.

## Tests

The test module defines a helper that parses a source string, lowers every
function into a fresh module, verifies the IR, and prints it:

```rust
{{#include ../../examples/kaleidoscope/from_ast.rs:lower_test_helper}}
```

## Try it out

```sh
cargo test --example kaleidoscope -- fibonacci_from_ast --show-output
cargo test --example kaleidoscope -- factorial_from_ast --show-output
cargo test --example kaleidoscope -- inline_fibonacci_from_ast --show-output
```

Each test prints the lowered IR so you can inspect how the AST maps to ops.

## Next step

Chapter 4 lowers the Kaleidoscope dialect into a lower-level LLVM dialect.
