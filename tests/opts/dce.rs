//! DCE integration tests using textual LLVM dialect IR parsing.

use pliron::{
    basic_block::BasicBlock,
    combine::Parser,
    context::{Context, Ptr},
    init_env_logger_for_tests,
    irbuild::IRStatus,
    irfmt::parsers::spaced,
    op::Op,
    operation::{Operation, verify_operation},
    opts::dce::{BlockArgRemoval, dce},
    parsable::{self, state_stream_from_iterator},
    printable::Printable,
    result::Result,
};

use pliron_llvm as _;

// Define a custom test operation with a single-block region and no side effects
// This is used to test DCE behavior when eliminating region-containing ops
use pliron::{
    builtin::op_interfaces::{NOpdsInterface, NResultsInterface},
    derive::pliron_op,
    opts::dce::SideEffects,
};

#[pliron_op(
    name = "test.pure_region",
    format = "region($0) `:` type($0)",
    verifier = "succ"
)]
pub struct PureRegionOp;

#[pliron::derive::op_interface_impl]
impl SideEffects for PureRegionOp {
    fn has_side_effects(&self, _ctx: &Context) -> bool {
        false // This op has no side effects, so it can be eliminated if unused
    }
}

#[pliron::derive::op_interface_impl]
impl BlockArgRemoval for PureRegionOp {
    fn can_remove_block_args(&self, ctx: &Context, block: Ptr<BasicBlock>) -> bool {
        use pliron::linked_list::ContainsLinkedList;
        // Only allow block argument removal for non-entry blocks
        self.get_operation()
            .deref(ctx)
            .get_region(0)
            .deref(ctx)
            .get_head()
            != Some(block)
    }
}

#[pliron_op(
  name = "test.multi_result_def",
  format = "`: ` types(CharSpace(`,`))",
  interfaces = [NOpdsInterface<0>, NResultsInterface<2>],
  verifier = "succ"
)]
pub struct MultiResultDefOp;

#[pliron::derive::op_interface_impl]
impl SideEffects for MultiResultDefOp {
    fn has_side_effects(&self, _ctx: &Context) -> bool {
        false
    }
}

#[pliron_op(
  name = "test.multi_use_sink",
  format = "$0 `, ` $1 `, ` $2 `, ` $3",
  interfaces = [NOpdsInterface<4>, NResultsInterface<0>],
  verifier = "succ"
)]
pub struct MultiUseSinkOp;

#[pliron::derive::op_interface_impl]
impl SideEffects for MultiUseSinkOp {
    fn has_side_effects(&self, _ctx: &Context) -> bool {
        false
    }
}

fn run_dce_on_text(input: &str) -> Result<(IRStatus, String, String)> {
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
    log::trace!("Before DCE:\n{}", before);
    verify_operation(op, ctx)?;

    let status = dce(op, ctx)?;

    let after = op.disp(ctx).to_string();
    log::trace!("After DCE:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, before, after))
}

#[test]
fn dce_removes_dead_llvm_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      live = builtin.constant <builtin.integer <7: si64>> : builtin.integer si64;
      dead = builtin.constant <builtin.integer <0: si64>> : builtin.integer si64;
      llvm.return live
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("<0: si64>"));
    assert!(after.contains("<7: si64>"));
    Ok(())
}

#[test]
fn dce_keeps_live_llvm_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      live = builtin.constant <builtin.integer <9: si64>> : builtin.integer si64;
      llvm.return live
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("<9: si64>"));
    Ok(())
}

#[test]
fn dce_does_not_remove_unused_entry_block_arg_in_llvm_func() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 (builtin.integer si64) variadic = false> [] {
      ^entry(arg0: builtin.integer si64):
      c = builtin.constant <builtin.integer <5: si64>> : builtin.integer si64;
      llvm.return c
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("(arg0"));
    Ok(())
}

#[test]
fn dce_removes_dead_non_entry_block_arg_and_br_operand() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      x = builtin.constant <builtin.integer <1: si64>> : builtin.integer si64;
      llvm.br ^bb1(x)

      ^bb1(arg0: builtin.integer si64):
      c = builtin.constant <builtin.integer <7: si64>> : builtin.integer si64;
      llvm.return c
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(!after.contains("(arg0"));
    assert!(!after.contains("<1: si64>"));
    Ok(())
}

#[test]
fn dce_keeps_used_non_entry_block_arg() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      x = builtin.constant <builtin.integer <1: si64>> : builtin.integer si64;
      llvm.br ^bb1(x)

      ^bb1(arg0: builtin.integer si64):
      llvm.return arg0
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("(arg0"));
    Ok(())
}

#[test]
fn dce_dead_arg_cascades_to_successor_operands() -> Result<()> {
    // Test that when a block argument is unused, the corresponding forwarded operand
    // from the predecessor's branch is also marked dead and eliminated.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      dead_val = builtin.constant <builtin.integer <1: si64>> : builtin.integer si64;
      live_val = builtin.constant <builtin.integer <42: si64>> : builtin.integer si64;
      llvm.br ^merge(dead_val, live_val)

      ^merge(dead_arg: builtin.integer si64, live_arg: builtin.integer si64):
      llvm.return live_arg
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Live constant should remain
    assert!(after.contains("<42: si64>"));
    // Dead constant should be eliminated
    assert!(!after.contains("<1: si64>"));
    // Dead block argument should be removed
    assert!(!after.contains("dead_arg"));
    Ok(())
}

#[test]
fn dce_multiple_preds_mixed_dead_live_operands() -> Result<()> {
    // Multiple paths to a block with mixed dead/live operands.
    // Verify DCE removes only the dead forwarded operands per predecessor.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      cond = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      dead_left = builtin.constant <builtin.integer <1: si64>> : builtin.integer si64;
      live_left = builtin.constant <builtin.integer <10: si64>> : builtin.integer si64;
      dead_right = builtin.constant <builtin.integer <2: si64>> : builtin.integer si64;
      live_right = builtin.constant <builtin.integer <20: si64>> : builtin.integer si64;
      llvm.cond_br if cond ^left(dead_left, live_left) else ^right(dead_right, live_right)

      ^left(left_dead: builtin.integer si64, left_live: builtin.integer si64):
      llvm.br ^merge(left_dead, left_live)

      ^right(right_dead: builtin.integer si64, right_live: builtin.integer si64):
      llvm.br ^merge(right_dead, right_live)

      ^merge(in_dead: builtin.integer si64, in_live: builtin.integer si64):
      llvm.return in_live
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Live constant should remain
    assert!(after.contains("<10: si64>"));
    assert!(after.contains("<20: si64>"));
    // Dead constants should be eliminated
    assert!(!after.contains("<1: si64>"));
    assert!(!after.contains("<2: si64>"));
    // Dead block argument should be removed
    assert!(!after.contains("in_dead"));
    assert!(!after.contains("left_dead"));
    assert!(!after.contains("right_dead"));
    Ok(())
}

#[test]
fn dce_all_successor_operands_dead() -> Result<()> {
    // All forwarded operands to a successor are dead, but successor block still exists.
    // Verify branch operand list becomes empty and block args are removed.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      dead1 = builtin.constant <builtin.integer <7: si64>> : builtin.integer si64;
      dead2 = builtin.constant <builtin.integer <8: si64>> : builtin.integer si64;
      live = builtin.constant <builtin.integer <99: si64>> : builtin.integer si64;
      llvm.br ^exit(dead1, dead2)

      ^exit(unused1: builtin.integer si64, unused2: builtin.integer si64):
      llvm.return live
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Both dead constants should be eliminated
    assert!(!after.contains("<7: si64>"));
    assert!(!after.contains("<8: si64>"));
    // Live constant should remain
    assert!(after.contains("<99: si64>"));
    // Exit block should have no arguments
    assert!(after.contains("^exit():") || !after.contains("unused1"));
    Ok(())
}

#[test]
fn dce_chain_of_dead_computations() -> Result<()> {
    // A chain of dead constants/branches where each dead result feeds into another dead context.
    // Verify entire chain is eliminated.
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      dead1 = builtin.constant <builtin.integer <1: si64>> : builtin.integer si64;
      dead2 = builtin.constant <builtin.integer <2: si64>> : builtin.integer si64;
      dead3 = builtin.constant <builtin.integer <3: si64>> : builtin.integer si64;
      live = builtin.constant <builtin.integer <99: si64>> : builtin.integer si64;
      llvm.br ^exit(dead1, dead2, dead3)

      ^exit(unused1: builtin.integer si64, unused2: builtin.integer si64, unused3: builtin.integer si64):
      llvm.return live
    }
  "#;

    let (status, _before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Live constant should remain
    assert!(after.contains("<99: si64>"));
    // All dead constants should be eliminated
    assert!(!after.contains("<1: si64>"));
    assert!(!after.contains("<2: si64>"));
    assert!(!after.contains("<3: si64>"));
    // Exit block should have no arguments (check that unused args were removed)
    assert!(!after.contains("unused1"));
    assert!(!after.contains("unused2"));
    assert!(!after.contains("unused3"));
    Ok(())
}

#[test]
fn dce_region_containing_dead_op_safely_ignores_inner_dead_code() -> Result<()> {
    init_env_logger_for_tests!();
    // This test verifies that when a region-containing op (with no side effects) is
    // eliminated by DCE, the pass safely handles inner dead instructions without
    // trying to dereference them after parent erasure.
    //
    // The scenario:
    // 1. Create a test.pure_region op (no side effects) containing dead constants
    // 2. Put this inside an llvm.func (so it's unused in the outer context)
    // 3. DCE collects both the inner dead constants and the parent pure_region op
    // 4. When parent is erased, DCE must not deref the inner dead ops later

    let input = r#"
    llvm.func @test: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      inner_dead1 = builtin.constant <builtin.integer <10: i64>> : builtin.integer i64;
      inner_dead2 = builtin.constant <builtin.integer <20: i64>> : builtin.integer i64;
      not_initially_dead = test.pure_region {
        ^region_entry():
          region_dead1 = builtin.constant <builtin.integer <100: i64>> : builtin.integer i64;
          region_dead2 = builtin.constant <builtin.integer <200: i64>> : builtin.integer i64;
          llvm.br ^region_dead(region_dead1)

        ^region_dead(arg0: builtin.integer i64):
          llvm.return
      } : builtin.integer i64;
      dead = llvm.add not_initially_dead, not_initially_dead <{nsw=false,nuw=false}> : builtin.integer i64;
      live = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return live
    }
  "#;

    let ctx = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        input.chars(),
        parsable::State::new(ctx, pliron::location::Source::InMemory),
    );
    let func_op = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .expect("textual LLVM IR should parse")
        .0;

    verify_operation(func_op, ctx)?;
    let _before = func_op.disp(ctx).to_string();

    // Run DCE - should:
    // 1. Mark inner_dead1, inner_dead2 as dead
    // 2. Mark region_dead1, region_dead2 as dead
    // 3. Mark test.pure_region as dead (no uses, no side effects)
    // 4. Eliminate all safely without panicking on inner dead ops
    let status = dce(func_op, ctx)?;
    assert!(status == IRStatus::Changed);

    verify_operation(func_op, ctx)?;
    let after = func_op.disp(ctx).to_string();

    // The pure_region op should be eliminated
    assert!(!after.contains("test.pure_region"));

    // All dead constants should be eliminated (both outer and inner)
    assert!(!after.contains("<10: i64>"));
    assert!(!after.contains("<20: i64>"));
    assert!(!after.contains("<100: i64>"));
    assert!(!after.contains("<200: i64>"));
    assert!(!after.contains("arg0"));

    // Live constant should remain
    assert!(after.contains("<99: i64>"));

    // Function should still be valid
    assert!(after.contains("@test"));

    // The main point: we got here without panicking, which means
    // DCE safely handled the pure_region op elimination and its inner dead code

    Ok(())
}

#[test]
fn dce_eliminates_multi_result_op_after_same_op_and_successor_uses_die() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer si64 () variadic = false> [] {
      ^entry():
      left, right = test.multi_result_def : builtin.integer si64, builtin.integer si64;
      test.multi_use_sink left, left, right, right;
      llvm.br ^exit(left, right)

      ^exit(arg0: builtin.integer si64, arg1: builtin.integer si64):
      live = builtin.constant <builtin.integer <99: si64>> : builtin.integer si64;
      llvm.return live
    }
  "#;

    let (status, before, after) = run_dce_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);

    assert!(before.contains("test.multi_result_def"));
    assert!(before.contains("test.multi_use_sink"));

    assert!(!after.contains("test.multi_result_def"));
    assert!(!after.contains("test.multi_use_sink"));
    assert!(!after.contains("(arg0"));
    assert!(!after.contains("arg1"));
    assert!(after.contains("<99: si64>"));

    Ok(())
}
