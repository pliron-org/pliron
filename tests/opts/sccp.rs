//! SCCP integration tests using textual LLVM dialect IR parsing.

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

use pliron_llvm as _;

use pliron::{
    builtin::op_interfaces::{NOpdsInterface, NResultsInterface},
    derive::pliron_op,
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<7: i64>"));
    assert!(!after.contains("llvm.add"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert!(after.contains("<2: i64>"));
    assert_eq!(status, IRStatus::Changed);
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<2: i64>"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<10: i64>"));
    assert!(after.contains("llvm.add"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert!(after.contains("llvm.add"));
    // Materialized constants inserted into ^bb0, ^bb1, and ^bb2
    assert_eq!(status, IRStatus::Changed);
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("llvm.add"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert!(after.contains("<7: i64>"));
    assert!(!after.contains("llvm.add"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // Both inner adds should fold.
    assert!(after.contains("<7: i64>"));
    assert!(after.contains("<30: i64>"));
    assert!(!after.contains("llvm.add"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The inner add should fold to 7.
    assert!(after.contains("<7: i64>"));
    assert!(!after.contains("llvm.add"));
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert_eq!(after.matches("constant").count(), 2);
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert_eq!(after.matches("constant").count(), 6);
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    assert_eq!(after.matches("constant").count(), 3);
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert_eq!(after.matches("constant").count(), 2);
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

    // Source: the arg name appears at its declaration site and twice as an
    // operand of `llvm.add` -> 3 occurrences.
    assert_eq!(input.matches("califragilistic").count(), 3);

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // After materialization, all uses of the block arg should have been
    // replaced with the materialized constant's result. The arg name should
    // remain only at its declaration site in the block header and in the
    // preserved debug-info metadata for that declaration.
    assert_eq!(after.matches("califragilistic").count(), 2);
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

    let (status, _before, after) = run_sccp_on_text(input)?;
    assert_eq!(status, IRStatus::Unchanged);
    assert_eq!(after.matches("constant").count(), 2);
    Ok(())
}

#[test]
fn sccp_treats_free_variables_as_non_constant() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      outer_three = builtin.constant <builtin.integer <3: i64>> : builtin.integer i64;
      outer_four = builtin.constant <builtin.integer <4: i64>> : builtin.integer i64;
      test.test_region {
        ^region_entry():
        inner_sum = llvm.add outer_three, outer_four <{nsw=false,nuw=false}> : builtin.integer i64;
        llvm.return inner_sum
      };
      done = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
      llvm.return done
    }
  "#;

    init_env_logger_for_tests!();
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
    let after = func_op.disp(ctx).to_string();

    // Even though `outer_three` and `outer_four` are syntactically
    // `builtin.constant` ops, the analysis must treat them as NotAConstant when
    // they appear free inside the analysis root, so the inner `llvm.add` must
    // *not* fold.
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("llvm.add"));
    Ok(())
}
