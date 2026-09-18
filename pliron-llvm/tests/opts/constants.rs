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
            sum_v3 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !3;
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
            diff_v3 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !3;
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
            shifted_v3 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !3;
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
            a_v0 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !1;
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
            a_v0 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !1;
            b_v1 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !2;
            c_v3 = builtin.constant <builtin.integer <-64: i8>> : builtin.integer i8 !3;
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
            a_v0 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !1;
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
            a_v0 = llvm.constant <builtin.integer <-1: i8>> : builtin.integer i8 !1;
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
            a_v0 = llvm.constant <builtin.integer <-1: i8>> : builtin.integer i8 !1;
            c_v2 = builtin.constant <builtin.integer <-1: i16>> : builtin.integer i16 !2;
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
            a_v0 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !1;
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
// llvm.trunc
// ---------------------------------------------------------------------------

/// Truncation keeps only the low bits: 5 (i16) stays 5, 258 (i16, 0x102) becomes
/// 2 (i8), and -1 (i16, 0xffff) becomes 0xff, which is -1 again at the narrower
/// width.
#[test]
fn trunc_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        fits = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16;
        high_bits = builtin.constant <builtin.integer <258: i16>> : builtin.integer i16;
        negative = builtin.constant <builtin.integer <65535: i16>> : builtin.integer i16;
        a = llvm.trunc fits to builtin.integer i8;
        b = llvm.trunc high_bits to builtin.integer i8;
        c = llvm.trunc negative to builtin.integer i8;
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
            fits_v0 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !1;
            high_bits_v1 = builtin.constant <builtin.integer <258: i16>> : builtin.integer i16 !2;
            negative_v2 = builtin.constant <builtin.integer <-1: i16>> : builtin.integer i16 !3;
            a_v6 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !4;
            a_v3 = llvm.trunc fits_v0 to builtin.integer i8 !5;
            b_v7 = builtin.constant <builtin.integer <2: i8>> : builtin.integer i8 !6;
            b_v4 = llvm.trunc high_bits_v1 to builtin.integer i8 !7;
            c_v8 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !8;
            c_v5 = llvm.trunc negative_v2 to builtin.integer i8 !9;
            llvm.return c_v8 !10
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn trunc_does_not_fold_with_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i16) variadic = false> [] {
        ^entry(x: builtin.integer i16):
        c = llvm.trunc x to builtin.integer i8;
        llvm.return c
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fpext
// ---------------------------------------------------------------------------

#[test]
fn fpext_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp64 () variadic = false> [] {
        ^entry():
        h = builtin.constant <builtin.half 1.5> : builtin.fp16;
        s = builtin.constant <builtin.single 2.5> : builtin.fp32;
        h_to_s = llvm.fpext <> h to builtin.fp32;
        h_to_d = llvm.fpext <> h to builtin.fp64;
        s_to_d = llvm.fpext <> s to builtin.fp64;
        llvm.return s_to_d
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp64 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            h_v0 = builtin.constant <builtin.half 1.5> : builtin.fp16  !1;
            s_v1 = builtin.constant <builtin.single 2.5> : builtin.fp32  !2;
            h_to_s_v5 = builtin.constant <builtin.single 1.5> : builtin.fp32  !3;
            h_to_s_v2 = llvm.fpext <> h_v0 to builtin.fp32  !4;
            h_to_d_v6 = builtin.constant <builtin.double 1.5> : builtin.fp64  !5;
            h_to_d_v3 = llvm.fpext <> h_v0 to builtin.fp64  !6;
            s_to_d_v7 = builtin.constant <builtin.double 2.5> : builtin.fp64  !7;
            s_to_d_v4 = llvm.fpext <> s_v1 to builtin.fp64  !8;
            llvm.return s_to_d_v7 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// A `nnan` or `ninf` operand is poison, so neither may be folded to a value.
#[test]
fn fpext_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp64 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        nnan_nan = llvm.fpext <NNAN> nan to builtin.fp64;
        ninf_inf = llvm.fpext <NINF> inf to builtin.fp64;
        non_constant = llvm.fpext <> x to builtin.fp64;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fptrunc
// ---------------------------------------------------------------------------

/// The last case rounds to the destination precision: 1.0000000001 is not
/// representable as a single.
#[test]
fn fptrunc_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        d = builtin.constant <builtin.double 2.5> : builtin.fp64;
        d_half = builtin.constant <builtin.double 1.5> : builtin.fp64;
        s = builtin.constant <builtin.single 3.25> : builtin.fp32;
        inexact = builtin.constant <builtin.double 1.0000000001> : builtin.fp64;
        d_to_s = llvm.fptrunc <> d to builtin.fp32;
        d_to_h = llvm.fptrunc <> d_half to builtin.fp16;
        s_to_h = llvm.fptrunc <> s to builtin.fp16;
        rounded = llvm.fptrunc <> inexact to builtin.fp32;
        llvm.return rounded
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            d_v0 = builtin.constant <builtin.double 2.5> : builtin.fp64  !1;
            d_half_v1 = builtin.constant <builtin.double 1.5> : builtin.fp64  !2;
            s_v2 = builtin.constant <builtin.single 3.25> : builtin.fp32  !3;
            inexact_v3 = builtin.constant <builtin.double 1.0000000001> : builtin.fp64  !4;
            d_to_s_v8 = builtin.constant <builtin.single 2.5> : builtin.fp32  !5;
            d_to_s_v4 = llvm.fptrunc <> d_v0 to builtin.fp32  !6;
            d_to_h_v9 = builtin.constant <builtin.half 1.5> : builtin.fp16  !7;
            d_to_h_v5 = llvm.fptrunc <> d_half_v1 to builtin.fp16  !8;
            s_to_h_v10 = builtin.constant <builtin.half 3.25> : builtin.fp16  !9;
            s_to_h_v6 = llvm.fptrunc <> s_v2 to builtin.fp16  !10;
            rounded_v11 = builtin.constant <builtin.single 1> : builtin.fp32  !11;
            rounded_v7 = llvm.fptrunc <> inexact_v3 to builtin.fp32  !12;
            llvm.return rounded_v11 !13
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// `ninf` also rules out a result that overflows to infinity, not just an
/// infinite operand.
#[test]
fn fptrunc_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp64) variadic = false> [] {
        ^entry(x: builtin.fp64):
        nan = builtin.constant <builtin.double NaN> : builtin.fp64;
        huge = builtin.constant <builtin.double 1e300> : builtin.fp64;
        nnan_nan = llvm.fptrunc <NNAN> nan to builtin.fp32;
        ninf_overflow = llvm.fptrunc <NINF> huge to builtin.fp32;
        non_constant = llvm.fptrunc <> x to builtin.fp32;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.sitofp
// ---------------------------------------------------------------------------

/// The i8 operand has its high bit set, so it reads as -1; and 16777217 is not
/// representable as a single, so it rounds to 16777216.
#[test]
fn sitofp_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        small = builtin.constant <builtin.integer <42: i16>> : builtin.integer i16;
        mid = builtin.constant <builtin.integer <257: i32>> : builtin.integer i32;
        high_bit = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        inexact = builtin.constant <builtin.integer <16777217: i32>> : builtin.integer i32;
        to_half = llvm.sitofp small to builtin.fp16;
        to_single = llvm.sitofp mid to builtin.fp32;
        negative = llvm.sitofp high_bit to builtin.fp64;
        rounded = llvm.sitofp inexact to builtin.fp32;
        llvm.return rounded
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp32 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            small_v0 = builtin.constant <builtin.integer <42: i16>> : builtin.integer i16 !1;
            mid_v1 = builtin.constant <builtin.integer <257: i32>> : builtin.integer i32 !2;
            high_bit_v2 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !3;
            inexact_v3 = builtin.constant <builtin.integer <16777217: i32>> : builtin.integer i32 !4;
            to_half_v8 = builtin.constant <builtin.half 42> : builtin.fp16  !5;
            to_half_v4 = llvm.sitofp small_v0 to builtin.fp16  !6;
            to_single_v9 = builtin.constant <builtin.single 257> : builtin.fp32  !7;
            to_single_v5 = llvm.sitofp mid_v1 to builtin.fp32  !8;
            negative_v10 = builtin.constant <builtin.double -1> : builtin.fp64  !9;
            negative_v6 = llvm.sitofp high_bit_v2 to builtin.fp64  !10;
            rounded_v11 = builtin.constant <builtin.single 16777216> : builtin.fp32  !11;
            rounded_v7 = llvm.sitofp inexact_v3 to builtin.fp32  !12;
            llvm.return rounded_v11 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn sitofp_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.integer i32) variadic = false> [] {
        ^entry(x: builtin.integer i32):
        wide = builtin.constant <builtin.integer <1: i129>> : builtin.integer i129;
        wider_than_128_bits = llvm.sitofp wide to builtin.fp64;
        non_constant = llvm.sitofp x to builtin.fp32;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.uitofp
// ---------------------------------------------------------------------------

/// The i8 operand has its high bit set, and reads as 255 rather than -1.
#[test]
fn uitofp_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp64 () variadic = false> [] {
        ^entry():
        small = builtin.constant <builtin.integer <42: i16>> : builtin.integer i16;
        mid = builtin.constant <builtin.integer <257: i32>> : builtin.integer i32;
        high_bit = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        non_negative = builtin.constant <builtin.integer <42: i8>> : builtin.integer i8;
        to_half = llvm.uitofp <nneg=false> small to builtin.fp16;
        to_single = llvm.uitofp <nneg=false> mid to builtin.fp32;
        unsigned = llvm.uitofp <nneg=false> high_bit to builtin.fp64;
        nneg = llvm.uitofp <nneg=true> non_negative to builtin.fp64;
        llvm.return nneg
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.fp64 () variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            small_v0 = builtin.constant <builtin.integer <42: i16>> : builtin.integer i16 !1;
            mid_v1 = builtin.constant <builtin.integer <257: i32>> : builtin.integer i32 !2;
            high_bit_v2 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !3;
            non_negative_v3 = builtin.constant <builtin.integer <42: i8>> : builtin.integer i8 !4;
            to_half_v8 = builtin.constant <builtin.half 42> : builtin.fp16  !5;
            to_half_v4 = llvm.uitofp <nneg=false> small_v0 to builtin.fp16  !6;
            to_single_v9 = builtin.constant <builtin.single 257> : builtin.fp32  !7;
            to_single_v5 = llvm.uitofp <nneg=false> mid_v1 to builtin.fp32  !8;
            unsigned_v10 = builtin.constant <builtin.double 255> : builtin.fp64  !9;
            unsigned_v6 = llvm.uitofp <nneg=false> high_bit_v2 to builtin.fp64  !10;
            nneg_v11 = builtin.constant <builtin.double 42> : builtin.fp64  !11;
            nneg_v7 = llvm.uitofp <nneg=true> non_negative_v3 to builtin.fp64  !12;
            llvm.return nneg_v11 !13
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// `uitofp nneg` on an operand whose bit pattern is negative is poison.
#[test]
fn uitofp_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.integer i32) variadic = false> [] {
        ^entry(x: builtin.integer i32):
        negative = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        wide = builtin.constant <builtin.integer <1: i129>> : builtin.integer i129;
        nneg_negative = llvm.uitofp <nneg=true> negative to builtin.fp64;
        wider_than_128_bits = llvm.uitofp <nneg=false> wide to builtin.fp64;
        non_constant = llvm.uitofp <nneg=false> x to builtin.fp32;
        llvm.return non_constant
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

// ---------------------------------------------------------------------------
// llvm.fcmp
// ---------------------------------------------------------------------------

/// The `O` predicates hold only when both operands are ordered, while the `U`
/// predicates also hold when either is NaN. `True` and `False` ignore their
/// operands, and IEEE equality makes -0.0 equal to 0.0.
#[test]
fn fcmp_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        a2 = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        neg_zero = builtin.constant <builtin.single -0.0> : builtin.fp32;
        pos_zero = builtin.constant <builtin.single 0.0> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        oeq_equal = llvm.fcmp <> a <OEQ> a2 : builtin.integer i1;
        oeq_unequal = llvm.fcmp <> a <OEQ> b : builtin.integer i1;
        oeq_signed_zeros = llvm.fcmp <> neg_zero <OEQ> pos_zero : builtin.integer i1;
        ordered_with_nan = llvm.fcmp <> nan <OLT> a : builtin.integer i1;
        unordered_with_nan = llvm.fcmp <> nan <UGT> a : builtin.integer i1;
        ord_with_nan = llvm.fcmp <> a <ORD> nan : builtin.integer i1;
        uno_with_nan = llvm.fcmp <> a <UNO> nan : builtin.integer i1;
        below_infinity = llvm.fcmp <> a <OLT> inf : builtin.integer i1;
        always_true = llvm.fcmp <> a <True> b : builtin.integer i1;
        always_false = llvm.fcmp <> a <False> b : builtin.integer i1;
        nnan_finite = llvm.fcmp <NNAN> a <OLT> b : builtin.integer i1;
        llvm.return nnan_finite
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i1() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.single 2.5> : builtin.fp32  !1;
            a2_v1 = builtin.constant <builtin.single 2.5> : builtin.fp32  !2;
            b_v2 = builtin.constant <builtin.single 4> : builtin.fp32  !3;
            neg_zero_v3 = builtin.constant <builtin.single -0> : builtin.fp32  !4;
            pos_zero_v4 = builtin.constant <builtin.single 0> : builtin.fp32  !5;
            nan_v5 = builtin.constant <builtin.single NaN> : builtin.fp32  !6;
            inf_v6 = builtin.constant <builtin.single +Inf> : builtin.fp32  !7;
            oeq_equal_v18 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !8;
            oeq_equal_v7 = llvm.fcmp <> a_v0 <OEQ> a2_v1 : builtin.integer i1 !9;
            oeq_unequal_v19 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !10;
            oeq_unequal_v8 = llvm.fcmp <> a_v0 <OEQ> b_v2 : builtin.integer i1 !11;
            oeq_signed_zeros_v20 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !12;
            oeq_signed_zeros_v9 = llvm.fcmp <> neg_zero_v3 <OEQ> pos_zero_v4 : builtin.integer i1 !13;
            ordered_with_nan_v21 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !14;
            ordered_with_nan_v10 = llvm.fcmp <> nan_v5 <OLT> a_v0 : builtin.integer i1 !15;
            unordered_with_nan_v22 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !16;
            unordered_with_nan_v11 = llvm.fcmp <> nan_v5 <UGT> a_v0 : builtin.integer i1 !17;
            ord_with_nan_v23 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !18;
            ord_with_nan_v12 = llvm.fcmp <> a_v0 <ORD> nan_v5 : builtin.integer i1 !19;
            uno_with_nan_v24 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !20;
            uno_with_nan_v13 = llvm.fcmp <> a_v0 <UNO> nan_v5 : builtin.integer i1 !21;
            below_infinity_v25 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !22;
            below_infinity_v14 = llvm.fcmp <> a_v0 <OLT> inf_v6 : builtin.integer i1 !23;
            always_true_v26 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !24;
            always_true_v15 = llvm.fcmp <> a_v0 <True> b_v2 : builtin.integer i1 !25;
            always_false_v27 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !26;
            always_false_v16 = llvm.fcmp <> a_v0 <False> b_v2 : builtin.integer i1 !27;
            nnan_finite_v28 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !28;
            nnan_finite_v17 = llvm.fcmp <NNAN> a_v0 <OLT> b_v2 : builtin.integer i1 !29;
            llvm.return nnan_finite_v28 !30
        }"#]].assert_eq(&after);
    Ok(())
}

/// `nnan` and `ninf` assert their operands are neither NaN nor infinite; when
/// they are, the result is poison, even though the comparison itself is total.
#[test]
fn fcmp_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        nnan_nan = llvm.fcmp <NNAN> nan <UNO> a : builtin.integer i1;
        ninf_inf = llvm.fcmp <NINF> inf <OGT> a : builtin.integer i1;
        non_constant = llvm.fcmp <> x <OEQ> a : builtin.integer i1;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.select
// ---------------------------------------------------------------------------

/// A true condition selects the first value operand: the select's result is
/// known to be 10, so the dependent add folds to 11.
/// A constant condition makes the select's result that of the chosen operand,
/// so the dependent adds fold to 11 and 21 respectively.
#[test]
fn select_constant_condition_propagates_chosen_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        f = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
        a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
        one = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        taken = llvm.select t ? a : b : builtin.integer i64;
        not_taken = llvm.select f ? a : b : builtin.integer i64;
        from_true = llvm.add taken, one <{nsw=false,nuw=false}> : builtin.integer i64;
        from_false = llvm.add not_taken, one <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return from_false
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            t_v0 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            f_v1 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !2;
            a_v2 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !3;
            b_v3 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !4;
            one_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !5;
            taken_v5 = llvm.select  t_v0 ? a_v2 : b_v3 : builtin.integer i64 !6;
            not_taken_v6 = llvm.select  f_v1 ? a_v2 : b_v3 : builtin.integer i64 !7;
            from_true_v9 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64 !8;
            from_true_v7 = llvm.add a_v2, one_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !9;
            from_false_v10 = builtin.constant <builtin.integer <21: i64>> : builtin.integer i64 !10;
            from_false_v8 = llvm.add b_v3, one_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !11;
            llvm.return from_false_v10 !12
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn select_constant_condition_forwards_non_constant_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        cond = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        b = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
        s = llvm.select cond ? x : b : builtin.integer i64;
        llvm.return s
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64) !0:
            cond_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            b_v2 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !2;
            s_v3 = llvm.select  cond_v1 ? x_v0 : b_v2 : builtin.integer i64 !3;
            llvm.return x_v0 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Whichever way an unknown condition goes, equal constant operands make the
/// result that same constant.
#[test]
fn select_unknown_condition_folds_equal_constant_operands() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
        ^entry(cond: builtin.integer i1):
        a = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
        s = llvm.select cond ? a : b : builtin.integer i64;
        llvm.return s
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            a_v1 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !1;
            b_v2 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !2;
            s_v4 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !3;
            s_v3 = llvm.select  cond_v0 ? a_v1 : b_v2 : builtin.integer i64 !4;
            llvm.return s_v4 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn select_does_not_fold_unknown_condition_with_distinct_operands() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
        ^entry(cond: builtin.integer i1):
        a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
        s = llvm.select cond ? a : b : builtin.integer i64;
        llvm.return s
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

/// Forwarding the chosen operand is type-agnostic: a scalar constant
/// condition folds a select between whole vectors.
#[test]
fn select_scalar_constant_condition_forwards_vector_operand() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.vector <Fixed x 4 x builtin.integer i32> (llvm.vector <Fixed x 4 x builtin.integer i32>, llvm.vector <Fixed x 4 x builtin.integer i32>) variadic = false> [] {
        ^entry(x: llvm.vector <Fixed x 4 x builtin.integer i32>, y: llvm.vector <Fixed x 4 x builtin.integer i32>):
        cond = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
        s = llvm.select cond ? x : y : llvm.vector <Fixed x 4 x builtin.integer i32>;
        llvm.return s
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <llvm.vector <Fixed x 4 x builtin.integer i32>(llvm.vector <Fixed x 4 x builtin.integer i32>, llvm.vector <Fixed x 4 x builtin.integer i32>) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: llvm.vector <Fixed x 4 x builtin.integer i32>, y_v1: llvm.vector <Fixed x 4 x builtin.integer i32>) !0:
            cond_v2 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !1;
            s_v3 = llvm.select  cond_v2 ? x_v0 : y_v1 : llvm.vector <Fixed x 4 x builtin.integer i32> !2;
            llvm.return y_v1 !3
        }"#]].assert_eq(&after);
    Ok(())
}

/// A vector-of-i1 condition selects element-wise; there is no vector constant
/// attribute to propagate, so SCCP must leave the op alone (and not crash).
#[test]
fn select_does_not_fold_with_vector_condition() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.vector <Fixed x 4 x builtin.integer i32> (llvm.vector <Fixed x 4 x builtin.integer i1>, llvm.vector <Fixed x 4 x builtin.integer i32>, llvm.vector <Fixed x 4 x builtin.integer i32>) variadic = false> [] {
        ^entry(cond: llvm.vector <Fixed x 4 x builtin.integer i1>, x: llvm.vector <Fixed x 4 x builtin.integer i32>, y: llvm.vector <Fixed x 4 x builtin.integer i32>):
        s = llvm.select cond ? x : y : llvm.vector <Fixed x 4 x builtin.integer i32>;
        llvm.return s
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}
