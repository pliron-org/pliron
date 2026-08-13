// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Test that llvm operations implement the constant folding interfaces
//! [ConstFoldInterface] and [BranchOpFoldInterface] correctly

use expect_test::expect;
use pliron::{
    context::Context, init_env_logger_for_tests, irbuild::IRStatus, op::Op,
    operation::verify_operation, opts::constants::sccp::sccp, printable::Printable, result::Result,
};

use pliron_llvm::ops::FuncOp;

use crate::common;

fn run_sccp_on_text(input: &str) -> Result<(IRStatus, String)> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let op: FuncOp = common::parse_op_verify(ctx, input)?;

    let status = sccp(op.get_operation(), ctx)?;
    let after = op.disp(ctx).to_string();
    log::trace!("After SCCP:\n{}", after);
    verify_operation(op.get_operation(), ctx)?;
    Ok((status, after))
}

// ---------------------------------------------------------------------------
// llvm.add
// ---------------------------------------------------------------------------

#[test]
fn add_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return sum
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            b_v1 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !2;
            sum_v3 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !3;
            sum_v2 = llvm.add a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return sum_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn add_wraps_on_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <127: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i8;
        llvm.return sum
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <127: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !2;
            sum_v3 = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8 !3;
            sum_v2 = llvm.add a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i8 !4;
            llvm.return sum_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn add_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        sum = llvm.add x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return sum
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn add_nsw_does_not_fold_on_signed_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <127: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        sum = llvm.add a, b <{nsw=true,nuw=false}> : builtin.integer i8;
        llvm.return sum
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn add_nuw_does_not_fold_on_unsigned_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        sum = llvm.add a, b <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return sum
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn add_nsw_still_folds_without_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8;
        sum = llvm.add a, b <{nsw=true,nuw=true}> : builtin.integer i8;
        llvm.return sum
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8 !2;
            sum_v3 = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8 !3;
            sum_v2 = llvm.add a_v0, b_v1 <{nsw=true,nuw=true}>: builtin.integer i8 !4;
            llvm.return sum_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.sub
// ---------------------------------------------------------------------------

#[test]
fn sub_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        diff = llvm.sub a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return diff
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !1;
            b_v1 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !2;
            diff_v3 = builtin.constant <builtin.integer <6: i64>> : builtin.integer i64 !3;
            diff_v2 = llvm.sub a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return diff_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sub_wraps_on_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        diff = llvm.sub a, b <{nsw=false,nuw=false}> : builtin.integer i8;
        llvm.return diff
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !2;
            diff_v3 = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8 !3;
            diff_v2 = llvm.sub a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i8 !4;
            llvm.return diff_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sub_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        diff = llvm.sub x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return diff
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn sub_nsw_does_not_fold_on_signed_overflow() -> Result<()> {
    // The bit pattern for 128 (10000000) is -128 read as signed two's complement.
    // Its true difference -128 - 1 == -129 does not fit in i8's signed range
    // [-128, 127], so this signed-overflows and `nsw` is violated.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        diff = llvm.sub a, b <{nsw=true,nuw=false}> : builtin.integer i8;
        llvm.return diff
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn sub_nuw_does_not_fold_on_unsigned_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        diff = llvm.sub a, b <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return diff
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn sub_nsw_still_folds_without_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8;
        diff = llvm.sub a, b <{nsw=true,nuw=true}> : builtin.integer i8;
        llvm.return diff
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8 !2;
            diff_v3 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !3;
            diff_v2 = llvm.sub a_v0, b_v1 <{nsw=true,nuw=true}>: builtin.integer i8 !4;
            llvm.return diff_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.mul
// ---------------------------------------------------------------------------

#[test]
fn mul_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <6: i64>> : builtin.integer i64;
        prod = llvm.mul a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return prod
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !1;
            b_v1 = builtin.constant <builtin.integer <6: i64>> : builtin.integer i64 !2;
            prod_v3 = builtin.constant <builtin.integer <30: i64>> : builtin.integer i64 !3;
            prod_v2 = llvm.mul a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return prod_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mul_wraps_on_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <100: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        prod = llvm.mul a, b <{nsw=false,nuw=false}> : builtin.integer i8;
        llvm.return prod
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <100: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !2;
            prod_v3 = builtin.constant <builtin.integer <44: i8>> : builtin.integer i8 !3;
            prod_v2 = llvm.mul a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i8 !4;
            llvm.return prod_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mul_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        prod = llvm.mul x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return prod
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn mul_nsw_does_not_fold_on_signed_overflow() -> Result<()> {
    // 100 * 2 == 200 does not fit the signed range [-128, 127], so `nsw` is
    // violated.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <100: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <2: i8>> : builtin.integer i8;
        prod = llvm.mul a, b <{nsw=true,nuw=false}> : builtin.integer i8;
        llvm.return prod
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn mul_nuw_does_not_fold_on_unsigned_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <200: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <2: i8>> : builtin.integer i8;
        prod = llvm.mul a, b <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return prod
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn mul_nsw_still_folds_without_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        prod = llvm.mul a, b <{nsw=true,nuw=true}> : builtin.integer i8;
        llvm.return prod
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !2;
            prod_v3 = builtin.constant <builtin.integer <30: i8>> : builtin.integer i8 !3;
            prod_v2 = llvm.mul a_v0, b_v1 <{nsw=true,nuw=true}>: builtin.integer i8 !4;
            llvm.return prod_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.shl
// ---------------------------------------------------------------------------

#[test]
fn shl_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        shifted = llvm.shl a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return shifted
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            b_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !2;
            shifted_v3 = builtin.constant <builtin.integer <8: i64>> : builtin.integer i64 !3;
            shifted_v2 = llvm.shl a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return shifted_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Without flags, `llvm.shl` discards the bits shifted off the top, just like
/// LLVM's `shl`.
#[test]
fn shl_wraps_on_overflow() -> Result<()> {
    // 00000011 << 7 shifts bit 0 to bit 7 and drops bit 1 off the top,
    // giving 10000000, or 128 in decimal
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=false,nuw=false}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8 !2;
            shifted_v3 = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8 !3;
            shifted_v2 = llvm.shl a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i8 !4;
            llvm.return shifted_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// A shift amount `>=` the bitwidth is undefined for `shl`; SCCP must not fold
/// it regardless of flags.
#[test]
fn shl_does_not_fold_when_shift_amount_exceeds_bitwidth() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=false,nuw=false}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn shl_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        shifted = llvm.shl x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return shifted
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// `llvm.shl nuw` must not fold when a set bit is shifted off the top.
#[test]
fn shl_nuw_does_not_fold_on_unsigned_overflow() -> Result<()> {
    // The bit pattern for 255 is 11111111. 11111111 << 1 shifts a set bit off the
    // top, so `nuw` is violated.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// `llvm.shl nsw` must not fold when the shift changes the sign, even if no set
/// bit is shifted off the top.
#[test]
fn shl_nsw_does_not_fold_on_signed_overflow() -> Result<()> {
    // The bit pattern for 64 is 01000000. 01000000 << 1 == 10000000, which flips the sign from + to -.
    // Only a 0 bit is shifted off the top, so `nuw` is satisfied, but `nsw` is
    // violated.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <64: i8>> : builtin.integer i8;
        b = llvm.constant <builtin.integer <1: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=true,nuw=false}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// A set overflow flag must not block folding when the shift does not actually
/// overflow.
#[test]
fn shl_nsw_nuw_still_folds_without_overflow() -> Result<()> {
    // i8: 1 << 3 == 8, with no bits shifted off the top and no sign change.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = llvm.constant <builtin.integer <1: i8>> : builtin.integer i8;
        b = llvm.constant <builtin.integer <3: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=true,nuw=true}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = llvm.constant <builtin.integer <1: i8>> : builtin.integer i8 !1;
            b_v1 = llvm.constant <builtin.integer <3: i8>> : builtin.integer i8 !2;
            shifted_v3 = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8 !3;
            shifted_v2 = llvm.shl a_v0, b_v1 <{nsw=true,nuw=true}>: builtin.integer i8 !4;
            llvm.return shifted_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.sdiv
// ---------------------------------------------------------------------------

#[test]
fn sdiv_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <2: i8>> : builtin.integer i8;
        q = llvm.sdiv a, b : builtin.integer i8;
        llvm.return q
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <2: i8>> : builtin.integer i8 !2;
            q_v3 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !3;
            q_v2 = llvm.sdiv a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return q_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sdiv_does_not_fold_on_division_by_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        q = llvm.sdiv a, b : builtin.integer i8;
        llvm.return q
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// `INT_MIN / -1` overflows (true quotient `INT_MAX + 1`); LLVM leaves it
/// poison, so we must not fold it.
#[test]
fn sdiv_does_not_fold_on_signed_overflow() -> Result<()> {
    // i8: INT_MIN is 128 unsigned, -1 is 255 unsigned.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        q = llvm.sdiv a, b : builtin.integer i8;
        llvm.return q
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.srem
// ---------------------------------------------------------------------------

#[test]
fn srem_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        r = llvm.srem a, b : builtin.integer i8;
        llvm.return r
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !2;
            r_v3 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !3;
            r_v2 = llvm.srem a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return r_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn srem_does_not_fold_on_division_by_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        r = llvm.srem a, b : builtin.integer i8;
        llvm.return r
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn srem_does_not_fold_on_signed_overflow() -> Result<()> {
    // i8: INT_MIN is 128 unsigned, -1 is 255 unsigned.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        r = llvm.srem a, b : builtin.integer i8;
        llvm.return r
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.udiv (unsigned: no signed-overflow case, only div-by-zero)
// ---------------------------------------------------------------------------

#[test]
fn udiv_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <13: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8;
        q = llvm.udiv a, b : builtin.integer i8;
        llvm.return q
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <13: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8 !2;
            q_v3 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !3;
            q_v2 = llvm.udiv a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return q_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn udiv_does_not_fold_on_division_by_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <13: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        q = llvm.udiv a, b : builtin.integer i8;
        llvm.return q
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.urem (unsigned: no signed-overflow case, only div-by-zero)
// ---------------------------------------------------------------------------

#[test]
fn urem_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <13: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8;
        r = llvm.urem a, b : builtin.integer i8;
        llvm.return r
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <13: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8 !2;
            r_v3 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !3;
            r_v2 = llvm.urem a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return r_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn urem_does_not_fold_on_division_by_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <13: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        r = llvm.urem a, b : builtin.integer i8;
        llvm.return r
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.and
// ---------------------------------------------------------------------------

#[test]
fn and_folds_two_constants() -> Result<()> {
    // 0b1100 & 0b1010 == 0b1000 == 8.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        c = llvm.and a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8 !3;
            c_v2 = llvm.and a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn and_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        c = llvm.and x, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn and_folds_to_zero_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 (builtin.integer i1) variadic = false> [] {
        ^entry(x: builtin.integer i1):
        z = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
        c = llvm.and x, z : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i1) !0:
            z_v1 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !1;
            c_v3 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !2;
            c_v2 = llvm.and x_v0, z_v1 : builtin.integer i1 !3;
            llvm.return c_v3 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.or
// ---------------------------------------------------------------------------

#[test]
fn or_folds_two_constants() -> Result<()> {
    // 0b1100 | 0b1010 == 0b1110 == 14.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        c = llvm.or a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <14: i8>> : builtin.integer i8 !3;
            c_v2 = llvm.or a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn or_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        c = llvm.or x, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn or_folds_to_one_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 (builtin.integer i1) variadic = false> [] {
        ^entry(x: builtin.integer i1):
        one = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        c = llvm.or x, one : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i1) !0:
            one_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            c_v3 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !2;
            c_v2 = llvm.or x_v0, one_v1 : builtin.integer i1 !3;
            llvm.return c_v3 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.xor
// ---------------------------------------------------------------------------

#[test]
fn xor_folds_two_constants() -> Result<()> {
    // 0b1100 ^ 0b1010 == 0b0110 == 6.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        c = llvm.xor a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !3;
            c_v2 = llvm.xor a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn xor_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        c = llvm.xor x, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.lshr
// ---------------------------------------------------------------------------

#[test]
fn lshr_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        c = llvm.lshr a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <64: i8>> : builtin.integer i8 !3;
            c_v2 = llvm.lshr a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn lshr_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        c = llvm.lshr x, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn lshr_does_not_fold_when_shift_amount_exceeds_bitwidth() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8;
        c = llvm.lshr a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.ashr
// ---------------------------------------------------------------------------

#[test]
fn ashr_folds_two_constants() -> Result<()> {
    // Arithmetic shift copies the sign bit: 128 is -128 signed, -128 >> 1 ==
    // -64, which is 192 unsigned.
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        c = llvm.ashr a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <192: i8>> : builtin.integer i8 !3;
            c_v2 = llvm.ashr a_v0, b_v1 : builtin.integer i8 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn ashr_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        c = llvm.ashr x, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn ashr_does_not_fold_when_shift_amount_exceeds_bitwidth() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8;
        c = llvm.ashr a, b : builtin.integer i8;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.icmp
// ---------------------------------------------------------------------------

#[test]
fn icmp_eq_folds_to_true() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        c = llvm.icmp a <EQ> b : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !3;
            c_v2 = llvm.icmp a_v0 <EQ> b_v1 : builtin.integer i1 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn icmp_eq_folds_to_false() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        c = llvm.icmp a <EQ> b : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !3;
            c_v2 = llvm.icmp a_v0 <EQ> b_v1 : builtin.integer i1 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// 0xff is -1 signed, so `slt 0` is true.
#[test]
fn icmp_signed_predicate_treats_high_bit_as_negative() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        c = llvm.icmp a <SLT> b : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !3;
            c_v2 = llvm.icmp a_v0 <SLT> b_v1 : builtin.integer i1 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// 0xff is 255 unsigned, so `ult 0` is false (the same operands compare
/// oppositely to the signed predicate above).
#[test]
fn icmp_unsigned_predicate_treats_high_bit_as_large() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        c = llvm.icmp a <ULT> b : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !3;
            c_v2 = llvm.icmp a_v0 <ULT> b_v1 : builtin.integer i1 !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn icmp_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        b = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        c = llvm.icmp x <EQ> b : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.sext
// ---------------------------------------------------------------------------

#[test]
fn sext_folds_non_negative_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        c = llvm.sext a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i16() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            c_v2 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !2;
            c_v1 = llvm.sext a_v0 to builtin.integer i16 !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// A negative value replicates the sign bit:
/// -1 (i8, 0xff) -> -1 (i16, 0xffff == 65535 unsigned).
#[test]
fn sext_folds_negative_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        c = llvm.sext a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i16() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8 !1;
            c_v2 = builtin.constant <builtin.integer <65535: i16>> : builtin.integer i16 !2;
            c_v1 = llvm.sext a_v0 to builtin.integer i16 !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sext_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        c = llvm.sext x to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.zext
// ---------------------------------------------------------------------------

/// A non-negative value extends with zeros: 5 (i8) -> 5 (i16).
#[test]
fn zext_folds_non_negative_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        c = llvm.zext <nneg=false> a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i16() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            c_v2 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !2;
            c_v1 = llvm.zext <nneg=false> a_v0 to builtin.integer i16 !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// The high bit is not replicated: 255 (i8, 0xff) zero-extends to 255 (i16),
/// not 65535 as `sext` would produce.
#[test]
fn zext_folds_high_bit_set_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        c = llvm.zext <nneg=false> a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i16() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8 !1;
            c_v2 = builtin.constant <builtin.integer <255: i16>> : builtin.integer i16 !2;
            c_v1 = llvm.zext <nneg=false> a_v0 to builtin.integer i16 !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// `zext nneg` of a value whose sign bit is set (255 == -1 signed) is poison,
/// so it must not be folded to a concrete value.
#[test]
fn zext_nneg_does_not_fold_negative_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        c = llvm.zext <nneg=true> a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// `zext nneg` still folds when the operand really is non-negative.
#[test]
fn zext_nneg_folds_non_negative_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        c = llvm.zext <nneg=true> a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i16() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            c_v2 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !2;
            c_v1 = llvm.zext <nneg=true> a_v0 to builtin.integer i16 !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn zext_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        c = llvm.zext <nneg=false> x to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fneg
// ---------------------------------------------------------------------------

#[test]
fn fneg_folds_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        c = llvm.fneg <> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            c_v2 = builtin.constant <builtin.single -2.5> : builtin.fp32  !2;
            c_v1 = llvm.fneg <> a_v0 : builtin.fp32  !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fneg_folds_negative_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single -0.0> : builtin.fp32;
        c = llvm.fneg <> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The result is positive zero; it must not still be -0.0.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single -0> : builtin.fp32  !1;
            c_v2 = builtin.constant <builtin.single 0> : builtin.fp32  !2;
            c_v1 = llvm.fneg <> a_v0 : builtin.fp32  !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fneg_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        c = llvm.fneg <> x : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fneg_folds_positive_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fneg <> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            c_v2 = builtin.constant <builtin.single -Inf> : builtin.fp32  !2;
            c_v1 = llvm.fneg <> a_v0 : builtin.fp32  !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fneg_folds_negative_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single -Inf> : builtin.fp32;
        c = llvm.fneg <> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single -Inf> : builtin.fp32  !1;
            c_v2 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v1 = llvm.fneg <> a_v0 : builtin.fp32  !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fneg_folds_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        c = llvm.fneg <> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single NaN> : builtin.fp32  !1;
            c_v2 = builtin.constant <builtin.single NaN> : builtin.fp32  !2;
            c_v1 = llvm.fneg <> a_v0 : builtin.fp32  !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fneg_nnan_does_not_fold_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        c = llvm.fneg <NNAN> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fneg_ninf_does_not_fold_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fneg <NINF> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fneg_nnan_still_folds_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        c = llvm.fneg <NNAN> a : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            c_v2 = builtin.constant <builtin.single -2.5> : builtin.fp32  !2;
            c_v1 = llvm.fneg <NNAN> a_v0 : builtin.fp32  !3;
            llvm.return c_v2 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fadd
// ---------------------------------------------------------------------------

#[test]
fn fadd_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fadd <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 6.5> : builtin.fp32  !3;
            c_v2 = llvm.fadd <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fadd_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fadd <> x, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fadd_folds_infinity_and_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single 1.0> : builtin.fp32;
        c = llvm.fadd <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 1> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single +Inf> : builtin.fp32  !3;
            c_v2 = llvm.fadd <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fadd_folds_opposite_infinities_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single -Inf> : builtin.fp32;
        c = llvm.fadd <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single -Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fadd <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fadd_folds_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        b = builtin.constant <builtin.single 1.0> : builtin.fp32;
        c = llvm.fadd <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single NaN> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 1> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fadd <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fadd_ninf_does_not_fold_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single 1.0> : builtin.fp32;
        c = llvm.fadd <NINF> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fadd_nnan_does_not_fold_nan_result() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single -Inf> : builtin.fp32;
        c = llvm.fadd <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fadd_nnan_still_folds_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fadd <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 6.5> : builtin.fp32  !3;
            c_v2 = llvm.fadd <NNAN> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fsub
// ---------------------------------------------------------------------------

#[test]
fn fsub_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = llvm.constant <builtin.single 2.5> : builtin.fp32;
        c = llvm.fsub <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 10> : builtin.fp32  !1;
            b_v1 = llvm.constant <builtin.single 2.5> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 7.5> : builtin.fp32  !3;
            c_v2 = llvm.fsub <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fsub_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 2.5> : builtin.fp32;
        c = llvm.fsub <> x, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fsub_folds_finite_minus_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 1.0> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fsub <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 1> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single -Inf> : builtin.fp32  !3;
            c_v2 = llvm.fsub <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fsub_folds_equal_infinities_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fsub <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fsub <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fsub_folds_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        b = builtin.constant <builtin.single 1.0> : builtin.fp32;
        c = llvm.fsub <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single NaN> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 1> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fsub <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fsub_nnan_does_not_fold_nan_result() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fsub <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fmul
// ---------------------------------------------------------------------------

#[test]
fn fmul_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fmul <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 10> : builtin.fp32  !3;
            c_v2 = llvm.fmul <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fmul_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fmul <> x, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fmul_folds_negative_operands_to_positive() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single -2.5> : builtin.fp32;
        b = builtin.constant <builtin.single -4.0> : builtin.fp32;
        c = llvm.fmul <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single -2.5> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single -4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 10> : builtin.fp32  !3;
            c_v2 = llvm.fmul <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fmul_folds_infinity_and_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.fmul <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 2> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single +Inf> : builtin.fp32  !3;
            c_v2 = llvm.fmul <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fmul_folds_zero_times_infinity_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 0.0> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fmul <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 0> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fmul <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fmul_folds_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.fmul <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single NaN> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 2> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fmul <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fmul_ninf_does_not_fold_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.fmul <NINF> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fmul_nnan_does_not_fold_nan_result() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = llvm.constant <builtin.single 0.0> : builtin.fp32;
        b = llvm.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fmul <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fmul_nnan_still_folds_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fmul <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 10> : builtin.fp32  !3;
            c_v2 = llvm.fmul <NNAN> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fdiv
// ---------------------------------------------------------------------------

#[test]
fn fdiv_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fdiv <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 10> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 2.5> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fdiv <> x, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fdiv_folds_finite_by_zero_to_infinity() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 1.0> : builtin.fp32;
        b = builtin.constant <builtin.single 0.0> : builtin.fp32;
        c = llvm.fdiv <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 1> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 0> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single +Inf> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_folds_zero_by_zero_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 0.0> : builtin.fp32;
        b = builtin.constant <builtin.single 0.0> : builtin.fp32;
        c = llvm.fdiv <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 0> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 0> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_folds_infinity_by_infinity_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fdiv <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_folds_finite_by_infinity_to_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 1.0> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.fdiv <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 1> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 0> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_folds_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.fdiv <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single NaN> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 2> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_ninf_does_not_fold_division_by_zero() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 1.0> : builtin.fp32;
        b = builtin.constant <builtin.single 0.0> : builtin.fp32;
        c = llvm.fdiv <NINF> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fdiv_nnan_does_not_fold_nan_result() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 0.0> : builtin.fp32;
        b = builtin.constant <builtin.single 0.0> : builtin.fp32;
        c = llvm.fdiv <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn fdiv_nnan_still_folds_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.fdiv <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 10> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 2.5> : builtin.fp32  !3;
            c_v2 = llvm.fdiv <NNAN> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.frem
// ---------------------------------------------------------------------------

#[test]
fn frem_folds_two_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 10> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 2> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn frem_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.frem <> x, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// `frem` truncates toward zero, so the result takes the sign of the dividend
/// rather than rounding to nearest as IEEE `remainder` would.
#[test]
fn frem_result_takes_sign_of_dividend() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single -10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single -10> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single -2> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// IEEE `remainder(3.0, 2.0)` would be -1.0; `frem`/`fmod` must give 1.0.
#[test]
fn frem_truncates_rather_than_rounding_to_nearest() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 3.0> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 3> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 2> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 1> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn frem_folds_by_zero_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 1.0> : builtin.fp32;
        b = builtin.constant <builtin.single 0.0> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 1> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 0> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn frem_folds_infinity_dividend_to_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single +Inf> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single +Inf> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 2> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// A finite dividend with an infinite divisor returns the dividend unchanged.
#[test]
fn frem_folds_infinity_divisor_to_dividend() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single +Inf> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 2.5> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn frem_folds_nan() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single NaN> : builtin.fp32;
        b = builtin.constant <builtin.single 2.0> : builtin.fp32;
        c = llvm.frem <> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single NaN> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 2> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single NaN> : builtin.fp32  !3;
            c_v2 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn frem_nnan_does_not_fold_nan_result() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 1.0> : builtin.fp32;
        b = builtin.constant <builtin.single 0.0> : builtin.fp32;
        c = llvm.frem <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn frem_ninf_does_not_fold_infinite_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single +Inf> : builtin.fp32;
        c = llvm.frem <NINF> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn frem_nnan_still_folds_finite() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        c = llvm.frem <NNAN> a, b : builtin.fp32;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 10> : builtin.fp32  !1;
            b_v1 = builtin.constant <builtin.single 4> : builtin.fp32  !2;
            c_v3 = builtin.constant <builtin.single 2> : builtin.fp32  !3;
            c_v2 = llvm.frem <NNAN> a_v0, b_v1 : builtin.fp32  !4;
            llvm.return c_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}
