// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Canonicalization integration tests using textual IR parsing.

use expect_test::expect;
use pliron::{
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::{NOpdsInterface, OneOpdInterface, OneResultInterface},
        ops::ConstantOp,
    },
    context::{Context, Ptr},
    derive::{op_interface_impl, pliron_op},
    graph::walkers::WALKCONFIG_PREORDER_REVERSE,
    init_env_logger_for_tests,
    irbuild::{
        IRStatus,
        match_rewrite::{RewriterOrder, apply_match_rewrite},
        rewriter::Rewriter,
    },
    irfmt::parsers::spaced,
    op::Op,
    operation::{Operation, verify_operation},
    opts::canonicalize::{Canonicalize, CanonicalizeInterface, CanonicalizeRewrite, canonicalize},
    parsable::parse_from_str,
    pass::{AnalysisManager, Pass},
    result::{ExpectOk, Result},
    value::Value,
};

use pliron_llvm as _;

/// Negation. `neg (neg x)` simplifies to `x`.
#[pliron_op(
    name = "test.neg",
    format = "$0 ` : ` type($0)",
    interfaces = [OneOpdInterface, OneResultInterface],
    verifier = "succ"
)]
pub struct NegOp;

#[op_interface_impl]
impl CanonicalizeInterface for NegOp {
    fn canonicalize(&self, ctx: &mut Context, rewriter: &mut dyn Rewriter) -> IRStatus {
        let opd = self.get_operand(ctx);
        let Some(inner_neg) = opd
            .defining_op()
            .and_then(|def| Operation::get_op::<NegOp>(def, ctx))
        else {
            return IRStatus::Unchanged;
        };
        let inner_opd = inner_neg.get_operand(ctx);
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![inner_opd]);
        IRStatus::Changed
    }
}

/// Copy. `copy x` simplifies to `x`.
#[pliron_op(
    name = "test.copy",
    format = "$0 ` : ` type($0)",
    interfaces = [OneOpdInterface, OneResultInterface],
    verifier = "succ"
)]
pub struct CopyOp;

#[op_interface_impl]
impl CanonicalizeInterface for CopyOp {
    fn canonicalize(&self, ctx: &mut Context, rewriter: &mut dyn Rewriter) -> IRStatus {
        let opd = self.get_operand(ctx);
        rewriter.replace_operation_with_values(ctx, self.get_operation(), vec![opd]);
        IRStatus::Changed
    }
}

/// Addition.
/// - A constant lhs moves to the rhs, in place.
/// - `add x, 0` simplifies to `x`.
#[pliron_op(
    name = "test.add",
    format = "$0 `, ` $1 ` : ` type($0)",
    interfaces = [NOpdsInterface<2>, OneResultInterface],
    verifier = "succ"
)]
pub struct AddOp;

/// The constant integer that defines `value`, if any.
fn const_int(ctx: &Context, value: Value) -> Option<IntegerAttr> {
    let def = value.defining_op()?;
    let const_op = Operation::get_op::<ConstantOp>(def, ctx)?;
    let attr = const_op.get_attr_builtin_constant_value(ctx)?;
    attr.downcast_ref::<IntegerAttr>().cloned()
}

#[op_interface_impl]
impl CanonicalizeInterface for AddOp {
    fn canonicalize(&self, ctx: &mut Context, rewriter: &mut dyn Rewriter) -> IRStatus {
        let op = self.get_operation();
        let lhs = op.deref(ctx).get_operand(0);
        let rhs = op.deref(ctx).get_operand(1);
        if const_int(ctx, lhs).is_some() && const_int(ctx, rhs).is_none() {
            Operation::replace_operand(op, ctx, 0, rhs);
            Operation::replace_operand(op, ctx, 1, lhs);
            rewriter.notify_operation_modified(ctx, op);
            return IRStatus::Changed;
        }
        if const_int(ctx, rhs).is_some_and(|rhs| rhs.value().is_zero()) {
            rewriter.replace_operation_with_values(ctx, op, vec![lhs]);
            return IRStatus::Changed;
        }
        IRStatus::Unchanged
    }
}

/// An op without side effects. It is erased when its result is unused.
#[pliron_op(
    name = "test.pure",
    format = "$0 ` : ` type($0)",
    interfaces = [OneOpdInterface, OneResultInterface],
    verifier = "succ"
)]
pub struct PureOp;

#[op_interface_impl]
impl CanonicalizeInterface for PureOp {
    fn canonicalize(&self, ctx: &mut Context, rewriter: &mut dyn Rewriter) -> IRStatus {
        if self.get_result(ctx).is_used(ctx) {
            return IRStatus::Unchanged;
        }
        rewriter.erase_operation(ctx, self.get_operation());
        IRStatus::Changed
    }
}

fn parse(ctx: &mut Context, input: &str) -> Result<Ptr<Operation>> {
    init_env_logger_for_tests!();
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);
    verify_operation(op, ctx)?;
    Ok(op)
}

fn print(ctx: &Context, op: Ptr<Operation>) -> Result<String> {
    verify_operation(op, ctx)?;
    Ok(Operation::get_op_dyn(op, ctx).disp(ctx).to_string())
}

#[test]
fn canonicalize_revisits_users_of_replacement() -> Result<()> {
    // `n2` is visited before `cp`. When `cp` is replaced by `n1`,
    // `n2` must be visited again to simplify `neg (neg c)` to `c`.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      n1 = test.neg c : builtin.integer i64;
      cp = test.copy n1 : builtin.integer i64;
      n2 = test.neg cp : builtin.integer i64;
      llvm.return n2
    }
  "#;
    let ctx = &mut Context::new();
    let op = parse(ctx, input)?;

    let order = RewriterOrder {
        collect: WALKCONFIG_PREORDER_REVERSE,
        ..Default::default()
    };
    let status = apply_match_rewrite(ctx, &mut CanonicalizeRewrite, order, op)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            n1_v1 = test.neg c_v0 : builtin.integer i64 !2;
            llvm.return c_v0 !3
        }"#]]
    .assert_eq(&print(ctx, op)?);
    Ok(())
}

#[test]
fn canonicalize_repeats_on_in_place_change() -> Result<()> {
    // `add 0, x` must change in place to `add x, 0`, and then simplify to `x`.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      zero = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      c = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      x = test.neg c : builtin.integer i64;
      sum = test.add zero, x : builtin.integer i64;
      llvm.return sum
    }
  "#;
    let ctx = &mut Context::new();
    let op = parse(ctx, input)?;

    let status = apply_match_rewrite(ctx, &mut CanonicalizeRewrite, Default::default(), op)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            zero_v0 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !1;
            c_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !2;
            x_v2 = test.neg c_v1 : builtin.integer i64 !3;
            llvm.return x_v2 !4
        }"#]]
    .assert_eq(&print(ctx, op)?);
    Ok(())
}

#[test]
fn canonicalize_reports_in_place_change() -> Result<()> {
    // Swapping the operands of `add 1, x` in place must be reported as a change.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      one = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      c = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      x = test.neg c : builtin.integer i64;
      sum = test.add one, x : builtin.integer i64;
      llvm.return sum
    }
  "#;
    let ctx = &mut Context::new();
    let op = parse(ctx, input)?;

    assert_eq!(canonicalize(op, ctx)?, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            one_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            c_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !2;
            x_v2 = test.neg c_v1 : builtin.integer i64 !3;
            sum_v3 = test.add x_v2, one_v0 : builtin.integer i64 !4;
            llvm.return sum_v3 !5
        }"#]]
    .assert_eq(&print(ctx, op)?);
    Ok(())
}

#[test]
fn canonicalize_reaches_fixpoint() -> Result<()> {
    // Erasing `p2` makes `p1` unused, but the driver does not visit `p1` again.
    // The pass must scan again to erase `p1`.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      p1 = test.pure c : builtin.integer i64;
      p2 = test.pure p1 : builtin.integer i64;
      llvm.return c
    }
  "#;
    let ctx = &mut Context::new();
    let op = parse(ctx, input)?;

    let result = Canonicalize::default().run(op, ctx, &mut AnalysisManager::default())?;
    assert_eq!(result.ir_changed, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            llvm.return c_v0 !2
        }"#]]
    .assert_eq(&print(ctx, op)?);
    Ok(())
}
