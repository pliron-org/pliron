# pliron_lints

[Dylint](https://github.com/trailofbits/dylint) lints for pliron and code built on it.

## Lints

The examples below are for illustration. Each lint is documented in its module.

`ir_borrow_with_context` (warn): a function takes a borrow of an IR object and also a
`Context`, so the object stays borrowed for the full call.

```rust
fn visit(op: &Operation, ctx: &Context) { /* ... */ }
```

`ir_ref_in_scrutinee` (warn): a `Ref` temporary in a `for`, `match`, `if let` or `while let`
header stays alive during a body that uses the `Context`.

```rust
for op in block.deref(ctx).iter(ctx) {
    process_op(op, ctx);
}
```

`ir_entity_in_type_or_attr` (deny): a type or an attribute holds an `Operation`, a
`BasicBlock` or a `Region`.

```rust
#[pliron_attr(name = "test.target", format)]
struct TargetAttr {
    target: Ptr<Operation>,
}
```

`explicit_op_construction` (deny), `self_op_construction` (warn) and
`new_op_outside_constructor` (warn): an `Op` is constructed incorrectly. A constructor of the
`Op` must use `Operation::new_op`. Other code must use the constructor or `Operation::get_op`.

```rust
// explicit_op_construction
let my_op = MyOp { op };
// self_op_construction
Self { op }
// new_op_outside_constructor
let my_op = Operation::new_op::<MyOp>(ctx, vec![], vec![], vec![], 0);
```

`op_layout` (deny): an `Op` type is not `#[repr(transparent)] struct { op: Ptr<Operation> }`.

```rust
struct MyOp {
    op: Ptr<Operation>,
    extra: u32,
}
```

`disp_in_printable` (warn): `disp` in an `impl Printable` prints with a default `State`
instead of the `State` of `fmt`.

```rust
fn fmt(&self, ctx: &Context, state: &State, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.ty.disp(ctx))
}
```

## Running the lints

Install Dylint tools:

```sh
cargo install cargo-dylint dylint-link
```

### For pliron

In the pliron repository root, run:

```sh
cargo dylint --all -- --workspace --all-targets
```

To apply the suggested fixes, add `--fix`:

```sh
cargo dylint --all --fix -- --workspace --all-targets
```

### For consumer crates

In crates using pliron, add this to `Cargo.toml`, with the `tag` or `rev` that
matches the pliron version used:

```toml
[workspace.metadata.dylint]
libraries = [
    { git = "https://github.com/pliron-org/pliron", tag = "<pliron version>", pattern = "lints/pliron_lints" },
]
```

To silence a lint at one site, use an attribute that is active only during a Dylint run:

```rust
#[cfg_attr(dylint_lib = "pliron_lints", allow(ir_ref_in_scrutinee))]
```

## Developer notes

This library uses `rustc`'s internal crates, so it needs a pinned nightly in
`rust-toolchain.toml` and a matching `clippy_utils` revision in `Cargo.toml`.

To move both to a newer nightly, run this in this directory:

```sh
cargo dylint upgrade
```

An upgrade may require changes for the new compiler API. Run the UI test
(`ui/main.rs` and `ui/main.stderr`) to check:

```sh
cargo test
```
