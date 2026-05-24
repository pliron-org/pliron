# Kaleidoscope Example

This directory contains the runnable Kaleidoscope compiler example used throughout
the Kaleidoscope tutorial. It is part of the tutorial's chapter-by-chapter
learning path, but it can also be read and used independently as a small
end-to-end compiler/JIT pipeline example built using pliron.

If you only want to run the example, you do not need to read the tutorial in order.
The code in this directory is organized so that `main.rs` wires together the parser,
AST lowering, LLVM dialect conversion, and JIT execution.

Each file also has unit tests that verify the behavior of its components in isolation.
You can run these tests to understand the expected behavior of each piece, and to
experiment with changes.

## What this example does

The example accepts a Kaleidoscope source file, lowers it through pliron's internal IR, and JIT-executes a chosen function.

The main entry point is [`main.rs`](main.rs), which uses these modules:

- [`ast.rs`](ast.rs): parser and AST definitions for the Kaleidoscope language.
- [`dialect.rs`](dialect.rs): the Kaleidoscope dialect definitions.
- [`from_ast.rs`](from_ast.rs): lowering from the AST into the dialect.
- [`to_llvm.rs`](to_llvm.rs): conversion from the dialect into the LLVM dialect.
- [`jit.rs`](jit.rs): JIT execution and host-side glue.

## Use it independently

From the repository root, run:

```sh
cargo run --example kaleidoscope -- --input examples/kaleidoscope/fibonacci.kal --fn main --arg 5
```

You can swap in any other Kaleidoscope source file that follows the same language subset.
For example, the repository includes:

- [`factorial.kal`](factorial.kal)
- [`fibonacci.kal`](fibonacci.kal)

A minimal independent workflow is:

1. Write a Kaleidoscope program in a `.kal` file.
2. Pick the function you want to execute with `--fn`.
3. Pass a single integer argument with `--arg`.
4. Run the example with `cargo run --example kaleidoscope -- --input <file> --fn <name> --arg <value>`.

For example, if your program defines `main(x)`, you could run:

```sh
cargo run --example kaleidoscope -- --input path/to/program.kal --fn main --arg 42
```

*Note*: We currently only support a single integer argument and single result, for simplicity.
The example can be extended (as an exercise) to support more complex inputs if desired.

## Working through the tutorial

The tutorial chapters in `kaleidoscope/src/` explain the same pipeline in smaller
steps. Reading them in order is helpful if you want to understand how the parser,
lowering, and JIT pieces fit together, but it is not required to run or experiment
with the example.
