// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

fn main() {
    // Tell Cargo to link to libffi
    println!("cargo::rustc-link-lib=ffi");
}
