# LLVM Dialect for [pliron](../README.md)

[![Crates.io](https://img.shields.io/crates/v/pliron-llvm)](https://crates.io/crates/pliron-llvm)
[![Docs.rs](https://img.shields.io/docsrs/pliron-llvm)](https://docs.rs/pliron-llvm/latest/pliron-llvm/)

This crate provides the following functionality:
1. Dialect definitions of LLVM ops, types and attributes.
2. A wrapper around [llvm-sys](https://crates.io/crates/llvm-sys)
  converting to and from our LLVM dialect. This requires
  LLVM to be installed locally.

We currently support LLVM-23 and it needs to be on your computer.
For installing on Debian / Ubuntu, it is recommended to use the
[automatic installation script](https://apt.llvm.org/). If you
prefer to install individual packages, you will need `libllvm23`,
`llvm-23-dev`, `llvm-23-tools`, `clang-23`, `libpolly-23-dev`, etc.

pliron-llvm also provides an [llvm-opt](llvm-opt/README.md) tool.
