//! simplify-cfg integration tests using textual LLVM dialect IR parsing.

use combine::Parser;
use pliron::{
    context::Context,
    init_env_logger_for_tests,
    irbuild::IRStatus,
    irfmt::parsers::spaced,
    operation::{Operation, verify_operation},
    opts::simplify_cfg::simplify_cfg,
    parsable::{self, state_stream_from_iterator},
    printable::Printable,
    result::Result,
};

use pliron_llvm as _;

fn run_simplify_cfg_on_text(input: &str) -> Result<(IRStatus, String, String)> {
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
    log::trace!("Before simplify-cfg:\n{}", before);
    verify_operation(op, ctx)?;

    let status = simplify_cfg(op, ctx)?;

    let after = op.disp(ctx).to_string();
    log::trace!("After simplify-cfg:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, before, after))
}

/// A block whose only successor has it as its only predecessor should be merged
/// into its predecessor, eliminating the intervening unconditional branch.
#[test]
fn simplify_cfg_merges_single_succ_single_pred_blocks() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      llvm.br ^bb1(c)

      ^bb1(x: builtin.integer i64):
      llvm.return x
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // ^bb1 should be merged into ^entry, so the unconditional branch goes away
    // and only the entry block remains.
    assert!(!after.contains("llvm.br"));
    assert!(!after.contains("^bb1"));
    assert!(after.contains("llvm.return"));
    Ok(())
}

/// A block that is unreachable from the region entry should be culled.
#[test]
fn simplify_cfg_culls_unreachable_block() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.return a

      ^dead():
      b = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.return b
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The unreachable ^dead block should be removed.
    assert!(!after.contains("^dead"));
    assert!(!after.contains("<2: i64>"));
    assert!(after.contains("<1: i64>"));
    Ok(())
}
