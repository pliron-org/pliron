# pliron_lints

[Dylint](https://github.com/trailofbits/dylint) lints for pliron and code built on it.

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
