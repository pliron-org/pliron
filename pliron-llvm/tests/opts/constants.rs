//! Test that llvm operations implement the constant folding interfaces
//! [ConstFoldInterface] and [BranchOpFoldInterface] correctly

use pliron::{
    combine::Parser,
    context::Context,
    init_env_logger_for_tests,
    irbuild::IRStatus,
    irfmt::parsers::spaced,
    operation::{Operation, verify_operation},
    opts::constants::sccp::sccp,
    parsable::{self, state_stream_from_iterator},
    printable::Printable,
    result::Result,
};

// Linking the crate registers the LLVM dialect
use pliron_llvm as _;

fn run_sccp_on_text(input: &str) -> Result<(IRStatus, String, String)> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        input.chars(),
        parsable::State::new(ctx, pliron::location::Source::InMemory),
    );
    let op = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .expect("textual LLVM IR should parse")
        .0;

    let before = op.disp(ctx).to_string();
    log::trace!("Before SCCP:\n{}", before);
    verify_operation(op, ctx)?;

    let status = sccp(op, ctx)?;

    let after = op.disp(ctx).to_string();
    log::trace!("After SCCP:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, before, after))
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<7: i64>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<128: i8>"));
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}

#[test]
fn add_nuw_does_not_fold_on_unsigned_overflow() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i8 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        sum = llvm.add a, b <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return sum
      }
    "#;

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<7: i8>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<6: i64>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<255: i8>"));
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<6: i8>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<30: i64>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<44: i8>"));
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<30: i8>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<8: i64>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<128: i8>"));
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=false,nuw=true}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, _before, _after) = run_sccp_on_text(input)?;
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
        b = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=true,nuw=false}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, _before, _after) = run_sccp_on_text(input)?;
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
        a = builtin.constant <builtin.integer <1: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <3: i8>> : builtin.integer i8;
        shifted = llvm.shl a, b <{nsw=true,nuw=true}> : builtin.integer i8;
        llvm.return shifted
      }
    "#;

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<8: i8>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<3: i8>"));
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<1: i8>"));
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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

    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<3: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<1: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<8: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert_eq!(after.matches("<0: i1>").count(), 2);
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<14: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert_eq!(after.matches("<1: i1>").count(), 2);
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<6: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<64: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<192: i8>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<1: i1>"));
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<0: i1>"));
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<1: i1>"));
    Ok(())
}

/// 0xff is 255 unsigned, so `ult 0` is false (the same operands compare
/// oppositely to the signed predicate above).
#[test]
fn icmp_unsigned_predicate_treats_high_bit_as_large() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i1 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        b = builtin.constant <builtin.integer <0: i8>> : builtin.integer i8;
        c = llvm.icmp a <ULT> b : builtin.integer i1;
        llvm.return c
      }
    "#;
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<0: i1>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<5: i16>"));
    Ok(())
}

/// A negative value replicates the sign bit:
/// -1 (i8, 0xff) -> -1 (i16, 0xffff == 65535 unsigned).
#[test]
fn sext_folds_negative_constant() -> Result<()> {
    let input = r#"
      llvm.func @f: llvm.func <builtin.integer i16 () variadic = false> [] {
        ^entry():
        a = builtin.constant <builtin.integer <255: i8>> : builtin.integer i8;
        c = llvm.sext a to builtin.integer i16;
        llvm.return c
      }
    "#;
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<65535: i16>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<5: i16>"));
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<255: i16>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
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
    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<5: i16>"));
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
    let (status, _before, _after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    Ok(())
}
