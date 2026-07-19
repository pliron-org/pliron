// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use pliron::{common_traits::Verify, context::Context};
use pliron_derive::verify_succ;

#[verify_succ]
struct AlwaysValidStruct {
    value: u32,
}

#[verify_succ]
enum AlwaysValidEnum<T> {
    Value(T),
    Empty,
}

#[test]
fn verify_succ_struct_returns_ok() {
    let ctx = Context::new();
    let value = AlwaysValidStruct { value: 7 };
    assert_eq!(value.value, 7);
    assert!(value.verify(&ctx).is_ok());
}

#[test]
fn verify_succ_enum_returns_ok() {
    let ctx = Context::new();
    let value = AlwaysValidEnum::Value(3u8);
    assert!(value.verify(&ctx).is_ok());

    let value = AlwaysValidEnum::<u8>::Empty;
    assert!(value.verify(&ctx).is_ok());
}
