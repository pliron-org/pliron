# Chapter 4: Lowering to the LLVM Dialect

In Chapter 3 we produced IR in the Kaleidoscope dialect — operations like
`kaleidoscope.if`, `kaleidoscope.binop`, and `kaleidoscope.while` that still
carry high-level, structured semantics.  This chapter converts that IR into
the LLVM dialect, where all control flow is flat (basic blocks + branches) and
every op corresponds closely to an LLVM instruction.

The implementation for this chapter lives in `examples/kaleidoscope/to_llvm.rs`.

## Design: Dialect Conversion

pliron ships a *dialect conversion* framework in
`pliron::irbuild::dialect_conversion`.  It is deliberately simpler than
MLIR's equivalent:

- **No unrealized conversion casts.** Types that do not need conversion are
  kept as-is.
- **Definitions before uses.** The framework guarantees that when an op's
  `rewrite` callback fires, all operands have already been converted.
- **Each op rewrites itself.** Rather than a monolithic pattern-match switch,
  each op implements (in this case) the `ToLLVMDialect` interface from `pliron_llvm`.

The three moving parts are:

| Piece | Responsibility |
|---|---|
| `DialectConversion` trait | Decides which ops to convert and calls the per-op rewrite |
| `DialectConversionRewriter` | Wraps `IRRewriter` and records all mutations |
| `apply_dialect_conversion` | Drives the worklist, ensures def-before-use order |

### The `DialectConversion` trait

```rust
pub trait DialectConversion {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool;
    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()>;
    // optional: can_convert_type, convert_type
}
```

`can_convert_op` is the filter.  `rewrite` is called for every op that
passes the filter, with the insertion point already positioned *before* the op.

### `OperandsInfo`

The `operands_info` parameter gives each rewrite access to the history of
type changes its operands went through during the conversion pass.  For the
Kaleidoscope lowering we do not convert types (everything stays `i64`), so
this is left unused in most rewrites.

### `apply_dialect_conversion`

```rust
pub fn apply_dialect_conversion<C: DialectConversion>(
    ctx: &mut Context,
    conversion: &mut C,
    op: Ptr<Operation>,
) -> Result<()>
```

The algorithm walks the IR tree rooted at `op`, collecting convertible ops into a worklist.
It then repeatedly pops from the worklist, and ensures to process operand-defining ops first.
 After each `rewrite`, recorded mutations are inspected: erased ops are dropped from the worklist,
newly inserted ops are added (if required), and block-argument types are updated if any successor
references changed.

## The conversion driver: `KalToLLVM`

Our driver is a zero-field struct that implements `DialectConversion`:

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:kal_to_llvm_driver}}
```

`op_impls::<dyn ToLLVMDialect>` checks whether the runtime type of the op
object implements the `ToLLVMDialect` interface.  Most conversions go through
that path: `op_cast` downcasts `dyn Op` to `dyn ToLLVMDialect` and dispatches
to the op's own `rewrite` method.

There is one explicit special case: `builtin.func` is matched and rewritten to
`llvm.func` in the driver itself.  We do this because `builtin.func` is not a
Kaleidoscope op and does not implement `ToLLVMDialect`, but we still need to
convert function signatures and block-argument types before lowering function
bodies.

## Entry point

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:lower_module}}
```

The module is modified *in place*: `builtin.module` and `builtin.func` are
kept as-is because they do not implement `ToLLVMDialect`.  Only the
Kaleidoscope ops inside the function bodies are converted.

## Lowering simple ops

### `kaleidoscope.constant` -> `llvm.constant`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:constant_to_llvm}}
```

`rewriter.insert_op` inserts the new op at the current insertion point (before
the op being replaced).  `replace_operation_with_values` replaces every use of
the original result with the new result and then erases the original op.

### `kaleidoscope.decl` -> `llvm.alloca`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:decl_to_llvm}}
```

`DeclOp` allocates a variable slot.  The LLVM equivalent is `alloca elem_type,
i32 1`.  The `alloca` instruction requires an *i32* count, so a separate
`llvm.constant 1 : i32` is inserted first.

### `kaleidoscope.load` -> `llvm.load`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:load_to_llvm}}
```

### `kaleidoscope.store` -> `llvm.store`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:store_to_llvm}}
```

`StoreOp` has no result, so we call `erase_operation` directly instead of
`replace_operation_with_values`.

### `kaleidoscope.binop` -> `llvm.add` / `sub` / `mul` / `icmp` + `sext`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:binop_to_llvm}}
```

Arithmetic ops are straightforward one-for-one translations.  Comparison ops
are two-step: `llvm.icmp` produces an `i1` (1-bit boolean), which is then
sign-extended to `i64` by `llvm.sext` so the result type is consistent with
the rest of the Kaleidoscope value universe.

### `kaleidoscope.call` -> `llvm.call`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:call_to_llvm}}
```

`llvm.call` requires a `FuncType` at the call site.  Because every
Kaleidoscope function takes and returns `i64`, the callee type is always
`(i64, …) -> i64` regardless of which function is being called.

### `kaleidoscope.return` -> `llvm.return`

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:return_to_llvm}}
```

## Lowering control flow

Control flow ops are the most interesting because they carry nested regions
that must be flattened into LLVM's flat CFG.  pliron's
`DialectConversionRewriter` provides the necessary primitives:

| Method | Effect |
|---|---|
| `split_block(ctx, block, BeforeOperation(op))` | Moves ops from `op` onwards into a fresh successor block; returns the new block |
| `inline_region(ctx, region, AfterBlock(block))` | Moves all blocks from `region` into the parent function, inserting after `block` |
| `create_block(ctx, AfterBlock(b), label, arg_types)` | Creates a new empty block and inserts it |
| `set_insertion_point(AtBlockEnd(b))` | Moves the insertion cursor to the end of `b` |

### `kaleidoscope.if` -> conditional CFG

Before conversion:

```
^entry:
  ... (IfOp with then-region and else-region) ...
  ... (rest of the function) ...
```

After conversion:

```
^entry:          ; everything up to IfOp + icmp + cond_br
^then_block:     ; inlined from then-region, ends with br ^merge
^else_block:     ; inlined from else-region, ends with br ^merge
^merge:          ; rest of the function (from split_block)
```

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:if_to_llvm}}
```

The key steps:
1. `split_block` cuts the current block at the `IfOp`, creating `merge_block`
   with everything that came after.
2. `icmp ne cond, 0` converts the `i64` condition to `i1`.
3. `cond_br cmp_i1, then_entry, else_entry` terminates `pre_if_block`.
4. The `YieldOp` terminators in both branches are replaced with `br merge_block`.
5. `inline_region` moves the then- and else-blocks into the enclosing function.
6. The `IfOp` shell (which now has empty regions) is erased.

### `kaleidoscope.while` -> loop CFG

Before conversion:

```
^entry:
  ... (WhileOp with body-region containing cond updates) ...
  ... (rest of the function) ...
```

After conversion:

```
^entry:          ; everything up to WhileOp + br ^header
^while_header:   ; load cond, icmp ne cond, 0, cond_br ^body / ^exit
^body_block:     ; inlined from body-region, ends with br ^header (back-edge)
^exit:           ; rest of the function (from split_block)
```

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:while_to_llvm}}
```

`WhileOp` uses the memory-backed condition pattern introduced in Chapter 3:
the loop condition variable is a `DeclOp` slot in the outer block.  The header
loads that slot on each iteration and branches accordingly.  The body region
updates the slot at the end of every iteration, then its `YieldOp` is replaced
with a back-edge branch to the header.

## Tests

The test helper parses a Kaleidoscope source string, lowers it to the
Kaleidoscope dialect, applies the LLVM lowering in place, then prints and
returns the resulting IR:

```rust
{{#include ../../examples/kaleidoscope/to_llvm.rs:lower_to_llvm_test_helper}}
```

Try it out:

```sh
cargo test --example kaleidoscope -- --show-output fibonacci_to_llvm
cargo test --example kaleidoscope -- --show-output factorial_to_llvm
cargo test --example kaleidoscope -- --show-output if_else_to_llvm
```

## Next step

Chapter 5 feeds the LLVM-dialect module into `pliron-llvm`'s JIT backend to
compile and execute the Kaleidoscope program.

