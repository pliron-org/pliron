## Programming Languages Intermediate Representation

[![Status](https://github.com/pliron-org/pliron/actions/workflows/ci.yml/badge.svg)](https://github.com/pliron-org/pliron/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/pliron)](https://crates.io/crates/pliron)
[![Docs.rs](https://img.shields.io/docsrs/pliron)](https://docs.rs/pliron/latest/pliron/)
[![Discord](https://img.shields.io/discord/1481908978978918523)](https://discord.gg/5M3K4Ujv7v)

`pliron` is an extensible compiler IR framework in Rust, inspired by [MLIR](https://mlir.llvm.org/).

### Build and Test
* Install the [rust toolchain](https://www.rust-lang.org/tools/install).
* `cargo build` and `cargo test` should build the compiler and run the testsuite.
* To see a simple IR constructed (by the [print_simple](tests/ir_construct.rs) test),
  use the following command:

      cargo test print_simple -- --show-output

  It should print something like:
  ```mlir
  builtin.module @bar 
  {
    ^block1v1():
      builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
      {
        ^entry_block2v1():
          c0_v0 = test.constant builtin.integer <0: si64> !0;
          test.return c0_v0
      }
  }

  outlined_attributes:
  !0 = [builtin_debug_info = builtin.debug_info [c0]]
  ```
* `pliron` provides an [LLVM Dialect](pliron-llvm/README.md) and
consequently an [`llvm-opt` tool](pliron-llvm/llvm-opt/README.md)
that can parse LLVM-IR bitcode into the LLVM dialect and output
LLVM-IR bitcode.

### Using the Library
Add a dependence on the [crate](https://crates.io/crates/pliron) in your Rust project.

Note: `pliron` is under active development. Every effort is made to ensure that the code is well tested
and of production quality. The LLVM dialect, although not complete, can be useful practically. It can,
for example, [compile bzip2](https://github.com/pliron-org/pliron/wiki/Compiling-bzip2-through-pliron's-LLVM-dialect).
We also plan to start work on supporting a cranelift dialect/backend soon.

### Documentation & Resources
* [Kaleidoscope tutorial](https://pliron-org.github.io/pliron/Kaleidoscope).
* Latest [docs](https://pliron-org.github.io/pliron/pliron) (built from `master`).
* Release [docs.rs](https://docs.rs/pliron/latest/pliron/).
* Misc articles on the [wiki](https://github.com/pliron-org/pliron/wiki)
* #### Some talks / videos on `pliron`
  * [pliron: An Extensible IR Framework in Rust - IICT'24](https://www.youtube.com/watch?v=LobYuwcUaZA)
  * [Declarative IR Specification in Pliron - IICT'25](https://www.youtube.com/watch?v=w-g4xSOC9og)
  * [Rust(ing) the Future of Compilers: Pliron as the MLIR Alternative (No C/C++)](https://www.youtube.com/watch?v=rRgYGBAhKQ0)
  * [Pliron Rust Workshop (6 sessions)](https://www.youtube.com/watch?v=6EjMWJ2PY-o)

### Projects using `pliron`
* [cuda-oxide](https://github.com/NVlabs/cuda-oxide): NVIDIA's Rust CUDA compiler.
* [Commonly used Pliron Dialects](https://github.com/pliron-org/pliron-common-dialects)
* [Pliron Dialect for Tensors](https://github.com/pliron-org/pliron-tensor)

### Discussions
- [Discord channel](https://discord.com/channels/1481908978978918523/) ([Invitation link](https://discord.gg/5M3K4Ujv7v))
- [GitHub Discussions](https://github.com/pliron-org/pliron/discussions)

![pliron-logo](.github/workflows/pliron-logo.png)
