# Chapter 2: The Kaleidoscope Dialect

In this chapter, we'll learn some of `pliron`'s core IR concepts
by defining a dialect for the Kaleidoscope language. As new IR
concepts are introduced, you may find it useful to refer to the
`pliron` [documentation](https://docs.rs/pliron/latest/pliron/)
for details and examples.

The implementation for this chapter lives in `examples/kaleidoscope/dialect.rs`.

## What is a Dialect?

`pliron` is an extensible compiler IR framework. This means that it does not
restrict you to a fixed set of operations (or instructions) or types.
Instead, you can define your own operations and types that capture the
semantics of your source language.

A dialect is merely a grouping of operations, types, and attributes that define
the IR for a particular domain, language or purpose. It serves as the foundation
for representing and manipulating programs in that domain.

## IR Entities / Structure

Every IR entity (such as an operation or type) is owned by a `Context`.
Almost every method in the framework requires a `Context` argument to
either look up existing entities or create new ones.

An `Operation` is a basic unit of execution. It has operands (or input values),
results (or output values), attributes (or metadata), and may own regions.

A sequential list of operations forms a `BasicBlock`. At the end of a block,
a terminator operation (like `ReturnOp` or `BranchOp`) indicates the end
of the sequence and a possible transfer of control. Basic-blocks can have
one or more arguments.

`Operation` results and `BasicBlock` arguments are
[SSA](https://en.wikipedia.org/wiki/Static_single_assignment_form) values.
`BasicBlock` arguments are an alternative (but equivalent and more convenient)
form of SSA ϕ-nodes.

The control-flow-graph (i.e., a directed graph of `BasicBlock`s) is contained
within a `Region`. The `Region` itself is owned by a parent operation.
For example, an `IfOp` owns two regions: one for the "then" block and one for
the "else" block.

These three entities (`Operation`, `BasicBlock`, `Region`) form the core structural IR
elements. They are the building blocks for representing programs in `pliron`.

All of these entities are owned by a `Context`, and are handled in the IR
through a `Ptr` (e.g., `Ptr<Operation>`, `Ptr<Region>`, etc.). A `Ptr`
can be dereferenced to access the underlying entity using the `deref`
and `deref_mut` methods, which return the underlying `Ref` or `RefMut`
of the entity. See [interior mutability](https://doc.rust-lang.org/book/ch15-05-interior-mutability.html).

## Types and Attributes

Every value (`BasicBlock` operand or `Operation` result) has a type.
Types are first-class IR entities that can be defined by the user.
For example, `IntegerType` in the builtin dialect represents integers.
Dialects can define new types as needed.

Attributes allow attaching arbitrary metadata to operations. For example,
in a `ConstantOp`, the constant value is stored as an attribute (e.g., `IntegerAttr`).

## Ops

Operations are the core IR entities that represent instructions or statements in the program.
Ops are thin wrappers around the underlying `Operation` struct, providing "opcode"
specific APIs and invariants. For example, a `BinOp` represents a binary operation (like addition)
and has specific attributes to indicate the kind of binary operation (e.g., `BinOpKind::Add`).
You can imagine an `Op` to look like:

```
struct MyOp {
    op: Ptr<Operation>
}

impl MyOp {
  ...
}
```

## Defining The Kaleidoscope Dialect

Let's dive in now and see how we can define a `ConstantOp`.

### `ConstantOp`: A Simple Example

The constant operation represents a literal integer value in the IR.
It has no operands and one result (the constant value).
The constant value is stored as an attribute of the operation.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:constant_op_decl}}
```

We define `ConstantOp` as simply an empty struct, annotated with the `pliron_op`
macro. In addition to adding the `op: Ptr<Operation>` field (that we saw above),
this macro fills up boilerplate code required for implementing pliron's
`Op` trait.

The `pliron_op` macro has one mandatory argument: A string specifying the dialect
and opcode name (e.g., `"kaleidoscope.constant"`).

We then specify, using the `attributes` field in the macro, that this operation has
one attribute named `value` of type `IntegerAttr`.

In this example, we're also specifying the `format` for the `Op`. This defines
the syntax for the operation when printed as IR. The full specification for the format
syntax string is available in the docs. In this case, the format string says that
the attribute (named `value`) containing the constant value (as an `IntegerAttr`)
should be printed first, followed by the result type (after a colon).

Defining a syntax amounts to automatically deriving the `Printable` and `Parsable`
traits for the operation, which are used by pliron's IR printer and parser respectively.

The `format` field can have its string argument skipped, in which case a canonical
(default) syntax is used.

Next, we have the `interfaces` field in the macro. Interfaces are Rust
traits that are annotated with the `#[op_interface]` macro. Interfaces allow
capturing common behaviour and invariants across multiple operations. 
In this case, `NOpdsInterface<0>` says that `ConstantOp` has zero operands,
and `OneResultInterface` says that it has exactly one result.

And finally we have the `verifier` field, which specifies the verifier function for
this operation. The verifier is responsible for enforcing invariants not captured
by the interfaces that this Op implements. In this case, we can use the built-in `succ`
verifier, which simply returns `Ok(())` to indicate that the operation is always valid.
For more complex invariants, you can write a custom verifier function by implementing
the `Verify` trait for the Op.

*Note*: We have not defined `IntegerAttr` in this dialect. We are using the `IntegerAttr`
defined in the builtin dialect. This is an important point: The IR is not restricted
to using only the operations, types, and attributes defined in a single dialect. You can
mix and match entities from different dialects as needed.

### DeclOp: Variable Declarations

A `DeclOp` returns a pointer-typed result while storing source-level variable type as a `TypeAttr`.
This represents local allocation of variables. (Analogous to LLVM's `AllocaInst`)
The `PointerType` is taken from pliron's LLVM dialect.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:decl_op_decl}}
```

### BinOp: Binary Operations

`BinOp` has two operands (the left-hand side and right-hand side of the binary operation)
and one result (the computed value). The specific binary operation (e.g., add, subtract,
multiply, divide) is captured as an attribute (`BinOpKind`).

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:binop_decl}}
```

Here we show how one can define convenience methods on the typed wrapper (Op):

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:binop_methods}}
```

### CallOp

The `CallOp` represents a function call. It has a variable number of operands (the arguments to the function) and one result (the return value of the function). The callee function is captured as an attribute (an `IdentifierAttr`).

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:call_op_decl}}
```

### Regions and Blocks

Structured control flow is represented through regions and blocks:

#### IfOp

The `IfOp` has two regions: one for the "then" block and one for the "else" block. 
Each region here contains a single block (captured by the `SingleBlockRegionInterface`),
which in turn contains a sequence of operations.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:if_op_decl}}
```

#### WhileOp

The `WhileOp` has a single region capturing the loop body. The loop condition is represented
as an operand of the op. Here we define the semantics of the condition to be a pointer
(to an integer value), captured by the `OperandNOfType<0, PointerType>` interface specification,
where a non-zero value (of the pointee) is considered true and zero is false.
The operand being a pointer allows the loop condition to be updated within the loop body.
Similar to `DeclOp`, the pointer type is taken from pliron's LLVM dialect.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:while_op_decl}}
```

### `YieldOp`
The `YieldOp` serves as a terminator for the body region of `WhileOp` and `IfOp`.
It has no operands or results, and simply indicates the end of the structured region.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:yield_op_decl}}
```

### `ReturnOp`
The `ReturnOp` serves as a terminator from a function body region.
It has one operand, which is the value being returned from the function.

To illustrate the use of a custom verifier, `ReturnOp` implements the
`Verify` trait to enforce the invariant that it must have exactly one operand
(this could also have been enforced declaratively using `NOpdsInterface<1>`).

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:return_op_decl}}
```

The custom verifier:

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:return_op_manual_verifier}}
```

## Construction Example

In this section, we manually construct IR, using pliron APIs and utilities
for a specific simple Fibonacci function. This is to illustrate how the IR is built up from
the basic entities (operations, blocks, regions) and how the dialect ops are used in practice.
The next chapter shows how to do this systematically by translating (lowering) the AST into
this IR.

Pseudocode in Kaleidoscope for the function being generated:

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:fib_build_pseudocode}}
```

Rather than reading the entire construction test as one block, it is easier to
follow it in the same order as the IR is assembled.

### 1. Set up the context, module, and entry block

First we create the `Context`, define the function type, create a module and a
`main` function, and position an inserter at the end of the entry block.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:fib_build_setup}}
```

At this point we have the outer container structure in place, but the function
body is still empty.

### 2. Allocate mutable slots for the variables

The Fibonacci implementation uses mutable local state (`a`, `b`, `tmp`, `i`,
`n`, and the loop-condition slot), so the next step is to create `DeclOp`s for
each one.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:fib_build_slots}}
```

Notice that we also attach result names such as `a` and `b`. These names are
not required for correctness, but they make the printed IR much easier to read.

### 3. Initialize the state and create the loop shell

Next we materialize the constants, store the initial values into the mutable
slots, compute the initial loop condition `i < n`, and build the `WhileOp`
itself.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:fib_build_init_and_cond}}
```

The important detail here is that `WhileOp` takes a pointer to the condition
slot, not the condition value directly. That lets the loop body recompute and
update the condition on every iteration.

### 4. Fill in the loop body region

Once the `WhileOp` exists, we create its body block and populate it with the
actual Fibonacci update logic.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:fib_build_while_body}}
```

This block mirrors the source program closely: compute `tmp = a + b`, rotate
`a` and `b`, increment `i`, recompute the loop condition, then end the region
with `YieldOp`.

### 5. Return the result and verify the IR

After the loop, the function loads the final value of `b`, returns it, and runs
verification on the constructed module.

```rust
{{#include ../../examples/kaleidoscope/dialect.rs:fib_build_return}}
```

This last step is worth keeping in mind whenever you construct IR manually:
verification is the cheapest way to catch structural mistakes early.

## Try it out

```sh
cargo test --example kaleidoscope -- build_fib_example --show-output
```

This prints Kaleidoscope IR for the Fibonacci function shown above. We suggest
studying the test and the printed IR together to understand how the dialect ops
are used to construct the IR.

Exercise: Try adding a test to build the factorial function similarly and print its IR.

## Next step

In this chapter we manually built the IR for a specific Kaleidoscope program.
Chapter 3 shows how to lower the AST into this IR, which means we can parse
any Kaleidoscope program into an AST and then lower it into IR.
