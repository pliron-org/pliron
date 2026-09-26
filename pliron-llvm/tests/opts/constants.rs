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
fn add_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        max_i8 = builtin.constant <builtin.integer <127: i8>> : builtin.integer i8;
        one_i8 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        three_i8 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        four_i8 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8;
        sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        wrapped = llvm.add max_i8, one_i8 <{nsw=false,nuw=false}> : builtin.integer i8;
        no_overflow = llvm.add three_i8, four_i8 <{nsw=true,nuw=true}> : builtin.integer i8;
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
            max_i8_v2 = builtin.constant <builtin.integer <127: i8>> : builtin.integer i8 !3;
            one_i8_v3 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !4;
            three_i8_v4 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !5;
            four_i8_v5 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8 !6;
            sum_v9 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !7;
            sum_v6 = llvm.add a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
            wrapped_v10 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !9;
            wrapped_v7 = llvm.add max_i8_v2, one_i8_v3 <{nsw=false,nuw=false}>: builtin.integer i8 !10;
            no_overflow_v11 = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8 !11;
            no_overflow_v8 = llvm.add three_i8_v4, four_i8_v5 <{nsw=true,nuw=true}>: builtin.integer i8 !12;
            llvm.return sum_v9 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn add_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        max_i8 = builtin.constant <builtin.integer <127: i8>> : builtin.integer i8;
        umax_i8 = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        one_i8 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        non_constant = llvm.add x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        signed_overflow = llvm.add max_i8, one_i8 <{nsw=true,nuw=false}> : builtin.integer i8;
        unsigned_overflow = llvm.add umax_i8, one_i8 <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.sub
// ---------------------------------------------------------------------------

#[test]
fn sub_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        zero_i8 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        one_i8 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        ten_i8 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        four_i8 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8;
        diff = llvm.sub a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        wrapped = llvm.sub zero_i8, one_i8 <{nsw=false,nuw=false}> : builtin.integer i8;
        no_overflow = llvm.sub ten_i8, four_i8 <{nsw=true,nuw=true}> : builtin.integer i8;
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
            zero_i8_v2 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8 !3;
            one_i8_v3 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8 !4;
            ten_i8_v4 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !5;
            four_i8_v5 = builtin.constant <builtin.integer <4: i8>> : builtin.integer i8 !6;
            diff_v9 = builtin.constant <builtin.integer <6: i64>> : builtin.integer i64 !7;
            diff_v6 = llvm.sub a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
            wrapped_v10 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !9;
            wrapped_v7 = llvm.sub zero_i8_v2, one_i8_v3 <{nsw=false,nuw=false}>: builtin.integer i8 !10;
            no_overflow_v11 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !11;
            no_overflow_v8 = llvm.sub ten_i8_v4, four_i8_v5 <{nsw=true,nuw=true}>: builtin.integer i8 !12;
            llvm.return diff_v9 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn sub_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        min_i8 = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        zero_i8 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        one_i8 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        non_constant = llvm.sub x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        signed_overflow = llvm.sub min_i8, one_i8 <{nsw=true,nuw=false}> : builtin.integer i8;
        unsigned_overflow = llvm.sub zero_i8, one_i8 <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.mul
// ---------------------------------------------------------------------------

#[test]
fn mul_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <6: i64>> : builtin.integer i64;
        hundred_i8 = builtin.constant <builtin.integer <100: i8>> : builtin.integer i8;
        three_i8 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        five_i8 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        six_i8 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        prod = llvm.mul a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        wrapped = llvm.mul hundred_i8, three_i8 <{nsw=false,nuw=false}> : builtin.integer i8;
        no_overflow = llvm.mul five_i8, six_i8 <{nsw=true,nuw=true}> : builtin.integer i8;
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
            hundred_i8_v2 = builtin.constant <builtin.integer <100: i8>> : builtin.integer i8 !3;
            three_i8_v3 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !4;
            five_i8_v4 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !5;
            six_i8_v5 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !6;
            prod_v9 = builtin.constant <builtin.integer <30: i64>> : builtin.integer i64 !7;
            prod_v6 = llvm.mul a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
            wrapped_v10 = builtin.constant <builtin.integer <44: i8>> : builtin.integer i8 !9;
            wrapped_v7 = llvm.mul hundred_i8_v2, three_i8_v3 <{nsw=false,nuw=false}>: builtin.integer i8 !10;
            no_overflow_v11 = builtin.constant <builtin.integer <30: i8>> : builtin.integer i8 !11;
            no_overflow_v8 = llvm.mul five_i8_v4, six_i8_v5 <{nsw=true,nuw=true}>: builtin.integer i8 !12;
            llvm.return prod_v9 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mul_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        hundred_i8 = builtin.constant <builtin.integer <100: i8>> : builtin.integer i8;
        two_hundred_i8 = llvm.constant <builtin.integer <200: i8>> : builtin.integer i8;
        two_i8 = builtin.constant <builtin.integer <2: i8>> : builtin.integer i8;
        non_constant = llvm.mul x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        signed_overflow = llvm.mul hundred_i8, two_i8 <{nsw=true,nuw=false}> : builtin.integer i8;
        unsigned_overflow = llvm.mul two_hundred_i8, two_i8 <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.shl
// ---------------------------------------------------------------------------

#[test]
fn shl_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        three_i8 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        seven_i8 = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8;
        one_i8 = llvm.constant <builtin.integer <1: i8>> : builtin.integer i8;
        three_bits = llvm.constant <builtin.integer <3: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        wrapped = llvm.shl three_i8, seven_i8 <{nsw=false,nuw=false}> : builtin.integer i8;
        no_overflow = llvm.shl one_i8, three_bits <{nsw=true,nuw=true}> : builtin.integer i8;
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
            three_i8_v2 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !3;
            seven_i8_v3 = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8 !4;
            one_i8_v4 = llvm.constant <builtin.integer <1: i8>> : builtin.integer i8 !5;
            three_bits_v5 = llvm.constant <builtin.integer <3: i8>> : builtin.integer i8 !6;
            shifted_v9 = builtin.constant <builtin.integer <8: i64>> : builtin.integer i64 !7;
            shifted_v6 = llvm.shl a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
            wrapped_v10 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !9;
            wrapped_v7 = llvm.shl three_i8_v2, seven_i8_v3 <{nsw=false,nuw=false}>: builtin.integer i8 !10;
            no_overflow_v11 = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8 !11;
            no_overflow_v8 = llvm.shl one_i8_v4, three_bits_v5 <{nsw=true,nuw=true}>: builtin.integer i8 !12;
            llvm.return shifted_v9 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn shl_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
        ^entry(x: builtin.integer i64):
        c = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        one_i8 = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        width_i8 = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8;
        all_ones_i8 = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        high_bit_i8 = builtin.constant <builtin.integer <64: i8>> : builtin.integer i8;
        one_bit = llvm.constant <builtin.integer <1: i8>> : builtin.integer i8;
        non_constant = llvm.shl x, c <{nsw=false,nuw=false}> : builtin.integer i64;
        too_wide = llvm.shl one_i8, width_i8 <{nsw=false,nuw=false}> : builtin.integer i8;
        unsigned_overflow = llvm.shl all_ones_i8, one_bit <{nsw=false,nuw=true}> : builtin.integer i8;
        signed_overflow = llvm.shl high_bit_i8, one_bit <{nsw=true,nuw=false}> : builtin.integer i8;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
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
fn sdiv_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        zero = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        int_min = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        minus_one = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        by_zero = llvm.sdiv a, zero : builtin.integer i8;
        overflow = llvm.sdiv int_min, minus_one : builtin.integer i8;
        llvm.return by_zero
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
fn srem_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <7: i8>> : builtin.integer i8;
        zero = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        int_min = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        minus_one = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        by_zero = llvm.srem a, zero : builtin.integer i8;
        overflow = llvm.srem int_min, minus_one : builtin.integer i8;
        llvm.return by_zero
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.udiv
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
// llvm.urem
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
fn and_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i1) variadic = false> [] {
        ^entry(x: builtin.integer i1):
        a = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        zero = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
        c = llvm.and a, b : builtin.integer i8;
        masked = llvm.and x, zero : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i1) !0:
            a_v1 = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8 !1;
            b_v2 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !2;
            zero_v3 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !3;
            c_v6 = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8 !4;
            c_v4 = llvm.and a_v1, b_v2 : builtin.integer i8 !5;
            masked_v7 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !6;
            masked_v5 = llvm.and x_v0, zero_v3 : builtin.integer i1 !7;
            llvm.return c_v6 !8
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

// ---------------------------------------------------------------------------
// llvm.or
// ---------------------------------------------------------------------------

#[test]
fn or_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i1) variadic = false> [] {
        ^entry(x: builtin.integer i1):
        a = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8;
        one = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        c = llvm.or a, b : builtin.integer i8;
        saturated = llvm.or x, one : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i1) !0:
            a_v1 = builtin.constant <builtin.integer <12: i8>> : builtin.integer i8 !1;
            b_v2 = builtin.constant <builtin.integer <10: i8>> : builtin.integer i8 !2;
            one_v3 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !3;
            c_v6 = builtin.constant <builtin.integer <14: i8>> : builtin.integer i8 !4;
            c_v4 = llvm.or a_v1, b_v2 : builtin.integer i8 !5;
            saturated_v7 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !6;
            saturated_v5 = llvm.or x_v0, one_v3 : builtin.integer i1 !7;
            llvm.return c_v6 !8
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

// ---------------------------------------------------------------------------
// llvm.xor
// ---------------------------------------------------------------------------

#[test]
fn xor_folds_two_constants() -> Result<()> {
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
fn lshr_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        one = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        width = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8;
        non_constant = llvm.lshr x, one : builtin.integer i8;
        too_wide = llvm.lshr a, width : builtin.integer i8;
        llvm.return non_constant
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
fn ashr_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        a = builtin.constant <builtin.integer <128: i8>> : builtin.integer i8;
        one = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        width = builtin.constant <builtin.integer <8: i8>> : builtin.integer i8;
        non_constant = llvm.ashr x, one : builtin.integer i8;
        too_wide = llvm.ashr a, width : builtin.integer i8;
        llvm.return non_constant
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
fn icmp_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        a2 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8;
        high_bit = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        zero = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        eq_true = llvm.icmp a <EQ> a2 : builtin.integer i1;
        eq_false = llvm.icmp a <EQ> b : builtin.integer i1;
        signed_lt = llvm.icmp high_bit <SLT> zero : builtin.integer i1;
        unsigned_lt = llvm.icmp high_bit <ULT> zero : builtin.integer i1;
        llvm.return eq_true
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
            a2_v1 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !2;
            b_v2 = builtin.constant <builtin.integer <6: i8>> : builtin.integer i8 !3;
            high_bit_v3 = llvm.constant <builtin.integer <-1: i8>> : builtin.integer i8 !4;
            zero_v4 = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8 !5;
            eq_true_v9 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !6;
            eq_true_v5 = llvm.icmp a_v0 <EQ> a2_v1 : builtin.integer i1 !7;
            eq_false_v10 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !8;
            eq_false_v6 = llvm.icmp a_v0 <EQ> b_v2 : builtin.integer i1 !9;
            signed_lt_v11 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !10;
            signed_lt_v7 = llvm.icmp high_bit_v3 <SLT> zero_v4 : builtin.integer i1 !11;
            unsigned_lt_v12 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !12;
            unsigned_lt_v8 = llvm.icmp high_bit_v3 <ULT> zero_v4 : builtin.integer i1 !13;
            llvm.return eq_true_v9 !14
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
fn sext_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        positive = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        negative = llvm.constant <builtin.integer <255: i8>> : builtin.integer i8;
        a = llvm.sext positive to builtin.integer i16;
        b = llvm.sext negative to builtin.integer i16;
        llvm.return b
      }
    "#;
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i16() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            positive_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            negative_v1 = llvm.constant <builtin.integer <-1: i8>> : builtin.integer i8 !2;
            a_v4 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !3;
            a_v2 = llvm.sext positive_v0 to builtin.integer i16 !4;
            b_v5 = builtin.constant <builtin.integer <-1: i16>> : builtin.integer i16 !5;
            b_v3 = llvm.sext negative_v1 to builtin.integer i16 !6;
            llvm.return b_v5 !7
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

#[test]
fn zext_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        positive = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8;
        high_bit = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        a = llvm.zext <nneg=false> positive to builtin.integer i16;
        b = llvm.zext <nneg=false> high_bit to builtin.integer i16;
        c = llvm.zext <nneg=true> positive to builtin.integer i16;
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
            positive_v0 = builtin.constant <builtin.integer <5: i8>> : builtin.integer i8 !1;
            high_bit_v1 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !2;
            a_v5 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !3;
            a_v2 = llvm.zext <nneg=false> positive_v0 to builtin.integer i16 !4;
            b_v6 = builtin.constant <builtin.integer <255: i16>> : builtin.integer i16 !5;
            b_v3 = llvm.zext <nneg=false> high_bit_v1 to builtin.integer i16 !6;
            c_v7 = builtin.constant <builtin.integer <5: i16>> : builtin.integer i16 !7;
            c_v4 = llvm.zext <nneg=true> positive_v0 to builtin.integer i16 !8;
            llvm.return c_v7 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn zext_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 (builtin.integer i8) variadic = false> [] {
        ^entry(x: builtin.integer i8):
        high_bit = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        negative = llvm.zext <nneg=true> high_bit to builtin.integer i16;
        non_constant = llvm.zext <nneg=false> x to builtin.integer i16;
        llvm.return non_constant
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.trunc
// ---------------------------------------------------------------------------

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
// llvm.fptosi
// ---------------------------------------------------------------------------

#[test]
fn fptosi_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        positive = builtin.constant <builtin.single 3.75> : builtin.fp32;
        fraction = builtin.constant <builtin.single 0.75> : builtin.fp32;
        boundary = builtin.constant <builtin.double -128.9> : builtin.fp64;
        negative = builtin.constant <builtin.double -123.75> : builtin.fp64;
        toward_zero = llvm.fptosi positive to builtin.integer i8;
        to_i1 = llvm.fptosi fraction to builtin.integer i1;
        at_lower_boundary = llvm.fptosi boundary to builtin.integer i8;
        negative_toward_zero = llvm.fptosi negative to builtin.integer i8;
        llvm.return negative_toward_zero
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            positive_v0 = builtin.constant <builtin.single 3.75> : builtin.fp32  !1;
            fraction_v1 = builtin.constant <builtin.single 0.75> : builtin.fp32  !2;
            boundary_v2 = builtin.constant <builtin.double -128.90000000000001> : builtin.fp64  !3;
            negative_v3 = builtin.constant <builtin.double -123.75> : builtin.fp64  !4;
            toward_zero_v8 = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8 !5;
            toward_zero_v4 = llvm.fptosi positive_v0 to builtin.integer i8 !6;
            to_i1_v9 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !7;
            to_i1_v5 = llvm.fptosi fraction_v1 to builtin.integer i1 !8;
            at_lower_boundary_v10 = builtin.constant <builtin.integer <-128: i8>> : builtin.integer i8 !9;
            at_lower_boundary_v6 = llvm.fptosi boundary_v2 to builtin.integer i8 !10;
            negative_toward_zero_v11 = builtin.constant <builtin.integer <-123: i8>> : builtin.integer i8 !11;
            negative_toward_zero_v7 = llvm.fptosi negative_v3 to builtin.integer i8 !12;
            llvm.return negative_toward_zero_v11 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn fptosi_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        too_large = builtin.constant <builtin.single 128> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        inf = builtin.constant <builtin.double +Inf> : builtin.fp64;
        one = builtin.constant <builtin.double 1> : builtin.fp64;
        out_of_range = llvm.fptosi too_large to builtin.integer i8;
        not_a_number = llvm.fptosi nan to builtin.integer i32;
        infinity = llvm.fptosi inf to builtin.integer i32;
        wider_than_128_bits = llvm.fptosi one to builtin.integer i129;
        non_constant = llvm.fptosi x to builtin.integer i32;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fptoui
// ---------------------------------------------------------------------------

#[test]
fn fptoui_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        positive = builtin.constant <builtin.half 42> : builtin.fp16;
        negative_fraction = builtin.constant <builtin.double -0.999> : builtin.fp64;
        fraction = builtin.constant <builtin.double 255.9> : builtin.fp64;
        whole = llvm.fptoui positive to builtin.integer i16;
        toward_zero_from_below = llvm.fptoui negative_fraction to builtin.integer i32;
        toward_zero = llvm.fptoui fraction to builtin.integer i8;
        llvm.return toward_zero
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i8() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            positive_v0 = builtin.constant <builtin.half 42> : builtin.fp16  !1;
            negative_fraction_v1 = builtin.constant <builtin.double -0.99899999999999999> : builtin.fp64  !2;
            fraction_v2 = builtin.constant <builtin.double 255.90000000000001> : builtin.fp64  !3;
            whole_v6 = builtin.constant <builtin.integer <42: i16>> : builtin.integer i16 !4;
            whole_v3 = llvm.fptoui positive_v0 to builtin.integer i16 !5;
            toward_zero_from_below_v7 = builtin.constant <builtin.integer <0: i32>> : builtin.integer i32 !6;
            toward_zero_from_below_v4 = llvm.fptoui negative_fraction_v1 to builtin.integer i32 !7;
            toward_zero_v8 = builtin.constant <builtin.integer <-1: i8>> : builtin.integer i8 !8;
            toward_zero_v5 = llvm.fptoui fraction_v2 to builtin.integer i8 !9;
            llvm.return toward_zero_v8 !10
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn fptoui_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        negative = builtin.constant <builtin.single -1> : builtin.fp32;
        too_large = builtin.constant <builtin.single 256> : builtin.fp32;
        nan = builtin.constant <builtin.double NaN> : builtin.fp64;
        inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        one = builtin.constant <builtin.double 1> : builtin.fp64;
        negative_one = llvm.fptoui negative to builtin.integer i32;
        out_of_range = llvm.fptoui too_large to builtin.integer i8;
        not_a_number = llvm.fptoui nan to builtin.integer i32;
        infinity = llvm.fptoui inf to builtin.integer i32;
        wider_than_128_bits = llvm.fptoui one to builtin.integer i129;
        non_constant = llvm.fptoui x to builtin.integer i32;
        llvm.return non_constant
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.extract_value
// ---------------------------------------------------------------------------

#[test]
fn extract_value_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i32 () variadic = false> [] {
        ^entry():
        s = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>> : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>;
        arr = builtin.constant <llvm.aggregate <[builtin.integer <1: i16>, builtin.integer <2: i16>, builtin.integer <3: i16>] : llvm.array [3 x builtin.integer i16]>> : llvm.array [3 x builtin.integer i16];
        nested = builtin.constant <llvm.aggregate <[llvm.aggregate <[builtin.integer <4: i32>, builtin.integer <9: i32>] : llvm.array [2 x builtin.integer i32]>, builtin.integer <7: i8>] : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>>> : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>;
        vec = builtin.constant <llvm.aggregate <[llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 2 x builtin.integer i32>>] : llvm.struct <{ llvm.vector <Fixed x 2 x builtin.integer i32> } : Unpacked>>> : llvm.struct <{ llvm.vector <Fixed x 2 x builtin.integer i32> } : Unpacked>;
        field = llvm.extract_value s [1] : builtin.integer i32;
        element = llvm.extract_value arr [2] : builtin.integer i16;
        deep = llvm.extract_value nested [0, 1] : builtin.integer i32;
        splat_field = llvm.extract_value vec [0] : llvm.vector <Fixed x 2 x builtin.integer i32>;
        llvm.return deep
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i32() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            s_v0 = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>> : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> !1;
            arr_v1 = builtin.constant <llvm.aggregate <[builtin.integer <1: i16>, builtin.integer <2: i16>, builtin.integer <3: i16>] : llvm.array [3 x builtin.integer i16]>> : llvm.array [3 x builtin.integer i16] !2;
            nested_v2 = builtin.constant <llvm.aggregate <[llvm.aggregate <[builtin.integer <4: i32>, builtin.integer <9: i32>] : llvm.array [2 x builtin.integer i32]>, builtin.integer <7: i8>] : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>>> : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked> !3;
            vec_v3 = builtin.constant <llvm.aggregate <[llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 2 x builtin.integer i32>>] : llvm.struct <{ llvm.vector <Fixed x 2 x builtin.integer i32> } : Unpacked>>> : llvm.struct <{ llvm.vector <Fixed x 2 x builtin.integer i32> } : Unpacked> !4;
            field_v8 = builtin.constant <builtin.integer <20: i32>> : builtin.integer i32 !5;
            field_v4 = llvm.extract_value s_v0[1] : builtin.integer i32 !6;
            element_v9 = builtin.constant <builtin.integer <3: i16>> : builtin.integer i16 !7;
            element_v5 = llvm.extract_value arr_v1[2] : builtin.integer i16 !8;
            deep_v10 = builtin.constant <builtin.integer <9: i32>> : builtin.integer i32 !9;
            deep_v6 = llvm.extract_value nested_v2[0, 1] : builtin.integer i32 !10;
            splat_field_v11 = llvm.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 2 x builtin.integer i32>>> : llvm.vector <Fixed x 2 x builtin.integer i32> !11;
            splat_field_v7 = llvm.extract_value vec_v3[0] : llvm.vector <Fixed x 2 x builtin.integer i32> !12;
            llvm.return deep_v10 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn extract_value_does_not_fold_with_non_constant_aggregate() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i32 (llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>) variadic = false> [] {
        ^entry(a: llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>):
        c = llvm.extract_value a [0] : builtin.integer i32;
        llvm.return c
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.insert_value
// ---------------------------------------------------------------------------

#[test]
fn insert_value_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> () variadic = false> [] {
        ^entry():
        s = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>> : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>;
        arr = builtin.constant <llvm.aggregate <[builtin.integer <1: i16>, builtin.integer <2: i16>, builtin.integer <3: i16>] : llvm.array [3 x builtin.integer i16]>> : llvm.array [3 x builtin.integer i16];
        nested = builtin.constant <llvm.aggregate <[llvm.aggregate <[builtin.integer <4: i32>, builtin.integer <9: i32>] : llvm.array [2 x builtin.integer i32]>, builtin.integer <7: i8>] : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>>> : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>;
        v16 = builtin.constant <builtin.integer <8: i16>> : builtin.integer i16;
        v32 = builtin.constant <builtin.integer <11: i32>> : builtin.integer i32;
        v99 = builtin.constant <builtin.integer <99: i32>> : builtin.integer i32;
        element = llvm.insert_value arr [1], v16 : llvm.array [3 x builtin.integer i16];
        deep = llvm.insert_value nested [0, 1], v32 : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>;
        field = llvm.insert_value s [0], v99 : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>;
        llvm.return field
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            s_v0 = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>> : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> !1;
            arr_v1 = builtin.constant <llvm.aggregate <[builtin.integer <1: i16>, builtin.integer <2: i16>, builtin.integer <3: i16>] : llvm.array [3 x builtin.integer i16]>> : llvm.array [3 x builtin.integer i16] !2;
            nested_v2 = builtin.constant <llvm.aggregate <[llvm.aggregate <[builtin.integer <4: i32>, builtin.integer <9: i32>] : llvm.array [2 x builtin.integer i32]>, builtin.integer <7: i8>] : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>>> : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked> !3;
            v16_v3 = builtin.constant <builtin.integer <8: i16>> : builtin.integer i16 !4;
            v32_v4 = builtin.constant <builtin.integer <11: i32>> : builtin.integer i32 !5;
            v99_v5 = builtin.constant <builtin.integer <99: i32>> : builtin.integer i32 !6;
            element_v9 = llvm.constant <llvm.aggregate <[builtin.integer <1: i16>, builtin.integer <8: i16>, builtin.integer <3: i16>] : llvm.array [3 x builtin.integer i16]>> : llvm.array [3 x builtin.integer i16] !7;
            element_v6 = llvm.insert_value arr_v1[1], v16_v3 : llvm.array [3 x builtin.integer i16] !8;
            deep_v10 = llvm.constant <llvm.aggregate <[llvm.aggregate <[builtin.integer <4: i32>, builtin.integer <11: i32>] : llvm.array [2 x builtin.integer i32]>, builtin.integer <7: i8>] : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked>>> : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked> !9;
            deep_v7 = llvm.insert_value nested_v2[0, 1], v32_v4 : llvm.struct <{ llvm.array [2 x builtin.integer i32], builtin.integer i8 } : Unpacked> !10;
            field_v11 = llvm.constant <llvm.aggregate <[builtin.integer <99: i32>, builtin.integer <20: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>> : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> !11;
            field_v8 = llvm.insert_value s_v0[0], v99_v5 : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> !12;
            llvm.return field_v11 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn insert_value_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked> (llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>, builtin.integer i32) variadic = false> [] {
        ^entry(a: llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>, v: builtin.integer i32):
        s = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>] : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>>> : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>;
        v5 = builtin.constant <builtin.integer <5: i32>> : builtin.integer i32;
        non_constant_aggregate = llvm.insert_value a [1], v5 : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>;
        non_constant_value = llvm.insert_value s [1], v : llvm.struct <{ builtin.integer i32, builtin.integer i32 } : Unpacked>;
        llvm.return non_constant_value
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.extractelement
// ---------------------------------------------------------------------------

#[test]
fn extract_element_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i32 () variadic = false> [] {
        ^entry():
        agg = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        splat = builtin.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        two = builtin.constant <builtin.integer <2: i32>> : builtin.integer i32;
        two_i16 = builtin.constant <builtin.integer <2: i16>> : builtin.integer i16;
        one_i1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        from_aggregate = llvm.extractelement agg, two : builtin.integer i32;
        from_splat = llvm.extractelement splat, two_i16 : builtin.integer i32;
        narrow_index = llvm.extractelement agg, one_i1 : builtin.integer i32;
        llvm.return narrow_index
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i32() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            agg_v0 = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !1;
            splat_v1 = builtin.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !2;
            two_v2 = builtin.constant <builtin.integer <2: i32>> : builtin.integer i32 !3;
            two_i16_v3 = builtin.constant <builtin.integer <2: i16>> : builtin.integer i16 !4;
            one_i1_v4 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !5;
            from_aggregate_v8 = builtin.constant <builtin.integer <30: i32>> : builtin.integer i32 !6;
            from_aggregate_v5 = llvm.extractelement agg_v0, two_v2 : builtin.integer i32 !7;
            from_splat_v9 = builtin.constant <builtin.integer <7: i32>> : builtin.integer i32 !8;
            from_splat_v6 = llvm.extractelement splat_v1, two_i16_v3 : builtin.integer i32 !9;
            narrow_index_v10 = builtin.constant <builtin.integer <20: i32>> : builtin.integer i32 !10;
            narrow_index_v7 = llvm.extractelement agg_v0, one_i1_v4 : builtin.integer i32 !11;
            llvm.return narrow_index_v10 !12
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn extract_element_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i32 (llvm.vector <Fixed x 3 x builtin.integer i32>, builtin.integer i32) variadic = false> [] {
        ^entry(v: llvm.vector <Fixed x 3 x builtin.integer i32>, i: builtin.integer i32):
        agg = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        three = builtin.constant <builtin.integer <3: i32>> : builtin.integer i32;
        high_bit = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        one = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32;
        out_of_bounds = llvm.extractelement agg, three : builtin.integer i32;
        unsigned_index = llvm.extractelement agg, high_bit : builtin.integer i32;
        non_constant_vector = llvm.extractelement v, one : builtin.integer i32;
        non_constant_index = llvm.extractelement agg, i : builtin.integer i32;
        llvm.return non_constant_index
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.insertelement
// ---------------------------------------------------------------------------

#[test]
fn insert_element_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.vector <Fixed x 3 x builtin.integer i32> () variadic = false> [] {
        ^entry():
        agg = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        splat = builtin.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        e99 = builtin.constant <builtin.integer <99: i32>> : builtin.integer i32;
        e11 = builtin.constant <builtin.integer <11: i32>> : builtin.integer i32;
        one = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32;
        two = builtin.constant <builtin.integer <2: i32>> : builtin.integer i32;
        into_splat = llvm.insertelement splat, e11, two : llvm.vector <Fixed x 3 x builtin.integer i32>;
        into_aggregate = llvm.insertelement agg, e99, one : llvm.vector <Fixed x 3 x builtin.integer i32>;
        llvm.return into_aggregate
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <llvm.vector <Fixed x 3 x builtin.integer i32>() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            agg_v0 = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !1;
            splat_v1 = builtin.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !2;
            e99_v2 = builtin.constant <builtin.integer <99: i32>> : builtin.integer i32 !3;
            e11_v3 = builtin.constant <builtin.integer <11: i32>> : builtin.integer i32 !4;
            one_v4 = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32 !5;
            two_v5 = builtin.constant <builtin.integer <2: i32>> : builtin.integer i32 !6;
            into_splat_v8 = llvm.constant <llvm.aggregate <[builtin.integer <7: i32>, builtin.integer <7: i32>, builtin.integer <11: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !7;
            into_splat_v6 = llvm.insertelement splat_v1, e11_v3, two_v5 : llvm.vector <Fixed x 3 x builtin.integer i32> !8;
            into_aggregate_v9 = llvm.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <99: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !9;
            into_aggregate_v7 = llvm.insertelement agg_v0, e99_v2, one_v4 : llvm.vector <Fixed x 3 x builtin.integer i32> !10;
            llvm.return into_aggregate_v9 !11
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn insert_element_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.vector <Fixed x 3 x builtin.integer i32> (llvm.vector <Fixed x 3 x builtin.integer i32>, builtin.integer i32, builtin.integer i32) variadic = false> [] {
        ^entry(v: llvm.vector <Fixed x 3 x builtin.integer i32>, e: builtin.integer i32, i: builtin.integer i32):
        agg = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        e99 = builtin.constant <builtin.integer <99: i32>> : builtin.integer i32;
        three = builtin.constant <builtin.integer <3: i32>> : builtin.integer i32;
        one = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32;
        out_of_bounds = llvm.insertelement agg, e99, three : llvm.vector <Fixed x 3 x builtin.integer i32>;
        non_constant_vector = llvm.insertelement v, e99, one : llvm.vector <Fixed x 3 x builtin.integer i32>;
        non_constant_element = llvm.insertelement agg, e, one : llvm.vector <Fixed x 3 x builtin.integer i32>;
        non_constant_index = llvm.insertelement agg, e99, i : llvm.vector <Fixed x 3 x builtin.integer i32>;
        llvm.return non_constant_index
      }
    "#;

    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.shuffle_vector
// ---------------------------------------------------------------------------

#[test]
fn shuffle_vector_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.vector <Fixed x 3 x builtin.integer i32> () variadic = false> [] {
        ^entry():
        a = builtin.constant <llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        b = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        splat7 = builtin.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        splat9 = builtin.constant <llvm.splat <builtin.integer <9: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        interleaved = llvm.shuffle_vector a, b, [0, 4, 2] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        reversed = llvm.shuffle_vector a, b, [5, 4, 3] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        with_splat = llvm.shuffle_vector a, splat9, [3, 1, 5] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        two_splats = llvm.shuffle_vector splat7, splat9, [0, 3, 1] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        llvm.return two_splats
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <llvm.vector <Fixed x 3 x builtin.integer i32>() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !1;
            b_v1 = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !2;
            splat7_v2 = builtin.constant <llvm.splat <builtin.integer <7: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !3;
            splat9_v3 = builtin.constant <llvm.splat <builtin.integer <9: i32> : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !4;
            interleaved_v8 = llvm.constant <llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <20: i32>, builtin.integer <3: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !5;
            interleaved_v4 = llvm.shuffle_vector a_v0, b_v1, [0, 4, 2] : llvm.vector <Fixed x 3 x builtin.integer i32> !6;
            reversed_v9 = llvm.constant <llvm.aggregate <[builtin.integer <30: i32>, builtin.integer <20: i32>, builtin.integer <10: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !7;
            reversed_v5 = llvm.shuffle_vector a_v0, b_v1, [5, 4, 3] : llvm.vector <Fixed x 3 x builtin.integer i32> !8;
            with_splat_v10 = llvm.constant <llvm.aggregate <[builtin.integer <9: i32>, builtin.integer <2: i32>, builtin.integer <9: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !9;
            with_splat_v6 = llvm.shuffle_vector a_v0, splat9_v3, [3, 1, 5] : llvm.vector <Fixed x 3 x builtin.integer i32> !10;
            two_splats_v11 = llvm.constant <llvm.aggregate <[builtin.integer <7: i32>, builtin.integer <9: i32>, builtin.integer <7: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32> !11;
            two_splats_v7 = llvm.shuffle_vector splat7_v2, splat9_v3, [0, 3, 1] : llvm.vector <Fixed x 3 x builtin.integer i32> !12;
            llvm.return two_splats_v11 !13
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn shuffle_vector_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <llvm.vector <Fixed x 3 x builtin.integer i32> (llvm.vector <Fixed x 3 x builtin.integer i32>, llvm.vector <Fixed x 3 x builtin.integer i32>) variadic = false> [] {
        ^entry(x: llvm.vector <Fixed x 3 x builtin.integer i32>, y: llvm.vector <Fixed x 3 x builtin.integer i32>):
        a = builtin.constant <llvm.aggregate <[builtin.integer <1: i32>, builtin.integer <2: i32>, builtin.integer <3: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        b = builtin.constant <llvm.aggregate <[builtin.integer <10: i32>, builtin.integer <20: i32>, builtin.integer <30: i32>] : llvm.vector <Fixed x 3 x builtin.integer i32>>> : llvm.vector <Fixed x 3 x builtin.integer i32>;
        poison_lane = llvm.shuffle_vector a, b, [0, -1, 2] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        non_constant_lhs = llvm.shuffle_vector x, b, [0, 4, 2] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        non_constant_rhs = llvm.shuffle_vector a, y, [0, 4, 2] : llvm.vector <Fixed x 3 x builtin.integer i32>;
        llvm.return non_constant_rhs
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
fn fneg_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        neg_zero = builtin.constant <builtin.single -0.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        neg_inf = builtin.constant <builtin.single -Inf> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        finite = llvm.fneg <> a : builtin.fp32;
        from_neg_zero = llvm.fneg <> neg_zero : builtin.fp32;
        from_pos_inf = llvm.fneg <> pos_inf : builtin.fp32;
        from_neg_inf = llvm.fneg <> neg_inf : builtin.fp32;
        from_nan = llvm.fneg <> nan : builtin.fp32;
        nnan_finite = llvm.fneg <NNAN> a : builtin.fp32;
        llvm.return nnan_finite
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
            neg_zero_v1 = builtin.constant <builtin.single -0> : builtin.fp32  !2;
            pos_inf_v2 = builtin.constant <builtin.single +Inf> : builtin.fp32  !3;
            neg_inf_v3 = builtin.constant <builtin.single -Inf> : builtin.fp32  !4;
            nan_v4 = builtin.constant <builtin.single NaN> : builtin.fp32  !5;
            finite_v11 = builtin.constant <builtin.single -2.5> : builtin.fp32  !6;
            finite_v5 = llvm.fneg <> a_v0 : builtin.fp32  !7;
            from_neg_zero_v12 = builtin.constant <builtin.single 0> : builtin.fp32  !8;
            from_neg_zero_v6 = llvm.fneg <> neg_zero_v1 : builtin.fp32  !9;
            from_pos_inf_v13 = builtin.constant <builtin.single -Inf> : builtin.fp32  !10;
            from_pos_inf_v7 = llvm.fneg <> pos_inf_v2 : builtin.fp32  !11;
            from_neg_inf_v14 = builtin.constant <builtin.single +Inf> : builtin.fp32  !12;
            from_neg_inf_v8 = llvm.fneg <> neg_inf_v3 : builtin.fp32  !13;
            from_nan_v15 = builtin.constant <builtin.single NaN> : builtin.fp32  !14;
            from_nan_v9 = llvm.fneg <> nan_v4 : builtin.fp32  !15;
            nnan_finite_v16 = builtin.constant <builtin.single -2.5> : builtin.fp32  !16;
            nnan_finite_v10 = llvm.fneg <NNAN> a_v0 : builtin.fp32  !17;
            llvm.return nnan_finite_v16 !18
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fneg_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        non_constant = llvm.fneg <> x : builtin.fp32;
        nnan_nan = llvm.fneg <NNAN> nan : builtin.fp32;
        ninf_inf = llvm.fneg <NINF> pos_inf : builtin.fp32;
        llvm.return non_constant
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fadd
// ---------------------------------------------------------------------------

#[test]
fn fadd_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        neg_inf = builtin.constant <builtin.single -Inf> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        finite = llvm.fadd <> a, b : builtin.fp32;
        with_infinity = llvm.fadd <> pos_inf, one : builtin.fp32;
        opposite_infinities = llvm.fadd <> pos_inf, neg_inf : builtin.fp32;
        with_nan = llvm.fadd <> nan, one : builtin.fp32;
        nnan_finite = llvm.fadd <NNAN> a, b : builtin.fp32;
        llvm.return finite
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
            one_v2 = builtin.constant <builtin.single 1> : builtin.fp32  !3;
            pos_inf_v3 = builtin.constant <builtin.single +Inf> : builtin.fp32  !4;
            neg_inf_v4 = builtin.constant <builtin.single -Inf> : builtin.fp32  !5;
            nan_v5 = builtin.constant <builtin.single NaN> : builtin.fp32  !6;
            finite_v11 = builtin.constant <builtin.single 6.5> : builtin.fp32  !7;
            finite_v6 = llvm.fadd <> a_v0, b_v1 : builtin.fp32  !8;
            with_infinity_v12 = builtin.constant <builtin.single +Inf> : builtin.fp32  !9;
            with_infinity_v7 = llvm.fadd <> pos_inf_v3, one_v2 : builtin.fp32  !10;
            opposite_infinities_v13 = builtin.constant <builtin.single NaN> : builtin.fp32  !11;
            opposite_infinities_v8 = llvm.fadd <> pos_inf_v3, neg_inf_v4 : builtin.fp32  !12;
            with_nan_v14 = builtin.constant <builtin.single NaN> : builtin.fp32  !13;
            with_nan_v9 = llvm.fadd <> nan_v5, one_v2 : builtin.fp32  !14;
            nnan_finite_v15 = builtin.constant <builtin.single 6.5> : builtin.fp32  !15;
            nnan_finite_v10 = llvm.fadd <NNAN> a_v0, b_v1 : builtin.fp32  !16;
            llvm.return finite_v11 !17
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fadd_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        neg_inf = builtin.constant <builtin.single -Inf> : builtin.fp32;
        non_constant = llvm.fadd <> x, b : builtin.fp32;
        ninf_inf = llvm.fadd <NINF> pos_inf, one : builtin.fp32;
        nnan_nan = llvm.fadd <NNAN> pos_inf, neg_inf : builtin.fp32;
        llvm.return non_constant
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fsub
// ---------------------------------------------------------------------------

#[test]
fn fsub_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = llvm.constant <builtin.single 2.5> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        pos_inf2 = builtin.constant <builtin.single +Inf> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        finite = llvm.fsub <> a, b : builtin.fp32;
        minus_infinity = llvm.fsub <> one, pos_inf : builtin.fp32;
        equal_infinities = llvm.fsub <> pos_inf, pos_inf2 : builtin.fp32;
        with_nan = llvm.fsub <> nan, one : builtin.fp32;
        llvm.return finite
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
            one_v2 = builtin.constant <builtin.single 1> : builtin.fp32  !3;
            pos_inf_v3 = builtin.constant <builtin.single +Inf> : builtin.fp32  !4;
            pos_inf2_v4 = builtin.constant <builtin.single +Inf> : builtin.fp32  !5;
            nan_v5 = builtin.constant <builtin.single NaN> : builtin.fp32  !6;
            finite_v10 = builtin.constant <builtin.single 7.5> : builtin.fp32  !7;
            finite_v6 = llvm.fsub <> a_v0, b_v1 : builtin.fp32  !8;
            minus_infinity_v11 = builtin.constant <builtin.single -Inf> : builtin.fp32  !9;
            minus_infinity_v7 = llvm.fsub <> one_v2, pos_inf_v3 : builtin.fp32  !10;
            equal_infinities_v12 = builtin.constant <builtin.single NaN> : builtin.fp32  !11;
            equal_infinities_v8 = llvm.fsub <> pos_inf_v3, pos_inf2_v4 : builtin.fp32  !12;
            with_nan_v13 = builtin.constant <builtin.single NaN> : builtin.fp32  !13;
            with_nan_v9 = llvm.fsub <> nan_v5, one_v2 : builtin.fp32  !14;
            llvm.return finite_v10 !15
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fsub_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 2.5> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        pos_inf2 = builtin.constant <builtin.single +Inf> : builtin.fp32;
        non_constant = llvm.fsub <> x, b : builtin.fp32;
        nnan_nan = llvm.fsub <NNAN> pos_inf, pos_inf2 : builtin.fp32;
        llvm.return non_constant
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
fn fmul_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 2.5> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        neg_a = builtin.constant <builtin.single -2.5> : builtin.fp32;
        neg_b = builtin.constant <builtin.single -4.0> : builtin.fp32;
        two = builtin.constant <builtin.single 2.0> : builtin.fp32;
        zero = builtin.constant <builtin.single 0.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        finite = llvm.fmul <> a, b : builtin.fp32;
        negative_operands = llvm.fmul <> neg_a, neg_b : builtin.fp32;
        with_infinity = llvm.fmul <> pos_inf, two : builtin.fp32;
        zero_times_infinity = llvm.fmul <> zero, pos_inf : builtin.fp32;
        with_nan = llvm.fmul <> nan, two : builtin.fp32;
        nnan_finite = llvm.fmul <NNAN> a, b : builtin.fp32;
        llvm.return finite
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
            neg_a_v2 = builtin.constant <builtin.single -2.5> : builtin.fp32  !3;
            neg_b_v3 = builtin.constant <builtin.single -4> : builtin.fp32  !4;
            two_v4 = builtin.constant <builtin.single 2> : builtin.fp32  !5;
            zero_v5 = builtin.constant <builtin.single 0> : builtin.fp32  !6;
            pos_inf_v6 = builtin.constant <builtin.single +Inf> : builtin.fp32  !7;
            nan_v7 = builtin.constant <builtin.single NaN> : builtin.fp32  !8;
            finite_v14 = builtin.constant <builtin.single 10> : builtin.fp32  !9;
            finite_v8 = llvm.fmul <> a_v0, b_v1 : builtin.fp32  !10;
            negative_operands_v15 = builtin.constant <builtin.single 10> : builtin.fp32  !11;
            negative_operands_v9 = llvm.fmul <> neg_a_v2, neg_b_v3 : builtin.fp32  !12;
            with_infinity_v16 = builtin.constant <builtin.single +Inf> : builtin.fp32  !13;
            with_infinity_v10 = llvm.fmul <> pos_inf_v6, two_v4 : builtin.fp32  !14;
            zero_times_infinity_v17 = builtin.constant <builtin.single NaN> : builtin.fp32  !15;
            zero_times_infinity_v11 = llvm.fmul <> zero_v5, pos_inf_v6 : builtin.fp32  !16;
            with_nan_v18 = builtin.constant <builtin.single NaN> : builtin.fp32  !17;
            with_nan_v12 = llvm.fmul <> nan_v7, two_v4 : builtin.fp32  !18;
            nnan_finite_v19 = builtin.constant <builtin.single 10> : builtin.fp32  !19;
            nnan_finite_v13 = llvm.fmul <NNAN> a_v0, b_v1 : builtin.fp32  !20;
            llvm.return finite_v14 !21
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fmul_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        two = builtin.constant <builtin.single 2.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        zero = llvm.constant <builtin.single 0.0> : builtin.fp32;
        inf = llvm.constant <builtin.single +Inf> : builtin.fp32;
        non_constant = llvm.fmul <> x, b : builtin.fp32;
        ninf_inf = llvm.fmul <NINF> pos_inf, two : builtin.fp32;
        nnan_nan = llvm.fmul <NNAN> zero, inf : builtin.fp32;
        llvm.return non_constant
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fdiv
// ---------------------------------------------------------------------------

#[test]
fn fdiv_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        two = builtin.constant <builtin.single 2.0> : builtin.fp32;
        zero = builtin.constant <builtin.single 0.0> : builtin.fp32;
        zero2 = builtin.constant <builtin.single 0.0> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        pos_inf2 = builtin.constant <builtin.single +Inf> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        finite = llvm.fdiv <> a, b : builtin.fp32;
        by_zero = llvm.fdiv <> one, zero : builtin.fp32;
        zero_by_zero = llvm.fdiv <> zero, zero2 : builtin.fp32;
        infinity_by_infinity = llvm.fdiv <> pos_inf, pos_inf2 : builtin.fp32;
        by_infinity = llvm.fdiv <> one, pos_inf : builtin.fp32;
        with_nan = llvm.fdiv <> nan, two : builtin.fp32;
        nnan_finite = llvm.fdiv <NNAN> a, b : builtin.fp32;
        llvm.return finite
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
            one_v2 = builtin.constant <builtin.single 1> : builtin.fp32  !3;
            two_v3 = builtin.constant <builtin.single 2> : builtin.fp32  !4;
            zero_v4 = builtin.constant <builtin.single 0> : builtin.fp32  !5;
            zero2_v5 = builtin.constant <builtin.single 0> : builtin.fp32  !6;
            pos_inf_v6 = builtin.constant <builtin.single +Inf> : builtin.fp32  !7;
            pos_inf2_v7 = builtin.constant <builtin.single +Inf> : builtin.fp32  !8;
            nan_v8 = builtin.constant <builtin.single NaN> : builtin.fp32  !9;
            finite_v16 = builtin.constant <builtin.single 2.5> : builtin.fp32  !10;
            finite_v9 = llvm.fdiv <> a_v0, b_v1 : builtin.fp32  !11;
            by_zero_v17 = builtin.constant <builtin.single +Inf> : builtin.fp32  !12;
            by_zero_v10 = llvm.fdiv <> one_v2, zero_v4 : builtin.fp32  !13;
            zero_by_zero_v18 = builtin.constant <builtin.single NaN> : builtin.fp32  !14;
            zero_by_zero_v11 = llvm.fdiv <> zero_v4, zero2_v5 : builtin.fp32  !15;
            infinity_by_infinity_v19 = builtin.constant <builtin.single NaN> : builtin.fp32  !16;
            infinity_by_infinity_v12 = llvm.fdiv <> pos_inf_v6, pos_inf2_v7 : builtin.fp32  !17;
            by_infinity_v20 = builtin.constant <builtin.single 0> : builtin.fp32  !18;
            by_infinity_v13 = llvm.fdiv <> one_v2, pos_inf_v6 : builtin.fp32  !19;
            with_nan_v21 = builtin.constant <builtin.single NaN> : builtin.fp32  !20;
            with_nan_v14 = llvm.fdiv <> nan_v8, two_v3 : builtin.fp32  !21;
            nnan_finite_v22 = builtin.constant <builtin.single 2.5> : builtin.fp32  !22;
            nnan_finite_v15 = llvm.fdiv <NNAN> a_v0, b_v1 : builtin.fp32  !23;
            llvm.return finite_v16 !24
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn fdiv_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        zero = builtin.constant <builtin.single 0.0> : builtin.fp32;
        zero2 = builtin.constant <builtin.single 0.0> : builtin.fp32;
        zero3 = builtin.constant <builtin.single 0.0> : builtin.fp32;
        non_constant = llvm.fdiv <> x, b : builtin.fp32;
        ninf_by_zero = llvm.fdiv <NINF> one, zero : builtin.fp32;
        nnan_nan = llvm.fdiv <NNAN> zero2, zero3 : builtin.fp32;
        llvm.return non_constant
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.frem
// ---------------------------------------------------------------------------

/// `frem` follows `fmod`: it truncates toward zero and keeps the dividend's sign.
#[test]
fn frem_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.single 10.0> : builtin.fp32;
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        neg_a = builtin.constant <builtin.single -10.0> : builtin.fp32;
        three = builtin.constant <builtin.single 3.0> : builtin.fp32;
        two = builtin.constant <builtin.single 2.0> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        zero = builtin.constant <builtin.single 0.0> : builtin.fp32;
        small = builtin.constant <builtin.single 2.5> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        nan = builtin.constant <builtin.single NaN> : builtin.fp32;
        finite = llvm.frem <> a, b : builtin.fp32;
        negative_dividend = llvm.frem <> neg_a, b : builtin.fp32;
        truncated = llvm.frem <> three, two : builtin.fp32;
        by_zero = llvm.frem <> one, zero : builtin.fp32;
        infinite_dividend = llvm.frem <> pos_inf, two : builtin.fp32;
        infinite_divisor = llvm.frem <> small, pos_inf : builtin.fp32;
        with_nan = llvm.frem <> nan, two : builtin.fp32;
        nnan_finite = llvm.frem <NNAN> a, b : builtin.fp32;
        llvm.return finite
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
            neg_a_v2 = builtin.constant <builtin.single -10> : builtin.fp32  !3;
            three_v3 = builtin.constant <builtin.single 3> : builtin.fp32  !4;
            two_v4 = builtin.constant <builtin.single 2> : builtin.fp32  !5;
            one_v5 = builtin.constant <builtin.single 1> : builtin.fp32  !6;
            zero_v6 = builtin.constant <builtin.single 0> : builtin.fp32  !7;
            small_v7 = builtin.constant <builtin.single 2.5> : builtin.fp32  !8;
            pos_inf_v8 = builtin.constant <builtin.single +Inf> : builtin.fp32  !9;
            nan_v9 = builtin.constant <builtin.single NaN> : builtin.fp32  !10;
            finite_v18 = builtin.constant <builtin.single 2> : builtin.fp32  !11;
            finite_v10 = llvm.frem <> a_v0, b_v1 : builtin.fp32  !12;
            negative_dividend_v19 = builtin.constant <builtin.single -2> : builtin.fp32  !13;
            negative_dividend_v11 = llvm.frem <> neg_a_v2, b_v1 : builtin.fp32  !14;
            truncated_v20 = builtin.constant <builtin.single 1> : builtin.fp32  !15;
            truncated_v12 = llvm.frem <> three_v3, two_v4 : builtin.fp32  !16;
            by_zero_v21 = builtin.constant <builtin.single NaN> : builtin.fp32  !17;
            by_zero_v13 = llvm.frem <> one_v5, zero_v6 : builtin.fp32  !18;
            infinite_dividend_v22 = builtin.constant <builtin.single NaN> : builtin.fp32  !19;
            infinite_dividend_v14 = llvm.frem <> pos_inf_v8, two_v4 : builtin.fp32  !20;
            infinite_divisor_v23 = builtin.constant <builtin.single 2.5> : builtin.fp32  !21;
            infinite_divisor_v15 = llvm.frem <> small_v7, pos_inf_v8 : builtin.fp32  !22;
            with_nan_v24 = builtin.constant <builtin.single NaN> : builtin.fp32  !23;
            with_nan_v16 = llvm.frem <> nan_v9, two_v4 : builtin.fp32  !24;
            nnan_finite_v25 = builtin.constant <builtin.single 2> : builtin.fp32  !25;
            nnan_finite_v17 = llvm.frem <NNAN> a_v0, b_v1 : builtin.fp32  !26;
            llvm.return finite_v18 !27
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn frem_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.fp32 (builtin.fp32) variadic = false> [] {
        ^entry(x: builtin.fp32):
        b = builtin.constant <builtin.single 4.0> : builtin.fp32;
        one = builtin.constant <builtin.single 1.0> : builtin.fp32;
        zero = builtin.constant <builtin.single 0.0> : builtin.fp32;
        small = builtin.constant <builtin.single 2.5> : builtin.fp32;
        pos_inf = builtin.constant <builtin.single +Inf> : builtin.fp32;
        non_constant = llvm.frem <> x, b : builtin.fp32;
        nnan_nan = llvm.frem <NNAN> one, zero : builtin.fp32;
        ninf_inf = llvm.frem <NINF> small, pos_inf : builtin.fp32;
        llvm.return non_constant
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

// ---------------------------------------------------------------------------
// llvm.fcmp
// ---------------------------------------------------------------------------

/// Ordered predicates reject NaN; unordered predicates accept it.
/// IEEE equality treats signed zeros as equal.
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

#[test]
fn select_folds_constants() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64, builtin.integer i1, llvm.vector <Fixed x 4 x builtin.integer i32>, llvm.vector <Fixed x 4 x builtin.integer i32>) variadic = false> [] {
        ^entry(x: builtin.integer i64, unknown_cond: builtin.integer i1, v0: llvm.vector <Fixed x 4 x builtin.integer i32>, v1: llvm.vector <Fixed x 4 x builtin.integer i32>):
        t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        f = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
        a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
        one = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        seven = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
        seven2 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
        taken = llvm.select t ? a : b : builtin.integer i64;
        not_taken = llvm.select f ? a : b : builtin.integer i64;
        from_true = llvm.add taken, one <{nsw=false,nuw=false}> : builtin.integer i64;
        from_false = llvm.add not_taken, one <{nsw=false,nuw=false}> : builtin.integer i64;
        forwarded = llvm.select t ? x : b : builtin.integer i64;
        use_forwarded = llvm.add forwarded, one <{nsw=false,nuw=false}> : builtin.integer i64;
        equal_operands = llvm.select unknown_cond ? seven : seven2 : builtin.integer i64;
        vector_operands = llvm.select f ? v0 : v1 : llvm.vector <Fixed x 4 x builtin.integer i32>;
        use_vector = llvm.select t ? vector_operands : v0 : llvm.vector <Fixed x 4 x builtin.integer i32>;
        llvm.return from_false
      }
    "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64, builtin.integer i1, llvm.vector <Fixed x 4 x builtin.integer i32>, llvm.vector <Fixed x 4 x builtin.integer i32>) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64, unknown_cond_v1: builtin.integer i1, v0_v2: llvm.vector <Fixed x 4 x builtin.integer i32>, v1_v3: llvm.vector <Fixed x 4 x builtin.integer i32>) !0:
            t_v4 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            f_v5 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !2;
            a_v6 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !3;
            b_v7 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !4;
            one_v8 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !5;
            seven_v9 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !6;
            seven2_v10 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !7;
            taken_v11 = llvm.select  t_v4 ? a_v6 : b_v7 : builtin.integer i64 !8;
            not_taken_v12 = llvm.select  f_v5 ? a_v6 : b_v7 : builtin.integer i64 !9;
            from_true_v20 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64 !10;
            from_true_v13 = llvm.add a_v6, one_v8 <{nsw=false,nuw=false}>: builtin.integer i64 !11;
            from_false_v21 = builtin.constant <builtin.integer <21: i64>> : builtin.integer i64 !12;
            from_false_v14 = llvm.add b_v7, one_v8 <{nsw=false,nuw=false}>: builtin.integer i64 !13;
            forwarded_v15 = llvm.select  t_v4 ? x_v0 : b_v7 : builtin.integer i64 !14;
            use_forwarded_v16 = llvm.add x_v0, one_v8 <{nsw=false,nuw=false}>: builtin.integer i64 !15;
            equal_operands_v22 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !16;
            equal_operands_v17 = llvm.select  unknown_cond_v1 ? seven_v9 : seven2_v10 : builtin.integer i64 !17;
            vector_operands_v18 = llvm.select  f_v5 ? v0_v2 : v1_v3 : llvm.vector <Fixed x 4 x builtin.integer i32> !18;
            use_vector_v19 = llvm.select  t_v4 ? v1_v3 : v0_v2 : llvm.vector <Fixed x 4 x builtin.integer i32> !19;
            llvm.return from_false_v21 !20
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn select_does_not_fold() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1, llvm.vector <Fixed x 4 x builtin.integer i1>, llvm.vector <Fixed x 4 x builtin.integer i32>, llvm.vector <Fixed x 4 x builtin.integer i32>) variadic = false> [] {
        ^entry(cond: builtin.integer i1, vector_cond: llvm.vector <Fixed x 4 x builtin.integer i1>, v0: llvm.vector <Fixed x 4 x builtin.integer i32>, v1: llvm.vector <Fixed x 4 x builtin.integer i32>):
        a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
        distinct_operands = llvm.select cond ? a : b : builtin.integer i64;
        vector_condition = llvm.select vector_cond ? v0 : v1 : llvm.vector <Fixed x 4 x builtin.integer i32>;
        llvm.return distinct_operands
      }
    "#;
    let (status, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}
