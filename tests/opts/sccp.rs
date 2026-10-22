// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! SCCP integration tests using textual LLVM dialect IR parsing.

use expect_test::expect;
use pliron::{
    context::Context,
    init_env_logger_for_tests,
    irbuild::IRStatus,
    irfmt::parsers::spaced,
    operation::{Operation, verify_operation},
    opts::constants::sccp::sccp,
    parsable::parse_from_str,
    result::{ExpectOk, Result},
};

use pliron_llvm as _;

use pliron::{
    attribute::AttrObj,
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::{
            IsTerminatorInterface, NOpdsInterface, NRegionsInterface, NResultsInterface,
            RegionBranchOpInterface, RegionBranchPoint, RegionEdge, RegionSuccessor,
        },
    },
    context::Ptr,
    derive::{op_interface_impl, pliron_op},
    irbuild::rewriter::Rewriter,
    linked_list::ContainsLinkedList,
    op::{Op, op_cast, op_impls},
    opts::{constants::RegionBranchOpFoldInterface, dce::SideEffects},
    region::Region,
    verify_err_noloc,
};

#[pliron_op(
    name = "test.test_region",
    format = "region($0)",
    interfaces = [NOpdsInterface<0>, NResultsInterface<0>],
    verifier = "succ"
)]
pub struct TestRegionOp;

#[pliron_op(
    name = "test.test_two_regions",
    format = "region($0) ` ` region($1)",
    interfaces = [NOpdsInterface<0>, NResultsInterface<0>],
    verifier = "succ"
)]
pub struct TestTwoRegionsOp;

/// The terminator of the tail block of `region`.
fn tail_terminator(ctx: &Context, region: Ptr<Region>) -> Option<Ptr<Operation>> {
    region.deref(ctx).get_tail()?.deref(ctx).get_terminator(ctx)
}

/// Is `attr` a known constant integer that is zero?
/// Returns [None] if `attr` is not a known constant integer.
fn known_int_is_zero(attr: Option<&Option<AttrObj>>) -> Option<bool> {
    attr?
        .as_ref()?
        .downcast_ref::<IntegerAttr>()
        .map(|int| int.value().is_zero())
}

#[derive(thiserror::Error, Debug)]
#[error("each region must end in a terminator with one operand per result")]
pub struct RegionBranchLayoutErr;

/// Terminates a region of [IfOp], [OpaqueIfOp] or [WhenOp].
/// Its operands flow into the results of the parent op.
#[pliron_op(
    name = "test.yield",
    format = "operands(CharSpace(`,`))",
    interfaces = [IsTerminatorInterface],
    verifier = "succ"
)]
pub struct YieldOp;

/// Verify that each region of `op` ends in a terminator with `op_results + extra`
/// operands.
fn verify_tail_terminators(op: Ptr<Operation>, ctx: &Context, extra: usize) -> Result<()> {
    let num_results = op.deref(ctx).get_num_results();
    for region in op.deref(ctx).regions() {
        let Some(terminator) = tail_terminator(ctx, region) else {
            return verify_err_noloc!(RegionBranchLayoutErr);
        };
        if terminator.deref(ctx).get_num_operands() != num_results + extra {
            return verify_err_noloc!(RegionBranchLayoutErr);
        }
    }
    Ok(())
}

/// The edges of an `if`: control enters one of the two regions,
/// and the operands of each region's tail terminator flow into the results.
fn if_edges(op: Ptr<Operation>, ctx: &Context) -> Vec<RegionEdge> {
    let cond = op.deref(ctx).get_operand_as_use(0);
    let results: Vec<_> = op.deref(ctx).results().collect();
    let mut edges = Vec::new();
    for region in op.deref(ctx).regions() {
        edges.push(RegionEdge {
            from: RegionBranchPoint::Parent,
            to: RegionSuccessor::Region(region),
            flows: vec![],
            reads: vec![cond],
        });
        let terminator = tail_terminator(ctx, region).unwrap();
        let flows = (0..results.len())
            .map(|i| terminator.deref(ctx).get_operand_as_use(i))
            .zip(results.iter().copied())
            .collect();
        edges.push(RegionEdge {
            from: RegionBranchPoint::Terminator(terminator),
            to: RegionSuccessor::After,
            flows,
            reads: vec![],
        });
    }
    edges
}

/// An `if`.
/// - Operand 0 is the condition. A non-zero condition enters region 0.
///   A zero condition enters region 1.
/// - The operands of the tail terminator of each region flow into the results.
#[pliron_op(
    name = "test.if",
    format = "$0 ` ` region($0) ` else ` region($1) ` : ` types(CharSpace(`,`))",
    interfaces = [NOpdsInterface<1>, NRegionsInterface<2>],
    verifier = "succ"
)]
pub struct IfOp;

#[op_interface_impl]
impl RegionBranchOpInterface for IfOp {
    fn verify_region_edges_layout(&self, ctx: &Context) -> Result<()> {
        verify_tail_terminators(self.get_operation(), ctx, 0)
    }

    fn region_edges(&self, ctx: &Context) -> Vec<RegionEdge> {
        if_edges(self.get_operation(), ctx)
    }
}

#[op_interface_impl]
impl RegionBranchOpFoldInterface for IfOp {
    fn check_fold(
        &self,
        ctx: &Context,
        point: RegionBranchPoint,
        op_attrs: &[Option<AttrObj>],
        _terminator_attrs: &[Option<AttrObj>],
    ) -> Vec<RegionSuccessor> {
        if let RegionBranchPoint::Terminator(_) = point {
            return vec![RegionSuccessor::After];
        }
        let op = self.get_operation();
        let regions: Vec<_> = op.deref(ctx).regions().collect();
        match known_int_is_zero(op_attrs.first()) {
            Some(false) => vec![RegionSuccessor::Region(regions[0])],
            Some(true) => vec![RegionSuccessor::Region(regions[1])],
            None => regions.into_iter().map(RegionSuccessor::Region).collect(),
        }
    }

    fn fold_in_place(
        &self,
        _ctx: &mut Context,
        _op_attrs: &[Option<AttrObj>],
        _rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
}

/// An `if` that is the same as [IfOp], but does not implement
/// [RegionBranchOpFoldInterface].
#[pliron_op(
    name = "test.if_opaque",
    format = "$0 ` ` region($0) ` else ` region($1) ` : ` types(CharSpace(`,`))",
    interfaces = [NOpdsInterface<1>, NRegionsInterface<2>],
    verifier = "succ"
)]
pub struct OpaqueIfOp;

#[op_interface_impl]
impl RegionBranchOpInterface for OpaqueIfOp {
    fn verify_region_edges_layout(&self, ctx: &Context) -> Result<()> {
        verify_tail_terminators(self.get_operation(), ctx, 0)
    }

    fn region_edges(&self, ctx: &Context) -> Vec<RegionEdge> {
        if_edges(self.get_operation(), ctx)
    }
}

/// Terminates the body of [LoopOp].
/// - Operand 0 is the condition. A non-zero condition repeats the body.
///   A zero condition exits the loop.
/// - The other operands flow into the body arguments or into the results.
#[pliron_op(
    name = "test.loop_condition",
    format = "operands(CharSpace(`,`))",
    interfaces = [IsTerminatorInterface],
    verifier = "succ"
)]
pub struct LoopConditionOp;

/// A loop.
/// - The operands flow into the arguments of the body entry block.
/// - The body ends in a [LoopConditionOp].
#[pliron_op(
    name = "test.loop",
    format = "`(` operands(CharSpace(`,`)) `) ` region($0) ` : ` types(CharSpace(`,`))",
    interfaces = [NRegionsInterface<1>],
    verifier = "succ"
)]
pub struct LoopOp;

#[op_interface_impl]
impl RegionBranchOpInterface for LoopOp {
    fn verify_region_edges_layout(&self, ctx: &Context) -> Result<()> {
        let op = self.get_operation();
        let body = op.deref(ctx).get_region(0);
        let num_results = op.deref(ctx).get_num_results();
        let Some(entry) = body.deref(ctx).get_head() else {
            return verify_err_noloc!(RegionBranchLayoutErr);
        };
        if op.deref(ctx).get_num_operands() != num_results
            || entry.deref(ctx).get_num_arguments() != num_results
        {
            return verify_err_noloc!(RegionBranchLayoutErr);
        }
        verify_tail_terminators(op, ctx, 1)
    }

    fn region_edges(&self, ctx: &Context) -> Vec<RegionEdge> {
        let op = self.get_operation();
        let body = op.deref(ctx).get_region(0);
        let args: Vec<_> = body
            .deref(ctx)
            .get_head()
            .unwrap()
            .deref(ctx)
            .arguments()
            .collect();
        let results: Vec<_> = op.deref(ctx).results().collect();
        let terminator = tail_terminator(ctx, body).unwrap();
        let carried: Vec<_> = (1..terminator.deref(ctx).get_num_operands())
            .map(|i| terminator.deref(ctx).get_operand_as_use(i))
            .collect();
        let cond = terminator.deref(ctx).get_operand_as_use(0);
        vec![
            RegionEdge {
                from: RegionBranchPoint::Parent,
                to: RegionSuccessor::Region(body),
                flows: op.deref(ctx).operands_as_uses().zip(args.clone()).collect(),
                reads: vec![],
            },
            RegionEdge {
                from: RegionBranchPoint::Terminator(terminator),
                to: RegionSuccessor::Region(body),
                flows: carried.iter().copied().zip(args).collect(),
                reads: vec![cond],
            },
            RegionEdge {
                from: RegionBranchPoint::Terminator(terminator),
                to: RegionSuccessor::After,
                flows: carried.into_iter().zip(results).collect(),
                reads: vec![cond],
            },
        ]
    }
}

#[op_interface_impl]
impl RegionBranchOpFoldInterface for LoopOp {
    fn check_fold(
        &self,
        ctx: &Context,
        point: RegionBranchPoint,
        _op_attrs: &[Option<AttrObj>],
        terminator_attrs: &[Option<AttrObj>],
    ) -> Vec<RegionSuccessor> {
        let body = RegionSuccessor::Region(self.get_operation().deref(ctx).get_region(0));
        if let RegionBranchPoint::Parent = point {
            return vec![body];
        }
        match known_int_is_zero(terminator_attrs.first()) {
            Some(false) => vec![body],
            Some(true) => vec![RegionSuccessor::After],
            None => vec![body, RegionSuccessor::After],
        }
    }

    fn fold_in_place(
        &self,
        _ctx: &mut Context,
        _op_attrs: &[Option<AttrObj>],
        _rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
}

/// An op whose region control enters only for a non-zero condition.
/// - Operand 0 is the condition. A non-zero condition enters the region.
///   A zero condition does not enter the region, and the other operands bypass it
///   into the results.
/// - The operands of the tail terminator of the region flow into the results.
#[pliron_op(
    name = "test.when",
    format = "`(` operands(CharSpace(`,`)) `) ` region($0) ` : ` types(CharSpace(`,`))",
    interfaces = [NRegionsInterface<1>],
    verifier = "succ"
)]
pub struct WhenOp;

#[op_interface_impl]
impl RegionBranchOpInterface for WhenOp {
    fn verify_region_edges_layout(&self, ctx: &Context) -> Result<()> {
        let op = self.get_operation();
        if op.deref(ctx).get_num_operands() != op.deref(ctx).get_num_results() + 1 {
            return verify_err_noloc!(RegionBranchLayoutErr);
        }
        verify_tail_terminators(op, ctx, 0)
    }

    fn region_edges(&self, ctx: &Context) -> Vec<RegionEdge> {
        let op = self.get_operation();
        let region = op.deref(ctx).get_region(0);
        let results: Vec<_> = op.deref(ctx).results().collect();
        let cond = op.deref(ctx).get_operand_as_use(0);
        let bypass: Vec<_> = op.deref(ctx).operands_as_uses().skip(1).collect();
        let terminator = tail_terminator(ctx, region).unwrap();
        let yielded: Vec<_> = terminator.deref(ctx).operands_as_uses().collect();
        vec![
            RegionEdge {
                from: RegionBranchPoint::Parent,
                to: RegionSuccessor::Region(region),
                flows: vec![],
                reads: vec![cond],
            },
            RegionEdge {
                from: RegionBranchPoint::Parent,
                to: RegionSuccessor::After,
                flows: bypass.into_iter().zip(results.clone()).collect(),
                reads: vec![cond],
            },
            RegionEdge {
                from: RegionBranchPoint::Terminator(terminator),
                to: RegionSuccessor::After,
                flows: yielded.into_iter().zip(results).collect(),
                reads: vec![],
            },
        ]
    }
}

#[op_interface_impl]
impl RegionBranchOpFoldInterface for WhenOp {
    fn check_fold(
        &self,
        ctx: &Context,
        point: RegionBranchPoint,
        op_attrs: &[Option<AttrObj>],
        _terminator_attrs: &[Option<AttrObj>],
    ) -> Vec<RegionSuccessor> {
        if let RegionBranchPoint::Terminator(_) = point {
            return vec![RegionSuccessor::After];
        }
        let region = RegionSuccessor::Region(self.get_operation().deref(ctx).get_region(0));
        match known_int_is_zero(op_attrs.first()) {
            Some(false) => vec![region],
            Some(true) => vec![RegionSuccessor::After],
            None => vec![region, RegionSuccessor::After],
        }
    }

    fn fold_in_place(
        &self,
        _ctx: &mut Context,
        _op_attrs: &[Option<AttrObj>],
        _rewriter: &mut dyn Rewriter,
    ) -> IRStatus {
        IRStatus::Unchanged
    }
}

#[op_interface_impl]
impl SideEffects for WhenOp {
    /// `test.when` has side effects only through the operations in its region,
    /// other than its terminator.
    fn has_side_effects(&self, ctx: &Context) -> bool {
        let region = self.get_operation().deref(ctx).get_region(0);
        let ops: Vec<_> = region
            .deref(ctx)
            .iter(ctx)
            .flat_map(|block| block.deref(ctx).iter(ctx).collect::<Vec<_>>())
            .collect();
        ops.into_iter().any(|op| {
            let op_dyn = Operation::get_op_dyn(op, ctx);
            !op_impls::<dyn IsTerminatorInterface>(op_dyn.as_ref())
                && op_cast::<dyn SideEffects>(op_dyn.as_ref())
                    .is_none_or(|op| op.has_side_effects(ctx))
        })
    }
}

fn run_sccp_on_text(input: &str) -> Result<(IRStatus, String)> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);

    verify_operation(op, ctx)?;

    let status = sccp(op, ctx)?;

    let after = Operation::get_op_dyn(op, ctx).disp(ctx).to_string();
    log::trace!("After SCCP:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, after))
}

#[test]
fn sccp_folds_add_of_two_constants() -> Result<()> {
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
fn sccp_is_path_sensitive() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
      ^entry(x: builtin.integer i64):
      y = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      one = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if one ^bb0(x, y) else ^bb1(x, y)

      ^bb0(x0: builtin.integer i64,y0: builtin.integer i64):
      llvm.br ^bb2(y0, y0)

      ^bb1(x1: builtin.integer i64,y1: builtin.integer i64):
      llvm.br ^bb2(x1, y1)

      ^bb2(x2: builtin.integer i64,y2: builtin.integer i64):
      z = llvm.add x2, y2 <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return z
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64) !0:
            y_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            one_v2 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !2;
            llvm.cond_br if one_v2 ^bb0_block4v1(x_v0, y_v1) else ^bb1_block5v1(x_v0, y_v1) !3

          ^bb0_block4v1(x0_v3: builtin.integer i64, y0_v4: builtin.integer i64) !4:
            y0_v11 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !5;
            llvm.br ^bb2_block3v3(y0_v11, y0_v11) !6

          ^bb1_block5v1(x1_v5: builtin.integer i64, y1_v6: builtin.integer i64) !7:
            llvm.br ^bb2_block3v3(x1_v5, y1_v6) !8

          ^bb2_block3v3(x2_v7: builtin.integer i64, y2_v8: builtin.integer i64) !9:
            y2_v13 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !10;
            x2_v12 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !11;
            z_v10 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !12;
            z_v9 = llvm.add x2_v12, y2_v13 <{nsw=false,nuw=false}>: builtin.integer i64 !13;
            llvm.return z_v10 !14
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_folded_condition_makes_branch_dead() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
      ^entry(x: builtin.integer i64):
      y = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      zero_i1 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
      one_i1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      one = llvm.add zero_i1, one_i1 <{nsw=false,nuw=false}> : builtin.integer i1;
      llvm.cond_br if one ^bb0(x, y) else ^bb1(x, y)

      ^bb0(x0: builtin.integer i64,y0: builtin.integer i64):
      llvm.br ^bb2(y0, y0)

      ^bb1(x1: builtin.integer i64,y1: builtin.integer i64):
      llvm.br ^bb2(x1, y1)

      ^bb2(x2: builtin.integer i64,y2: builtin.integer i64):
      z = llvm.add x2, y2 <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return z
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64) !0:
            y_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            zero_i1_v2 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !2;
            one_i1_v3 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !3;
            one_v12 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !4;
            one_v4 = llvm.add zero_i1_v2, one_i1_v3 <{nsw=false,nuw=false}>: builtin.integer i1 !5;
            llvm.cond_br if one_v12 ^bb0_block4v1(x_v0, y_v1) else ^bb1_block5v1(x_v0, y_v1) !6

          ^bb0_block4v1(x0_v5: builtin.integer i64, y0_v6: builtin.integer i64) !7:
            y0_v14 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !8;
            llvm.br ^bb2_block3v3(y0_v14, y0_v14) !9

          ^bb1_block5v1(x1_v7: builtin.integer i64, y1_v8: builtin.integer i64) !10:
            llvm.br ^bb2_block3v3(x1_v7, y1_v8) !11

          ^bb2_block3v3(x2_v9: builtin.integer i64, y2_v10: builtin.integer i64) !12:
            y2_v16 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !13;
            x2_v15 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !14;
            z_v13 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !15;
            z_v11 = llvm.add x2_v15, y2_v16 <{nsw=false,nuw=false}>: builtin.integer i64 !16;
            llvm.return z_v13 !17
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_meets_distinct_constants_from_live_predecessors_as_not_a_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      llvm.cond_br if cond ^bb0() else ^bb1()

      ^bb0():
      a0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      b0 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
      llvm.br ^bb2(a0, b0)

      ^bb1():
      a1 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      b1 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
      llvm.br ^bb2(a1, b1)

      ^bb2(x: builtin.integer i64, y: builtin.integer i64):
      x_plus_y = llvm.add x, y <{nsw=false,nuw=false}> : builtin.integer i64;
      y_plus_y = llvm.add y, y <{nsw=false,nuw=false}> : builtin.integer i64;
      result = llvm.add x_plus_y, y_plus_y <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            llvm.cond_br if cond_v0 ^bb0_block4v1() else ^bb1_block5v1() !1

          ^bb0_block4v1() !2:
            a0_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !3;
            b0_v2 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !4;
            llvm.br ^bb2_block3v3(a0_v1, b0_v2) !5

          ^bb1_block5v1() !6:
            a1_v3 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !7;
            b1_v4 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !8;
            llvm.br ^bb2_block3v3(a1_v3, b1_v4) !9

          ^bb2_block3v3(x_v5: builtin.integer i64, y_v6: builtin.integer i64) !10:
            y_v11 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !11;
            x_plus_y_v7 = llvm.add x_v5, y_v11 <{nsw=false,nuw=false}>: builtin.integer i64 !12;
            y_plus_y_v10 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !13;
            y_plus_y_v8 = llvm.add y_v11, y_v11 <{nsw=false,nuw=false}>: builtin.integer i64 !14;
            result_v9 = llvm.add x_plus_y_v7, y_plus_y_v10 <{nsw=false,nuw=false}>: builtin.integer i64 !15;
            llvm.return result_v9 !16
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_is_path_sensitive_2() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
      ^entry(x: builtin.integer i64):
      y = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      one = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if one ^bb1(x, y) else ^bb0(x, y)

      ^bb0(x0: builtin.integer i64,y0: builtin.integer i64):
      llvm.br ^bb2(y0, y0)

      ^bb1(x1: builtin.integer i64,y1: builtin.integer i64):
      llvm.br ^bb2(x1, y1)

      ^bb2(x2: builtin.integer i64,y2: builtin.integer i64):
      z = llvm.add x2, y2 <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return z
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    // Materialized constants inserted into ^bb0, ^bb1, and ^bb2
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64) !0:
            y_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            one_v2 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !2;
            llvm.cond_br if one_v2 ^bb1_block5v1(x_v0, y_v1) else ^bb0_block4v1(x_v0, y_v1) !3

          ^bb0_block4v1(x0_v3: builtin.integer i64, y0_v4: builtin.integer i64) !4:
            llvm.br ^bb2_block2v3(y0_v4, y0_v4) !5

          ^bb1_block5v1(x1_v5: builtin.integer i64, y1_v6: builtin.integer i64) !6:
            y1_v10 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !7;
            llvm.br ^bb2_block2v3(x1_v5, y1_v10) !8

          ^bb2_block2v3(x2_v7: builtin.integer i64, y2_v8: builtin.integer i64) !9:
            y2_v11 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !10;
            z_v9 = llvm.add x2_v7, y2_v11 <{nsw=false,nuw=false}>: builtin.integer i64 !11;
            llvm.return z_v9 !12
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_does_not_fold_when_operands_are_nested_region_entry_args() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      test.test_region {
        ^region_entry(a: builtin.integer i64, b: builtin.integer i64):
        sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return sum
      };
      done = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return done
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            test.test_region 
            {
              ^region_entry_block2v1(a_v0: builtin.integer i64, b_v1: builtin.integer i64) !1:
                sum_v2 = llvm.add a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !2;
                llvm.return sum_v2 !3
            } !4;
            done_v3 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !5;
            llvm.return done_v3 !6
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_folds_inside_nested_region_using_outer_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      outer_a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      outer_b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
      test.test_region {
        ^region_entry():
        inner_sum = llvm.add outer_a, outer_b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return inner_sum
      };
      done = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return done
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            outer_a_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            outer_b_v1 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !2;
            test.test_region 
            {
              ^region_entry_block2v1() !3:
                inner_sum_v4 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !4;
                inner_sum_v2 = llvm.add outer_a_v0, outer_b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !5;
                llvm.return inner_sum_v4 !6
            } !7;
            done_v3 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !8;
            llvm.return done_v3 !9
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_folds_inside_two_nested_regions() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      test.test_two_regions {
        ^r0_entry():
        a0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        b0 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        sum0 = llvm.add a0, b0 <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return sum0
      } {
        ^r1_entry():
        a1 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
        b1 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
        sum1 = llvm.add a1, b1 <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return sum1
      };
      done = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return done
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Both inner adds should fold.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            test.test_two_regions 
            {
              ^r0_entry_block2v1() !1:
                a0_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !2;
                b0_v1 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !3;
                sum0_v7 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !4;
                sum0_v2 = llvm.add a0_v0, b0_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !5;
                llvm.return sum0_v7 !6
            } 
            {
              ^r1_entry_block3v1() !7:
                a1_v3 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !8;
                b1_v4 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !9;
                sum1_v8 = builtin.constant <builtin.integer <30: i64>> : builtin.integer i64 !10;
                sum1_v5 = llvm.add a1_v3, b1_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !11;
                llvm.return sum1_v8 !12
            } !13;
            done_v6 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !14;
            llvm.return done_v6 !15
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_folds_inside_nested_region() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      test.test_region {
        ^region_entry():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        inner_sum = llvm.add a, b <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return inner_sum
      };
      outer = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return outer
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The inner add should fold to 7.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            test.test_region 
            {
              ^region_entry_block2v1() !1:
                a_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !2;
                b_v1 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !3;
                inner_sum_v4 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !4;
                inner_sum_v2 = llvm.add a_v0, b_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !5;
                llvm.return inner_sum_v4 !6
            } !7;
            outer_v3 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !8;
            llvm.return outer_v3 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_materializes_constant_block_arg() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.br ^bb1(c)

      ^bb1(x: builtin.integer i64):
      llvm.return x
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !1;
            llvm.br ^bb1_block3v1(c_v0) !2

          ^bb1_block3v1(x_v1: builtin.integer i64) !3:
            x_v2 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !4;
            llvm.return x_v2 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_materializes_multiple_constant_block_args() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      a0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      b0 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
      a1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      b1 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
      llvm.cond_br if cond ^bb1(a0, b0) else ^bb1(a1, b1)

      ^bb1(x: builtin.integer i64, y: builtin.integer i64):
      llvm.return x
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            a0_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            b0_v2 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !2;
            a1_v3 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !3;
            b1_v4 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !4;
            llvm.cond_br if cond_v0 ^bb1_block3v1(a0_v1, b0_v2) else ^bb1_block3v1(a1_v3, b1_v4) !5

          ^bb1_block3v1(x_v5: builtin.integer i64, y_v6: builtin.integer i64) !6:
            y_v8 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !7;
            x_v7 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !8;
            llvm.return x_v7 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_materializes_constant_carried_through_loop_back_edge() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      c = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.br ^loop(c)

      ^loop(x: builtin.integer i64):
      llvm.cond_br if cond ^loop(x) else ^exit(x)

      ^exit(y: builtin.integer i64):
      llvm.return y
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            c_v1 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !1;
            llvm.br ^loop_block3v1(c_v1) !2

          ^loop_block3v1(x_v2: builtin.integer i64) !3:
            x_v4 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !4;
            llvm.cond_br if cond_v0 ^loop_block3v1(x_v4) else ^exit_block4v1(x_v4) !5

          ^exit_block4v1(y_v3: builtin.integer i64) !6:
            y_v5 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !7;
            llvm.return y_v5 !8
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_loop_back_edge_with_different_constant_meets_to_not_a_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      c1 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.br ^loop(c1)

      ^loop(x: builtin.integer i64):
      c2 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.cond_br if cond ^loop(c2) else ^exit(x)

      ^exit(y: builtin.integer i64):
      llvm.return y
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            c1_v1 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !1;
            llvm.br ^loop_block3v1(c1_v1) !2

          ^loop_block3v1(x_v2: builtin.integer i64) !3:
            c2_v3 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !4;
            llvm.cond_br if cond_v0 ^loop_block3v1(c2_v3) else ^exit_block4v1(x_v2) !5

          ^exit_block4v1(y_v4: builtin.integer i64) !6:
            llvm.return y_v4 !7
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_materialization_replaces_uses_of_block_arg() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.br ^bb1(c)

      ^bb1(califragilistic: builtin.integer i64):
      sum = llvm.add califragilistic, califragilistic <{nsw=false,nuw=false}> : builtin.integer i64;
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
            c_v0 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !1;
            llvm.br ^bb1_block3v1(c_v0) !2

          ^bb1_block3v1(califragilistic_v1: builtin.integer i64) !3:
            califragilistic_v4 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !4;
            sum_v3 = builtin.constant <builtin.integer <84: i64>> : builtin.integer i64 !5;
            sum_v2 = llvm.add califragilistic_v4, califragilistic_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !6;
            llvm.return sum_v3 !7
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_does_not_materialize_not_a_constant_block_arg() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      a0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      a1 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      llvm.cond_br if cond ^bb1(a0) else ^bb1(a1)

      ^bb1(x: builtin.integer i64):
      llvm.return x
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            a0_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            a1_v2 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !2;
            llvm.cond_br if cond_v0 ^bb1_block3v1(a0_v1) else ^bb1_block3v1(a1_v2) !3

          ^bb1_block3v1(x_v3: builtin.integer i64) !4:
            llvm.return x_v3 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_treats_free_variables_as_non_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      outer_three = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      outer_four = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
      outer_true = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      test.test_region {
        ^region_entry():
        inner_sum = llvm.add outer_three, outer_four <{nsw=false,nuw=false}> : builtin.integer i64;
        inner_three = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        llvm.cond_br if outer_true ^from_inner() else ^from_outer()
        ^from_inner():
        llvm.br ^join(inner_three)
        ^from_outer():
        llvm.br ^join(outer_four)
        ^join(joined: builtin.integer i64):
        llvm.return joined
      };
      done = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return done
    }
  "#;

    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let func_op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);
    verify_operation(func_op, ctx)?;

    use pliron::linked_list::ContainsLinkedList;
    let entry_block = func_op
        .deref(ctx)
        .regions()
        .next()
        .unwrap()
        .deref(ctx)
        .get_head()
        .unwrap();
    let region_op = entry_block
        .deref(ctx)
        .iter(ctx)
        .find(|op| Operation::get_opid(*op, ctx).to_string() == "test.test_region")
        .expect("test.test_region op should be in the entry block");

    let status = sccp(region_op, ctx)?;
    verify_operation(func_op, ctx)?;
    let after = Operation::get_op_dyn(func_op, ctx).disp(ctx).to_string();

    // Even though `outer_three` and `outer_four` are syntactically
    // `builtin.constant` ops, the analysis must treat them as NotAConstant when
    // they appear free inside the analysis root, so the inner `llvm.add` must
    // *not* fold. For the same reason, `joined` must not become the constant 3
    // that only the `from_inner` edge carries.
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            outer_three_v0 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            outer_four_v1 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !2;
            outer_true_v2 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !3;
            test.test_region 
            {
              ^region_entry_block2v1() !4:
                inner_sum_v3 = llvm.add outer_three_v0, outer_four_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !5;
                inner_three_v4 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !6;
                llvm.cond_br if outer_true_v2 ^from_inner_block5v1() else ^from_outer_block6v1() !7

              ^from_inner_block5v1() !8:
                llvm.br ^join_block4v3(inner_three_v4) !9

              ^from_outer_block6v1() !10:
                llvm.br ^join_block4v3(outer_four_v1) !11

              ^join_block4v3(joined_v5: builtin.integer i64) !12:
                llvm.return joined_v5 !13
            } !14;
            done_v6 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !15;
            llvm.return done_v6 !16
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_processes_newly_reachable_block_in_order() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i32 (builtin.integer i32, builtin.integer i1) variadic = false> [] {
      ^entry(x: builtin.integer i32, c: builtin.integer i1):
      t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      f = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
      llvm.cond_br if c ^a() else ^b()

      ^a():
      llvm.br ^j(t)

      ^b():
      llvm.br ^j(f)

      ^j(v: builtin.integer i1):
      llvm.cond_br if v ^s() else ^r()

      ^r():
      llvm.return x

      ^s():
      w = llvm.add x, x <{nsw=false,nuw=false}> : builtin.integer i32;
      y = llvm.select v ? w : x : builtin.integer i32;
      llvm.return y
    }
  "#;

    // When `v` drops to NotAConstant, its `cond_br` makes `^s` feasible. The
    // `select` in `^s` also uses `v`, but SCCP must not process it before `w`,
    // which `^s` defines first.
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i32(builtin.integer i32, builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i32, c_v1: builtin.integer i1) !0:
            t_v2 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            f_v3 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !2;
            llvm.cond_br if c_v1 ^a_block4v1() else ^b_block5v1() !3

          ^a_block4v1() !4:
            llvm.br ^j_block3v3(t_v2) !5

          ^b_block5v1() !6:
            llvm.br ^j_block3v3(f_v3) !7

          ^j_block3v3(v_v4: builtin.integer i1) !8:
            llvm.cond_br if v_v4 ^s_block6v3() else ^r_block7v1() !9

          ^r_block7v1() !10:
            llvm.return x_v0 !11

          ^s_block6v3() !12:
            w_v5 = llvm.add x_v0, x_v0 <{nsw=false,nuw=false}>: builtin.integer i32 !13;
            y_v6 = llvm.select  v_v4 ? w_v5 : x_v0 : builtin.integer i32 !14;
            llvm.return y_v6 !15
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn sccp_processes_newly_reachable_loop_exit_in_order() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i32 (builtin.integer i32) variadic = false> [] {
      ^entry(x: builtin.integer i32):
      t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.br ^h(t)

      ^h(v: builtin.integer i1):
      llvm.cond_br if v ^l() else ^b()

      ^l():
      f = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
      llvm.br ^h(f)

      ^b():
      a = llvm.add x, x <{nsw=false,nuw=false}> : builtin.integer i32;
      s = llvm.select v ? a : x : builtin.integer i32;
      llvm.return s
    }
  "#;

    // The back edge drops `v` to NotAConstant, so its `cond_br` makes `^b`
    // feasible. The `select` in `^b` also uses `v`, but SCCP must not process
    // it before `a`, which `^b` defines first.
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i32(builtin.integer i32) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i32) !0:
            t_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            llvm.br ^h_block3v1(t_v1) !2

          ^h_block3v1(v_v2: builtin.integer i1) !3:
            llvm.cond_br if v_v2 ^l_block5v1() else ^b_block2v5() !4

          ^l_block5v1() !5:
            f_v3 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !6;
            llvm.br ^h_block3v1(f_v3) !7

          ^b_block2v5() !8:
            a_v4 = llvm.add x_v0, x_v0 <{nsw=false,nuw=false}>: builtin.integer i32 !9;
            s_v5 = llvm.select  v_v2 ? a_v4 : x_v0 : builtin.integer i32 !10;
            llvm.return s_v5 !11
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that a constant condition makes only one region of a `test.if` reachable,
/// and that a user after the `test.if`, in the same block, must see its constant result.
#[test]
fn sccp_region_branch_prunes_untaken_region() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
      ^entry(x: builtin.integer i64):
      t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      r = test.if t {
        ^then():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        test.yield a
      } else {
        ^else():
        b = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
        c = llvm.add b, b <{nsw=false,nuw=false}> : builtin.integer i64;
        test.yield x
      } : builtin.integer i64;
      s = llvm.add r, r <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return s
    }
  "#;

    // Check that the untaken region stays unreachable: its `llvm.add` stays as it is.
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64) !0:
            t_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            r_v8 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !2;
            r_v5 = test.if t_v1 
            {
              ^then_block2v1() !3:
                a_v2 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !4;
                test.yield a_v2 !5
            } else 
            {
              ^else_block3v1() !6:
                b_v3 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !7;
                c_v4 = llvm.add b_v3, b_v3 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
                test.yield x_v0 !9
            } : builtin.integer i64 !10;
            s_v7 = builtin.constant <builtin.integer <6: i64>> : builtin.integer i64 !11;
            s_v6 = llvm.add r_v8, r_v8 <{nsw=false,nuw=false}>: builtin.integer i64 !12;
            llvm.return s_v7 !13
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that the result of a `test.if` must be a constant when both regions
/// yield the same constant, even if the condition is not a constant.
#[test]
fn sccp_region_branch_meets_equal_constants_from_regions() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      r = test.if cond {
        ^then():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        test.yield a
      } else {
        ^else():
        b = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        c = llvm.add b, b <{nsw=false,nuw=false}> : builtin.integer i64;
        d = llvm.add c, b <{nsw=false,nuw=false}> : builtin.integer i64;
        test.yield d
      } : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            r_v8 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !1;
            r_v5 = test.if cond_v0 
            {
              ^then_block2v1() !2:
                a_v1 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !3;
                test.yield a_v1 !4
            } else 
            {
              ^else_block3v1() !5:
                b_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !6;
                c_v6 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !7;
                c_v3 = llvm.add b_v2, b_v2 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
                d_v7 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !9;
                d_v4 = llvm.add c_v6, b_v2 <{nsw=false,nuw=false}>: builtin.integer i64 !10;
                test.yield d_v7 !11
            } : builtin.integer i64 !12;
            llvm.return r_v8 !13
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that a region branch op without `RegionBranchOpFoldInterface` must keep
/// all of its edges, even with a constant condition.
#[test]
fn sccp_region_branch_without_fold_interface_keeps_all_edges() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64) variadic = false> [] {
      ^entry(x: builtin.integer i64):
      t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      r = test.if_opaque t {
        ^then():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        test.yield a
      } else {
        ^else():
        test.yield x
      } : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64) variadic = false>
          [] 
        {
          ^entry_block1v1(x_v0: builtin.integer i64) !0:
            t_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            r_v3 = test.if_opaque t_v1 
            {
              ^then_block2v1() !2:
                a_v2 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !3;
                test.yield a_v2 !4
            } else 
            {
              ^else_block3v1() !5:
                test.yield x_v0 !6
            } : builtin.integer i64 !7;
            llvm.return r_v3 !8
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that a constant exit condition of a `test.loop` must prune the back edge,
/// so that the loop-carried value and the result are constants.
#[test]
fn sccp_region_branch_prunes_loop_back_edge() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      five = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
      r = test.loop (five) {
        ^body(v: builtin.integer i64):
        f = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
        test.loop_condition f, v
      } : builtin.integer i64;
      s = llvm.add r, r <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return s
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            five_v0 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !1;
            r_v6 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !2;
            r_v3 = test.loop (five_v0) 
            {
              ^body_block2v1(v_v1: builtin.integer i64) !3:
                v_v7 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !4;
                f_v2 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !5;
                test.loop_condition f_v2, v_v7 !6
            } : builtin.integer i64 !7;
            s_v5 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !8;
            s_v4 = llvm.add r_v6, r_v6 <{nsw=false,nuw=false}>: builtin.integer i64 !9;
            llvm.return s_v5 !10
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that the successors of a `cond_br` on the result of a `test.loop`
/// that never exits must stay unreachable.
#[test]
fn sccp_region_branch_loop_without_exit_keeps_successors_unreachable() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      r = test.loop (c) {
        ^body(v: builtin.integer i1):
        t = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
        test.loop_condition t, v
      } : builtin.integer i1;
      llvm.cond_br if r ^a() else ^b()

      ^a():
      one = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      two = llvm.add one, one <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return two

      ^b():
      three = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      six = llvm.add three, three <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return six
    }
  "#;

    // Check that `^a` and `^b` stay unreachable: their `llvm.add`s stay as they are.
    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            r_v3 = test.loop (c_v0) 
            {
              ^body_block2v1(v_v1: builtin.integer i1) !1:
                t_v2 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !2;
                test.loop_condition t_v2, v_v1 !3
            } : builtin.integer i1 !4;
            llvm.cond_br if r_v3 ^a_block5v1() else ^b_block3v3() !5

          ^a_block5v1() !6:
            one_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !7;
            two_v5 = llvm.add one_v4, one_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !8;
            llvm.return two_v5 !9

          ^b_block3v3() !10:
            three_v6 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !11;
            six_v7 = llvm.add three_v6, three_v6 <{nsw=false,nuw=false}>: builtin.integer i64 !12;
            llvm.return six_v7 !13
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that when the region of a `test.when` is unreachable, its result must get the
/// operand that bypasses the region, and that a user after the `test.when` must see
/// that constant.
#[test]
fn sccp_region_branch_bypasses_unreachable_region() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      five = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64;
      f = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1;
      r = test.when (f, five) {
        ^body():
        a = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
        test.yield a
      } : builtin.integer i64;
      s = llvm.add r, r <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return s
    }
  "#;

    let (status, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            five_v0 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !1;
            f_v1 = builtin.constant <builtin.integer <0: i1>> : builtin.integer i1 !2;
            r_v6 = builtin.constant <builtin.integer <5: i64>> : builtin.integer i64 !3;
            r_v3 = test.when (f_v1, five_v0) 
            {
              ^body_block2v1() !4:
                a_v2 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !5;
                test.yield a_v2 !6
            } : builtin.integer i64 !7;
            s_v5 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !8;
            s_v4 = llvm.add r_v6, r_v6 <{nsw=false,nuw=false}>: builtin.integer i64 !9;
            llvm.return s_v5 !10
        }"#]]
    .assert_eq(&after);
    Ok(())
}

/// Check that SCCP on a `test.if` must fold only inside the regions of that `test.if`.
/// The `llvm.add` after the `test.if` is outside the analyzed operation.
#[test]
fn sccp_region_branch_op_as_root_changes_only_its_regions() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      one = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      r = test.if c {
        ^then():
        a = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        b = llvm.add a, a <{nsw=false,nuw=false}> : builtin.integer i64;
        test.yield b
      } else {
        ^else():
        test.yield one
      } : builtin.integer i64;
      s = llvm.add one, one <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return s
    }
  "#;

    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let func_op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);
    verify_operation(func_op, ctx)?;
    let entry_block = func_op
        .deref(ctx)
        .regions()
        .next()
        .unwrap()
        .deref(ctx)
        .get_head()
        .unwrap();
    let if_op = entry_block
        .deref(ctx)
        .iter(ctx)
        .find(|op| Operation::get_opid(*op, ctx).to_string() == "test.if")
        .expect("test.if op should be in the entry block");

    let status = sccp(if_op, ctx)?;
    verify_operation(func_op, ctx)?;
    let after = Operation::get_op_dyn(func_op, ctx).disp(ctx).to_string();
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            one_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            r_v4 = test.if c_v0 
            {
              ^then_block2v1() !2:
                a_v2 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !3;
                b_v6 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !4;
                b_v3 = llvm.add a_v2, a_v2 <{nsw=false,nuw=false}>: builtin.integer i64 !5;
                test.yield b_v6 !6
            } else 
            {
              ^else_block3v1() !7:
                test.yield one_v1 !8
            } : builtin.integer i64 !9;
            s_v5 = llvm.add one_v1, one_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !10;
            llvm.return s_v5 !11
        }"#]]
    .assert_eq(&after);
    Ok(())
}
