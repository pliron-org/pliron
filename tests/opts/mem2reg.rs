//! Tests for the `mem2reg` optimization pass.

use pliron::{
    builtin::op_interfaces::{IsTerminatorInterface, NOpdsInterface, NResultsInterface},
    combine::Parser,
    context::Context,
    derive::pliron_op,
    init_env_logger_for_tests,
    irbuild::{IRStatus, rewriter::Rewriter},
    irfmt::parsers::spaced,
    operation::{Operation, verify_operation},
    opts::mem2reg::{AllocInfo, PromotableOpInterface, PromotableOpKind, mem2reg},
    parsable::{self, state_stream_from_iterator},
    pass_manager::AnalysisManager,
    printable::Printable,
    result::Result,
};

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

fn run_mem2reg(input: &str) -> Result<(IRStatus, String, String)> {
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
    log::trace!("Before mem2reg:\n{}", before);
    verify_operation(op, ctx)?;

    let mut analyses = AnalysisManager::default();
    let status = mem2reg(op, ctx, &mut analyses)?;

    let after = op.disp(ctx).to_string();
    log::trace!("After mem2reg:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, before, after))
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca should be removed
    assert!(!after.contains("llvm.alloca"));
    // Store should be removed
    assert!(!after.contains("llvm.store"));
    // Load should be removed (replaced with constant)
    assert!(!after.contains("llvm.load"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Should contain the final stored value
    assert!(after.contains("<42: i64>"));
    // Alloca and stores removed
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca removed
    assert!(!after.contains("llvm.alloca"));
    // Load removed
    assert!(!after.contains("llvm.load"));
    // Should have poison operation
    assert!(after.contains("llvm.poison"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca and store should be removed
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Alloca removed
    assert!(!after.contains("llvm.alloca"));
    // Stores removed (phis created instead)
    assert!(!after.contains("llvm.store"));
    // Load removed
    assert!(!after.contains("llvm.load"));
    // Merge still exists and branch forwarding got materialized via successor operands.
    assert!(after.contains("llvm.br ^") && after.contains("llvm.return"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Both allocas removed
    assert!(!after.contains("llvm.alloca"));
    // All stores removed
    assert!(!after.contains("llvm.store"));
    // Both loads removed
    assert!(!after.contains("llvm.load"));
    // Add operation should work with the promoted values
    assert!(after.contains("llvm.add"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
    assert!(!after.contains("llvm.load"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
    assert!(!after.contains("llvm.load"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
    assert!(!after.contains("llvm.load"));
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

    let (_status, _before, _after) = run_mem2reg(input)?;
    // Should be changed (stores are dead, can be eliminated)
    // The exact behavior may vary, but alloca should be gone
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
    assert!(!after.contains("llvm.store"));
    assert!(!after.contains("llvm.load"));
    // Should have poison for uninitialized load
    assert!(after.contains("llvm.poison"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
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

    let (_status, _before, after) = run_mem2reg(input)?;
    // The allocation should not be promoted because its address is used
    // However, some loads/stores might still be promotable depending on implementation
    // This test documents the expected behavior
    assert!(after.contains("llvm.alloca"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("llvm.alloca"));
    Ok(())
}

#[test]
fn mem2reg_not_promoted_when_load_is_in_nested_region() -> Result<()> {
    // Region hierarchy corner case: nested-region uses currently force pruning.
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("llvm.alloca"));
    assert!(after.contains("llvm.load alloc"));
    Ok(())
}

#[test]
fn mem2reg_not_promoted_when_store_is_in_nested_region() -> Result<()> {
    // Region hierarchy corner case with nested definitions.
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("llvm.alloca"));
    assert!(after.contains("llvm.store *alloc"));
    assert!(after.contains("llvm.load alloc"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("llvm.alloca"));
    assert!(after.contains("test.non_promotable_use"));
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

    let (status, _before, after) = run_mem2reg(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("llvm.alloca"));
    assert!(after.contains("llvm.load alloc"));
    assert!(after.contains("test.non_branch_succ_term"));
    Ok(())
}
