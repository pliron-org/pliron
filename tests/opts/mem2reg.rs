// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for the `mem2reg` optimization pass.

use expect_test::expect;
use pliron::{
    builtin::op_interfaces::{
        IsTerminatorInterface, NOpdsInterface, NRegionsInterface, NResultsInterface,
        RegionBranchOpInterface, RegionBranchPoint, RegionEdge, RegionSuccessor,
    },
    context::{Context, Ptr},
    derive::{op_interface_impl, pliron_op},
    init_env_logger_for_tests,
    irbuild::{IRStatus, rewriter::Rewriter},
    irfmt::parsers::spaced,
    linked_list::ContainsLinkedList,
    op::Op,
    operation::{Operation, verify_operation},
    opts::mem2reg::{
        AllocInfo, PromotableOpInterface, PromotableOpKind, PromotableRegionBranchOpInterface,
        mem2reg,
    },
    parsable::parse_from_str,
    pass::AnalysisManager,
    region::Region,
    result::{ExpectOk, Result},
    r#type::TypeHandle,
    value::Value,
    verify_err_noloc,
};

use crate::sccp::{IfOp, LoopOp, RegionBranchLayoutErr, WhenOp};

use pliron_llvm as _;

#[pliron_op(
  name = "test.region_carrier",
  format = "region($0)",
  interfaces = [NOpdsInterface<0>, NResultsInterface<0>],
  verifier = "succ"
)]
pub struct RegionCarrierOp;

#[pliron_op(
  name = "test.non_promotable_use",
  format = "$0",
  interfaces = [NOpdsInterface<1>, NResultsInterface<0>],
  verifier = "succ"
)]
pub struct NonPromotableUseOp;

#[pliron_op(
  name = "test.region_term",
  format = "`term`",
  interfaces = [NOpdsInterface<0>, NResultsInterface<0>, IsTerminatorInterface],
  verifier = "succ"
)]
pub struct RegionTermOp;

#[pliron_op(
  name = "test.non_branch_succ_term",
  format = "succ($0) `(` operands(CharSpace(`,`)) `)`",
  interfaces = [NOpdsInterface<0>, NResultsInterface<0>, IsTerminatorInterface],
  verifier = "succ"
)]
pub struct NonBranchSuccTermOp;

#[pliron::derive::op_interface_impl]
impl PromotableOpInterface for NonPromotableUseOp {
    fn promotion_kind(&self, _ctx: &Context, _alloc_info: &AllocInfo) -> PromotableOpKind {
        // Explicitly model a use that mem2reg cannot rewrite.
        PromotableOpKind::NonPromotableUse
    }

    fn promote(
        &self,
        _ctx: &mut Context,
        _alloc_info_reaching_defs: &[(AllocInfo, pliron::value::Value)],
        _rewriter: &mut dyn Rewriter,
    ) -> Result<()> {
        unreachable!("NonPromotableUseOp::promote must never be called")
    }
}

/// The terminator of the tail block of `region`.
fn tail_terminator(ctx: &Context, region: Ptr<Region>) -> Ptr<Operation> {
    region
        .deref(ctx)
        .get_tail()
        .and_then(|block| block.deref(ctx).get_terminator(ctx))
        .expect("Region must end in a terminator")
}

/// Push a result of type `ty` to `op` and return it.
fn push_result(ctx: &Context, op: Ptr<Operation>, ty: TypeHandle) -> Value {
    let idx = Operation::push_result(op, ctx, ty);
    op.deref(ctx).get_result(idx)
}

#[op_interface_impl]
impl PromotableRegionBranchOpInterface for IfOp {
    fn stores_visible_in_successors(&self, _ctx: &Context) -> bool {
        true
    }

    fn add_receivers(
        &self,
        ctx: &mut Context,
        successors: &[RegionSuccessor],
        ty: TypeHandle,
    ) -> Vec<(RegionSuccessor, Value)> {
        assert_eq!(successors, [RegionSuccessor::After]);
        vec![(
            RegionSuccessor::After,
            push_result(ctx, self.get_operation(), ty),
        )]
    }

    fn add_carried_operand(
        &self,
        ctx: &mut Context,
        source: RegionBranchPoint,
        _receivers: &[(RegionSuccessor, Value)],
        operand: Value,
    ) {
        let RegionBranchPoint::Terminator(terminator) = source else {
            panic!("Only the terminators of an `if` can be sources");
        };
        Operation::push_operand(terminator, ctx, operand);
    }
}

#[op_interface_impl]
impl PromotableRegionBranchOpInterface for LoopOp {
    fn stores_visible_in_successors(&self, _ctx: &Context) -> bool {
        true
    }

    /// The loop condition forwards the same operands to the body and to the results,
    /// so both always get a receiver.
    fn add_receivers(
        &self,
        ctx: &mut Context,
        _successors: &[RegionSuccessor],
        ty: TypeHandle,
    ) -> Vec<(RegionSuccessor, Value)> {
        let op = self.get_operation();
        let body = op.deref(ctx).get_region(0);
        let entry = body
            .deref(ctx)
            .get_head()
            .expect("Loop body must have a block");
        let arg_idx = pliron::basic_block::BasicBlock::push_argument(entry, ctx, ty);
        let arg = entry.deref(ctx).get_argument(arg_idx);
        vec![
            (RegionSuccessor::Region(body), arg),
            (RegionSuccessor::After, push_result(ctx, op, ty)),
        ]
    }

    fn add_carried_operand(
        &self,
        ctx: &mut Context,
        source: RegionBranchPoint,
        _receivers: &[(RegionSuccessor, Value)],
        operand: Value,
    ) {
        let source_op = match source {
            RegionBranchPoint::Parent => self.get_operation(),
            RegionBranchPoint::Terminator(terminator) => terminator,
        };
        Operation::push_operand(source_op, ctx, operand);
    }
}

#[op_interface_impl]
impl PromotableRegionBranchOpInterface for WhenOp {
    fn stores_visible_in_successors(&self, _ctx: &Context) -> bool {
        true
    }

    fn add_receivers(
        &self,
        ctx: &mut Context,
        successors: &[RegionSuccessor],
        ty: TypeHandle,
    ) -> Vec<(RegionSuccessor, Value)> {
        assert_eq!(successors, [RegionSuccessor::After]);
        vec![(
            RegionSuccessor::After,
            push_result(ctx, self.get_operation(), ty),
        )]
    }

    fn add_carried_operand(
        &self,
        ctx: &mut Context,
        source: RegionBranchPoint,
        _receivers: &[(RegionSuccessor, Value)],
        operand: Value,
    ) {
        let source_op = match source {
            RegionBranchPoint::Parent => self.get_operation(),
            RegionBranchPoint::Terminator(terminator) => terminator,
        };
        Operation::push_operand(source_op, ctx, operand);
    }
}

/// An op whose region runs many times, possibly at the same time.
/// The region ends in a `test.yield` without operands.
#[pliron_op(
  name = "test.parallel",
  format = "region($0)",
  interfaces = [NOpdsInterface<0>, NResultsInterface<0>, NRegionsInterface<1>],
  verifier = "succ"
)]
pub struct ParallelOp;

#[op_interface_impl]
impl RegionBranchOpInterface for ParallelOp {
    fn verify_region_edges_layout(&self, ctx: &Context) -> Result<()> {
        let body = self.get_operation().deref(ctx).get_region(0);
        let tail = body.deref(ctx).get_tail();
        match tail {
            Some(tail) if tail.deref(ctx).get_terminator(ctx).is_some() => Ok(()),
            _ => verify_err_noloc!(RegionBranchLayoutErr),
        }
    }

    fn region_edges(&self, ctx: &Context) -> Vec<RegionEdge> {
        let body = self.get_operation().deref(ctx).get_region(0);
        vec![
            RegionEdge {
                from: RegionBranchPoint::Parent,
                to: RegionSuccessor::Region(body),
                flows: vec![],
                reads: vec![],
            },
            RegionEdge {
                from: RegionBranchPoint::Terminator(tail_terminator(ctx, body)),
                to: RegionSuccessor::After,
                flows: vec![],
                reads: vec![],
            },
        ]
    }
}

#[op_interface_impl]
impl PromotableRegionBranchOpInterface for ParallelOp {
    fn stores_visible_in_successors(&self, _ctx: &Context) -> bool {
        false
    }

    fn add_receivers(
        &self,
        _ctx: &mut Context,
        _successors: &[RegionSuccessor],
        _ty: TypeHandle,
    ) -> Vec<(RegionSuccessor, Value)> {
        unreachable!("The stores of a parallel op are not visible in its successors")
    }

    fn add_carried_operand(
        &self,
        _ctx: &mut Context,
        _source: RegionBranchPoint,
        _receivers: &[(RegionSuccessor, Value)],
        _operand: Value,
    ) {
        unreachable!("The stores of a parallel op are not visible in its successors")
    }
}

fn run_mem2reg(input: &str) -> Result<(IRStatus, String)> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);

    verify_operation(op, ctx)?;

    let mut analyses = AnalysisManager::default();
    let status = mem2reg(op, ctx, &mut analyses)?;

    let after = Operation::get_op_dyn(op, ctx).disp(ctx).to_string();
    log::trace!("After mem2reg:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, after))
}

#[test]
fn mem2reg_basic_store_and_load() -> Result<()> {
    // Test basic allocation, store, and load in a single block.
    // Expected: alloca removed, load replaced with constant value, store removed.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      stored_val = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.store *alloc <- stored_val;
      loaded_val = llvm.load alloc : builtin.integer i64;
      llvm.return loaded_val
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca should be removed.
    // Store should be removed.
    // Load should be removed (replaced with the constant).
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            stored_val_v2 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !2;
            llvm.return stored_val_v2 !3
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_multiple_stores_one_load() -> Result<()> {
    // Test multiple stores with only the last value loaded.
    // Expected: first store is dead, only last store value propagates.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.store *alloc <- val1;
      val2 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.store *alloc <- val2;
      loaded = llvm.load alloc : builtin.integer i64;
      llvm.return loaded
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Should contain the final stored value.
    // Alloca and stores removed.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            val1_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !2;
            val2_v3 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !3;
            llvm.return val2_v3 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_no_store_uses_default() -> Result<()> {
    // Test allocation with no store - should use default value (poison).
    // Expected: load replaced with poison value.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      loaded = llvm.load alloc : builtin.integer i64;
      llvm.return loaded
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca removed.
    // Load removed.
    // Should have a poison operation.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            loaded_v3 = llvm.poison : builtin.integer i64 !2;
            llvm.return loaded_v3 !3
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_no_load_dead_allocation() -> Result<()> {
    // Test allocation with store but no load - should be eliminated completely.
    // Expected: entire allocation and store removed.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.store *alloc <- val;
      dead_val = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.return dead_val
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca and store should be removed.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            val_v2 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !2;
            dead_val_v3 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !3;
            llvm.return dead_val_v3 !4
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_phi_with_conditional_branch() -> Result<()> {
    // Test conditional branch requiring phi insertion for allocated value.
    // Expected: phis created, alloca removed, loads replaced with phi arguments.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      llvm.cond_br if cond ^then() else ^else()

      ^then():
      val_then = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.store *alloc <- val_then;
      llvm.br ^merge()

      ^else():
      val_else = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.store *alloc <- val_else;
      llvm.br ^merge()

      ^merge():
      result = llvm.load alloc : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca removed.
    // Stores removed (phis created instead).
    // Load removed.
    // Merge still exists and branch forwarding got materialized via successor operands.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            llvm.cond_br if cond_v0 ^then_block4v1() else ^else_block5v1() !2

          ^then_block4v1() !3:
            val_then_v3 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !4;
            llvm.br ^merge_block3v3(val_then_v3) !5

          ^else_block5v1() !6:
            val_else_v4 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !7;
            llvm.br ^merge_block3v3(val_else_v4) !8

          ^merge_block3v3(alloc_v6: builtin.integer i64) !9:
            llvm.return alloc_v6 !10
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_multiple_allocations() -> Result<()> {
    // Test multiple independent allocations in same block.
    // Expected: all allocations promoted independently.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc1 = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      alloc2 = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val1 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
      val2 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
      llvm.store *alloc1 <- val1;
      llvm.store *alloc2 <- val2;
      load1 = llvm.load alloc1 : builtin.integer i64;
      load2 = llvm.load alloc2 : builtin.integer i64;
      result = llvm.add load1, load2 <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Both allocas removed.
    // All stores removed.
    // Both loads removed.
    // The add operation should work with the promoted values.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            val1_v3 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !2;
            val2_v4 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !3;
            result_v7 = llvm.add val1_v3, val2_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return result_v7 !5
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_linear_chain_of_stores_and_loads() -> Result<()> {
    // Test a linear chain: store, load, store, load pattern.
    // Expected: all intermediate values correctly threaded.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.store *alloc <- val1;
      load1 = llvm.load alloc : builtin.integer i64;
      llvm.return load1
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            val1_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !2;
            llvm.return val1_v2 !3
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_diamond_pattern() -> Result<()> {
    // Test diamond control flow (two paths merge back together).
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      init_val = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.store *alloc <- init_val;
      llvm.cond_br if cond ^then() else ^else()

      ^then():
      then_val = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
      llvm.store *alloc <- then_val;
      llvm.br ^merge()

      ^else():
      else_val = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
      llvm.store *alloc <- else_val;
      llvm.br ^merge()

      ^merge():
      result = llvm.load alloc : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            init_val_v3 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            llvm.cond_br if cond_v0 ^then_block4v1() else ^else_block5v1() !3

          ^then_block4v1() !4:
            then_val_v4 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !5;
            llvm.br ^merge_block3v3(then_val_v4) !6

          ^else_block5v1() !7:
            else_val_v5 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !8;
            llvm.br ^merge_block3v3(else_val_v5) !9

          ^merge_block3v3(alloc_v7: builtin.integer i64) !10:
            llvm.return alloc_v7 !11
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_nested_branches() -> Result<()> {
    // Test nested if-then-else structures.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1, builtin.integer i1) variadic = false> [] {
      ^entry(cond1: builtin.integer i1, cond2: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val0 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.store *alloc <- val0;
      llvm.cond_br if cond1 ^if1_then() else ^if1_else()

      ^if1_then():
      val1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.store *alloc <- val1;
      llvm.cond_br if cond2 ^if2_then() else ^if2_else()

      ^if2_then():
      val2 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.store *alloc <- val2;
      llvm.br ^merge()

      ^if2_else():
      val3 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      llvm.store *alloc <- val3;
      llvm.br ^merge()

      ^if1_else():
      val4 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
      llvm.store *alloc <- val4;
      llvm.br ^merge()

      ^merge():
      result = llvm.load alloc : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1, builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond1_v0: builtin.integer i1, cond2_v1: builtin.integer i1) !0:
            size_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            val0_v4 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            llvm.cond_br if cond1_v0 ^if1_then_block4v1() else ^if1_else_block5v3() !3

          ^if1_then_block4v1() !4:
            val1_v5 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !5;
            llvm.cond_br if cond2_v1 ^if2_then_block6v1() else ^if2_else_block7v1() !6

          ^if2_then_block6v1() !7:
            val2_v6 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !8;
            llvm.br ^merge_block3v3(val2_v6) !9

          ^if2_else_block7v1() !10:
            val3_v7 = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64 !11;
            llvm.br ^merge_block3v3(val3_v7) !12

          ^if1_else_block5v3() !13:
            val4_v8 = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64 !14;
            llvm.br ^merge_block3v3(val4_v8) !15

          ^merge_block3v3(alloc_v10: builtin.integer i64) !16:
            llvm.return alloc_v10 !17
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_unused_block_arguments() -> Result<()> {
    // Test removal of block arguments that are not used (dead phi values).
    // When a phi is created but never used, DCE should remove it.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val_then = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      val_else = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.cond_br if cond ^then() else ^else()

      ^then():
      llvm.store *alloc <- val_then;
      llvm.br ^merge()

      ^else():
      llvm.store *alloc <- val_else;
      llvm.br ^merge()

      ^merge():
      unused = llvm.load alloc : builtin.integer i64;
      ret_val = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return ret_val
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    // Stores are dead once promoted, so they should be eliminated along with the alloca.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            val_then_v3 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !2;
            val_else_v4 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !3;
            llvm.cond_br if cond_v0 ^then_block4v1() else ^else_block5v1() !4

          ^then_block4v1() !5:
            llvm.br ^merge_block3v3(val_then_v3) !6

          ^else_block5v1() !7:
            llvm.br ^merge_block3v3(val_else_v4) !8

          ^merge_block3v3(alloc_v7: builtin.integer i64) !9:
            ret_val_v6 = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64 !10;
            llvm.return ret_val_v6 !11
        }"#]]
    .assert_eq(&after);
    assert_eq!(status, IRStatus::Changed);
    Ok(())
}

#[test]
fn mem2reg_multiple_paths_convergence() -> Result<()> {
    // Test multiple paths (more than 2) converging to a merge point.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1, builtin.integer i1) variadic = false> [] {
      ^entry(cond1: builtin.integer i1, cond2: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v0 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.store *alloc <- v0;
      llvm.cond_br if cond1 ^path1() else ^path2()

      ^path1():
      v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.store *alloc <- v1;
      llvm.cond_br if cond2 ^path1a() else ^path1b()

      ^path1a():
      v1a = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
      llvm.store *alloc <- v1a;
      llvm.br ^merge()

      ^path1b():
      v1b = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64;
      llvm.store *alloc <- v1b;
      llvm.br ^merge()

      ^path2():
      v2 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.store *alloc <- v2;
      llvm.br ^merge()

      ^merge():
      result = llvm.load alloc : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1, builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond1_v0: builtin.integer i1, cond2_v1: builtin.integer i1) !0:
            size_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v0_v4 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            llvm.cond_br if cond1_v0 ^path1_block4v1() else ^path2_block5v3() !3

          ^path1_block4v1() !4:
            v1_v5 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !5;
            llvm.cond_br if cond2_v1 ^path1a_block6v1() else ^path1b_block7v1() !6

          ^path1a_block6v1() !7:
            v1a_v6 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !8;
            llvm.br ^merge_block3v3(v1a_v6) !9

          ^path1b_block7v1() !10:
            v1b_v7 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64 !11;
            llvm.br ^merge_block3v3(v1b_v7) !12

          ^path2_block5v3() !13:
            v2_v8 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !14;
            llvm.br ^merge_block3v3(v2_v8) !15

          ^merge_block3v3(alloc_v10: builtin.integer i64) !16:
            llvm.return alloc_v10 !17
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_load_before_any_store() -> Result<()> {
    // Test load before any store - should use default value.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      first_load = llvm.load alloc : builtin.integer i64;
      store_val = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.store *alloc <- store_val;
      second_load = llvm.load alloc : builtin.integer i64;
      result = llvm.add first_load, second_load <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Should have poison for the uninitialized load.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            first_load_v6 = llvm.poison : builtin.integer i64 !2;
            store_val_v3 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !3;
            result_v5 = llvm.add first_load_v6, store_val_v3 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return result_v5 !5
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_complex_liveness() -> Result<()> {
    // Test complex liveness scenario where phis are needed in multiple blocks.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      init = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.store *alloc <- init;
      loaded1 = llvm.load alloc : builtin.integer i64;
      llvm.cond_br if cond ^then() else ^else()

      ^then():
      val_then = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
      llvm.store *alloc <- val_then;
      llvm.br ^merge()

      ^else():
      val_else = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
      llvm.store *alloc <- val_else;
      loaded_else = llvm.load alloc : builtin.integer i64;
      llvm.br ^merge()

      ^merge():
      loaded2 = llvm.load alloc : builtin.integer i64;
      result = llvm.add loaded2, loaded2 <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            init_v3 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            llvm.cond_br if cond_v0 ^then_block4v1() else ^else_block5v1() !3

          ^then_block4v1() !4:
            val_then_v5 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64 !5;
            llvm.br ^merge_block3v3(val_then_v5) !6

          ^else_block5v1() !7:
            val_else_v6 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64 !8;
            llvm.br ^merge_block3v3(val_else_v6) !9

          ^merge_block3v3(alloc_v10: builtin.integer i64) !10:
            result_v9 = llvm.add alloc_v10, alloc_v10 <{nsw=false,nuw=false}>: builtin.integer i64 !11;
            llvm.return result_v9 !12
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_no_promotion_when_alloca_address_escapes() -> Result<()> {
    // Test that allocations whose address escapes are not promoted.
    // This is currently handled by the interface pruning logic.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      val = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.store *alloc <- val;
      loaded = llvm.load alloc : builtin.integer i64;
      casted = llvm.ptrtoint alloc to builtin.integer i64;
      result = llvm.add loaded, casted <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return result
    }
  "#;

    // The allocation should not be promoted because its address is used.
    // However, some loads/stores might still be promotable depending on implementation.
    // This test documents the expected behavior.
    let (_status, after) = run_mem2reg(input)?;
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            alloc_v1 = llvm.alloca [builtin.integer i64 x size_v0]  : llvm.ptr (0) !2;
            val_v2 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !3;
            llvm.store *alloc_v1 <- val_v2  !4;
            loaded_v3 = llvm.load alloc_v1  : builtin.integer i64 !5;
            casted_v4 = llvm.ptrtoint alloc_v1 to builtin.integer i64 !6;
            result_v5 = llvm.add loaded_v3, casted_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !7;
            llvm.return result_v5 !8
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_repeated_forward_edges() -> Result<()> {
    // Test case with repeated forward edges (e.g., multiple branches to same target).
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1, builtin.integer i1) variadic = false> [] {
      ^entry(cond1: builtin.integer i1, cond2: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v0 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.store *alloc <- v0;
      llvm.cond_br if cond1 ^block_a() else ^block_b()

      ^block_a():
      v_a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.store *alloc <- v_a;
      llvm.cond_br if cond2 ^merge() else ^merge()

      ^block_b():
      v_b = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.store *alloc <- v_b;
      llvm.br ^merge()

      ^merge():
      result = llvm.load alloc : builtin.integer i64;
      llvm.return result
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1, builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond1_v0: builtin.integer i1, cond2_v1: builtin.integer i1) !0:
            size_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v0_v4 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            llvm.cond_br if cond1_v0 ^block_a_block4v1() else ^block_b_block5v1() !3

          ^block_a_block4v1() !4:
            v_a_v5 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !5;
            llvm.cond_br if cond2_v1 ^merge_block3v3(v_a_v5) else ^merge_block3v3(v_a_v5) !6

          ^block_b_block5v1() !7:
            v_b_v6 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !8;
            llvm.br ^merge_block3v3(v_b_v6) !9

          ^merge_block3v3(alloc_v8: builtin.integer i64) !10:
            llvm.return alloc_v8 !11
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_not_promoted_when_load_is_in_nested_region() -> Result<()> {
    // A load in the region of `test.region_carrier`, which does not implement
    // PromotableRegionBranchOpInterface, must block promotion.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      test.region_carrier {
        ^nested():
        inner = llvm.load alloc : builtin.integer i64;
        test.region_term term
      };
      llvm.return v
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            alloc_v1 = llvm.alloca [builtin.integer i64 x size_v0]  : llvm.ptr (0) !2;
            v_v2 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !3;
            llvm.store *alloc_v1 <- v_v2  !4;
            test.region_carrier 
            {
              ^nested_block2v1() !5:
                inner_v3 = llvm.load alloc_v1  : builtin.integer i64 !6;
                test.region_term term !7
            } !8;
            llvm.return v_v2 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_not_promoted_when_store_is_in_nested_region() -> Result<()> {
    // A store in the region of `test.region_carrier`, which does not implement
    // PromotableRegionBranchOpInterface, must block promotion.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      test.region_carrier {
        ^nested():
        llvm.store *alloc <- v;
        test.region_term term
      };
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            alloc_v1 = llvm.alloca [builtin.integer i64 x size_v0]  : llvm.ptr (0) !2;
            v_v2 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !3;
            test.region_carrier 
            {
              ^nested_block2v1() !4:
                llvm.store *alloc_v1 <- v_v2  !5;
                test.region_term term !6
            } !7;
            out_v3 = llvm.load alloc_v1  : builtin.integer i64 !8;
            llvm.return out_v3 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_not_promoted_for_interface_declared_non_promotable_use() -> Result<()> {
    // Interface-specific corner case: use in same region but explicitly non-promotable.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <13: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      test.non_promotable_use alloc;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            alloc_v1 = llvm.alloca [builtin.integer i64 x size_v0]  : llvm.ptr (0) !2;
            v_v2 = builtin.constant <builtin.integer <13: i64>> : builtin.integer i64 !3;
            llvm.store *alloc_v1 <- v_v2  !4;
            test.non_promotable_use alloc_v1 !5;
            out_v3 = llvm.load alloc_v1  : builtin.integer i64 !6;
            llvm.return out_v3 !7
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_not_promoted_when_phi_pred_has_non_branch_successor_terminator() -> Result<()> {
    // Interface-specific CFG corner case: a predecessor reaches the merge block with a
    // successor-bearing terminator that does not implement BranchOpInterface.
    // mem2reg should prune this candidate rather than attempting phi operand insertion.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(cond: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      llvm.cond_br if cond ^left() else ^right()

      ^left():
      lv = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64;
      llvm.store *alloc <- lv;
      llvm.br ^merge()

      ^right():
      rv = builtin.constant <builtin.integer <22: i64>> : builtin.integer i64;
      llvm.store *alloc <- rv;
      test.non_branch_succ_term ^merge()

      ^merge():
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(cond_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            alloc_v2 = llvm.alloca [builtin.integer i64 x size_v1]  : llvm.ptr (0) !2;
            llvm.cond_br if cond_v0 ^left_block4v1() else ^right_block5v1() !3

          ^left_block4v1() !4:
            lv_v3 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64 !5;
            llvm.store *alloc_v2 <- lv_v3  !6;
            llvm.br ^merge_block3v3() !7

          ^right_block5v1() !8:
            rv_v4 = builtin.constant <builtin.integer <22: i64>> : builtin.integer i64 !9;
            llvm.store *alloc_v2 <- rv_v4  !10;
            test.non_branch_succ_term ^merge_block3v3() !11

          ^merge_block3v3() !12:
            out_v5 = llvm.load alloc_v2  : builtin.integer i64 !13;
            llvm.return out_v5 !14
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_alloca_inside_loop_body() -> Result<()> {
    // An allocation inside a loop body, conditionally stored to. The promoted value is
    // live at the loop entry, so placing a poison value just before the alloca isn't good
    // enough, it must be placed in the entry block (i.e., dominate any use of the promoted
    // value, not just the alloca itself).
    // In other words, the places where the promoted value may be live need not be dominated
    // by the alloca itself.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i64, builtin.integer i1) variadic = false> [] {
      ^entry(n: builtin.integer i64, cond: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      zero = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      one = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.br ^header(zero)

      ^header(i: builtin.integer i64):
      continue_loop = llvm.icmp i <ULT> n : builtin.integer i1;
      llvm.cond_br if continue_loop ^body() else ^exit()

      ^body():
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      llvm.cond_br if cond ^then() else ^merge()

      ^then():
      v = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      llvm.br ^merge()

      ^merge():
      out = llvm.load alloc : builtin.integer i64;
      next_i = llvm.add i, one <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.br ^header(next_i)

      ^exit():
      llvm.return zero
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The uninitialized path through ^body needs a default value.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i64, builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(n_v0: builtin.integer i64, cond_v1: builtin.integer i1) !0:
            size_v2 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            zero_v3 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            one_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !3;
            v13 = llvm.poison : builtin.integer i64;
            llvm.br ^header_block3v1(zero_v3, v13) !4

          ^header_block3v1(i_v5: builtin.integer i64, alloc_v12: builtin.integer i64) !5:
            continue_loop_v6 = llvm.icmp i_v5 <ULT> n_v0 : builtin.integer i1 !6;
            llvm.cond_br if continue_loop_v6 ^body_block5v1() else ^exit_block6v3() !7

          ^body_block5v1() !8:
            llvm.cond_br if cond_v1 ^then_block7v1() else ^merge_block2v7(alloc_v12) !9

          ^then_block7v1() !10:
            v_v8 = builtin.constant <builtin.integer <42: i64>> : builtin.integer i64 !11;
            llvm.br ^merge_block2v7(v_v8) !12

          ^merge_block2v7(alloc_v11: builtin.integer i64) !13:
            next_i_v10 = llvm.add i_v5, one_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !14;
            llvm.br ^header_block3v1(next_i_v10, alloc_v11) !15

          ^exit_block6v3() !16:
            llvm.return zero_v3 !17
        }"#]].assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_load_in_if_region() -> Result<()> {
    // A load in a region of `test.if` must get the value stored before the `test.if`.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.if c {
        ^then():
        a = llvm.load alloc : builtin.integer i64;
        test.yield a
      } else {
        ^else():
        test.yield size
      } : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v5 = test.if c_v0 
            {
              ^then_block2v1() !3:
                test.yield v_v3 !4
            } else 
            {
              ^else_block3v1() !5:
                test.yield size_v1 !6
            } : builtin.integer i64 !7;
            llvm.return r_v5 !8
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_load_in_loop_body() -> Result<()> {
    // A load in the body of `test.loop` must get the value stored before the loop.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.loop (size) {
        ^body(i: builtin.integer i64):
        x = llvm.load alloc : builtin.integer i64;
        s = llvm.add i, x <{nsw=false,nuw=false}> : builtin.integer i64;
        test.loop_condition c, s
      } : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v7 = test.loop (size_v1) 
            {
              ^body_block2v1(i_v4: builtin.integer i64) !3:
                s_v6 = llvm.add i_v4, v_v3 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
                test.loop_condition c_v0, s_v6 !5
            } : builtin.integer i64 !6;
            llvm.return r_v7 !7
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_load_two_regions_deep() -> Result<()> {
    // A load in a `test.if` in a `test.loop` must get the value stored before the loop.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.loop (size) {
        ^body(i: builtin.integer i64):
        y = test.if c {
          ^then():
          x = llvm.load alloc : builtin.integer i64;
          test.yield x
        } else {
          ^else():
          test.yield i
        } : builtin.integer i64;
        test.loop_condition c, y
      } : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v7 = test.loop (size_v1) 
            {
              ^body_block2v1(i_v4: builtin.integer i64) !3:
                y_v6 = test.if c_v0 
                {
                  ^then_block3v1() !4:
                    test.yield v_v3 !5
                } else 
                {
                  ^else_block4v1() !6:
                    test.yield i_v4 !7
                } : builtin.integer i64 !8;
                test.loop_condition c_v0, y_v6 !9
            } : builtin.integer i64 !10;
            llvm.return r_v7 !11
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_load_in_parallel_region() -> Result<()> {
    // A load in the region of `test.parallel` must get the value stored before it.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      other = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      test.parallel {
        ^body():
        x = llvm.load alloc : builtin.integer i64;
        test.non_promotable_use other;
        llvm.store *other <- x;
        test.yield
      };
      llvm.return v
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            other_v2 = llvm.alloca [builtin.integer i64 x size_v0]  : llvm.ptr (0) !2;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !3;
            test.parallel 
            {
              ^body_block2v1() !4:
                test.non_promotable_use other_v2 !5;
                llvm.store *other_v2 <- v_v3  !6;
                test.yield  !7
            } !8;
            llvm.return v_v3 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_stores_in_both_if_regions() -> Result<()> {
    // Stores in both regions of `test.if` must reach a load after it through a new result.
    // The regions must not get new arguments.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      r = test.if c {
        ^then():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        llvm.store *alloc <- a;
        test.yield a
      } else {
        ^else():
        b = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        llvm.store *alloc <- b;
        test.yield b
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            r_v5, alloc_v7 = test.if c_v0 
            {
              ^then_block2v1() !2:
                a_v3 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !3;
                test.yield a_v3, a_v3 !4
            } else 
            {
              ^else_block3v1() !5:
                b_v4 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !6;
                test.yield b_v4, b_v4 !7
            } : builtin.integer i64, builtin.integer i64 !8;
            llvm.return alloc_v7 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_store_in_one_if_region() -> Result<()> {
    // A store in one region of `test.if` must merge, in a new result, with the value
    // stored before the `test.if`, which the other region yields.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.if c {
        ^then():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        llvm.store *alloc <- a;
        test.yield a
      } else {
        ^else():
        test.yield size
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v5, alloc_v7 = test.if c_v0 
            {
              ^then_block2v1() !3:
                a_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !4;
                test.yield a_v4, a_v4 !5
            } else 
            {
              ^else_block3v1() !6:
                test.yield size_v1, v_v3 !7
            } : builtin.integer i64, builtin.integer i64 !8;
            llvm.return alloc_v7 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_store_in_loop_body() -> Result<()> {
    // A store in the body of `test.loop` must reach the next run of the body through a
    // new body argument, and a load after the loop through a new result.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      zero = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.store *alloc <- zero;
      r = test.loop (size) {
        ^body(i: builtin.integer i64):
        x = llvm.load alloc : builtin.integer i64;
        y = llvm.add x, i <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.store *alloc <- y;
        test.loop_condition c, i
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            zero_v3 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64 !2;
            r_v7, alloc_v10 = test.loop (size_v1, zero_v3) 
            {
              ^body_block2v1(i_v4: builtin.integer i64, alloc_v9: builtin.integer i64) !3:
                y_v6 = llvm.add alloc_v9, i_v4 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
                test.loop_condition c_v0, i_v4, y_v6 !5
            } : builtin.integer i64, builtin.integer i64 !6;
            llvm.return alloc_v10 !7
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_store_in_when_region() -> Result<()> {
    // A store in the region of `test.when` must reach a load after it through a new
    // result. The edge that skips the region must carry the value stored before it.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.when (c, size) {
        ^body():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        llvm.store *alloc <- a;
        test.yield a
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v5, alloc_v7 = test.when (c_v0, size_v1, v_v3) 
            {
              ^body_block2v1() !3:
                a_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !4;
                test.yield a_v4, a_v4 !5
            } : builtin.integer i64, builtin.integer i64 !6;
            llvm.return alloc_v7 !7
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_phi_in_if_region() -> Result<()> {
    // Stores in two blocks of a `test.if` region must merge in a new argument of the
    // block where they meet. That argument must reach a load after the `test.if`.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.if c {
        ^then():
        llvm.cond_br if c ^then_a() else ^then_b()
        ^then_a():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        llvm.store *alloc <- a;
        llvm.br ^then_merge()
        ^then_b():
        b = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        llvm.store *alloc <- b;
        llvm.br ^then_merge()
        ^then_merge():
        x = llvm.load alloc : builtin.integer i64;
        test.yield x
      } else {
        ^else():
        test.yield size
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      s = llvm.add r, out <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return s
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v7, alloc_v11 = test.if c_v0 
            {
              ^then_block2v1() !3:
                llvm.cond_br if c_v0 ^then_a_block5v1() else ^then_b_block6v1() !4

              ^then_a_block5v1() !5:
                a_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !6;
                llvm.br ^then_merge_block4v3(a_v4) !7

              ^then_b_block6v1() !8:
                b_v5 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !9;
                llvm.br ^then_merge_block4v3(b_v5) !10

              ^then_merge_block4v3(alloc_v10: builtin.integer i64) !11:
                test.yield alloc_v10, alloc_v10 !12
            } else 
            {
              ^else_block3v5() !13:
                test.yield size_v1, v_v3 !14
            } : builtin.integer i64, builtin.integer i64 !15;
            s_v9 = llvm.add r_v7, alloc_v11 <{nsw=false,nuw=false}>: builtin.integer i64 !16;
            llvm.return s_v9 !17
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_phi_in_if_region_without_load() -> Result<()> {
    // Stores in two blocks of a `test.if` region must merge in a new argument of the
    // block where they meet, also without a load in that block. The `test.yield` there
    // must carry that argument to a load after the `test.if`.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.store *alloc <- v;
      r = test.if c {
        ^then():
        llvm.cond_br if c ^then_a() else ^then_b()
        ^then_a():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        llvm.store *alloc <- a;
        llvm.br ^then_merge()
        ^then_b():
        b = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
        llvm.store *alloc <- b;
        llvm.br ^then_merge()
        ^then_merge():
        test.yield size
      } else {
        ^else():
        test.yield size
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v_v3 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !2;
            r_v6, alloc_v9 = test.if c_v0 
            {
              ^then_block2v1() !3:
                llvm.cond_br if c_v0 ^then_a_block5v1() else ^then_b_block6v1() !4

              ^then_a_block5v1() !5:
                a_v4 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !6;
                llvm.br ^then_merge_block4v3(a_v4) !7

              ^then_b_block6v1() !8:
                b_v5 = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64 !9;
                llvm.br ^then_merge_block4v3(b_v5) !10

              ^then_merge_block4v3(alloc_v8: builtin.integer i64) !11:
                test.yield size_v1, alloc_v8 !12
            } else 
            {
              ^else_block3v5() !13:
                test.yield size_v1, v_v3 !14
            } : builtin.integer i64, builtin.integer i64 !15;
            llvm.return alloc_v9 !16
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_not_promoted_when_store_is_in_parallel_region() -> Result<()> {
    // A store in the region of `test.parallel`, whose stores are not visible in its
    // successors, must block promotion.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      v = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      test.parallel {
        ^body():
        llvm.store *alloc <- v;
        test.yield
      };
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            size_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            alloc_v1 = llvm.alloca [builtin.integer i64 x size_v0]  : llvm.ptr (0) !2;
            v_v2 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !3;
            test.parallel 
            {
              ^body_block2v1() !4:
                llvm.store *alloc_v1 <- v_v2  !5;
                test.yield  !6
            } !7;
            out_v3 = llvm.load alloc_v1  : builtin.integer i64 !8;
            llvm.return out_v3 !9
        }"#]]
    .assert_eq(&after);
    Ok(())
}

#[test]
fn mem2reg_store_in_one_if_region_without_store_before() -> Result<()> {
    // With no store before `test.if`, the region without a store must yield the
    // default value.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      size = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      alloc = llvm.alloca [builtin.integer i64 x size] : llvm.ptr (0);
      r = test.if c {
        ^then():
        a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
        llvm.store *alloc <- a;
        test.yield a
      } else {
        ^else():
        test.yield size
      } : builtin.integer i64;
      out = llvm.load alloc : builtin.integer i64;
      llvm.return out
    }
  "#;

    let (status, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            size_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            v7 = llvm.poison : builtin.integer i64;
            r_v4, alloc_v6 = test.if c_v0 
            {
              ^then_block2v1() !2:
                a_v3 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !3;
                test.yield a_v3, a_v3 !4
            } else 
            {
              ^else_block3v1() !5:
                test.yield size_v1, v7 !6
            } : builtin.integer i64, builtin.integer i64 !7;
            llvm.return alloc_v6 !8
        }"#]]
    .assert_eq(&after);
    Ok(())
}
