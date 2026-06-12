//! simplify-cfg integration tests using textual LLVM dialect IR parsing.

use pliron::{
    combine::Parser,
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
    assert!(after.contains("llvm.return c"));
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

/// A conditional branch on a constant `i1` should fold to an unconditional
/// branch to the taken target, after which the untaken block becomes
/// unreachable and is culled.
#[test]
fn simplify_cfg_culls_untaken_branch_of_constant_cond_br() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      cond = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if cond ^taken() else ^untaken()

      ^taken():
      a = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64;
      llvm.return a

      ^untaken():
      b = builtin.constant <builtin.integer <2: i64>> : builtin.integer i64;
      llvm.return b
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The cond_br folds away, leaving only an unconditional branch to ^taken.
    assert!(!after.contains("llvm.cond_br"));
    // The untaken branch becomes unreachable and is culled.
    assert!(!after.contains("^untaken"));
    assert!(!after.contains("<2: i64>"));
    // The taken branch survives.
    assert!(after.contains("<1: i64>"));
    Ok(())
}

/// When a constant-conditioned `cond_br` folds, it must rewrite to an
/// unconditional branch carrying the *taken* edge's successor operands, not the
/// untaken edge's.
#[test]
fn simplify_cfg_fold_preserves_taken_edge_args() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      cond = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      pt = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      pf = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.cond_br if cond ^taken(pt) else ^untaken(pf)

      ^taken(t: builtin.integer i64):
      sum = llvm.add t, t <{nsw=false,nuw=false}> : builtin.integer i64;
      llvm.return sum

      ^untaken(u: builtin.integer i64):
      llvm.return u
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The branch folds and the untaken edge is culled.
    assert!(!after.contains("llvm.cond_br"));
    assert!(!after.contains("^untaken"));
    // The taken-edge value `pt` is forwarded into the add instead of `pf`.
    assert!(after.contains("llvm.add pt"));
    assert!(!after.contains("llvm.add pf"));
    Ok(())
}

/// A `llvm.switch` on a constant condition matching a non-default case should
/// fold to an unconditional branch to that case, after which the default case
/// and the other (untaken) cases become unreachable and are culled.
#[test]
fn simplify_cfg_culls_untaken_cases_of_constant_switch() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      cond = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32;
      llvm.switch cond, ^default()
      [
        { <0: i32> : ^bb0() },
        { <1: i32> : ^bb1() }
      ]

      ^default():
      d = builtin.constant <builtin.integer <100: i64>> : builtin.integer i64;
      llvm.return d

      ^bb0():
      z0 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.return z0

      ^bb1():
      z1 = builtin.constant <builtin.integer <22: i64>> : builtin.integer i64;
      llvm.return z1
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The switch folds away, leaving only an unconditional branch to ^bb1.
    assert!(!after.contains("llvm.switch"));
    // The default and untaken cases become unreachable and are culled.
    assert!(!after.contains("^default"));
    assert!(!after.contains("^bb0"));
    assert!(!after.contains("<100: i64>"));
    assert!(!after.contains("<0: i64>"));
    // The taken (non-default) case survives but gets merged into ^entry
    assert!(after.contains("<22: i64>"));
    assert!(!after.contains("^bb1"));
    Ok(())
}

/// A `cond_br` on a non-constant condition cannot be folded, so both of its
/// successors stay live. Each successor has a `cond_br` on a *constant*
/// condition that folds to an unconditional branch into a shared join block.
///
/// This exercises two things at once:
///   - The untaken side of each folded branch (`^only_a`, `^only_b`) becomes
///     unreachable and is culled.
///   - The shared join block `^join` survives even though it loses two of its
///     predecessors (`^only_a` and `^only_b`), because `^a` and `^b` still
///     reach it. Having more than one remaining predecessor also prevents it
///     from being merged into either of them.
#[test]
fn simplify_cfg_keeps_join_block_with_surviving_predecessor() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      llvm.cond_br if c ^a() else ^b()

      ^a():
      ta = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if ta ^join() else ^only_a()

      ^b():
      tb = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if tb ^join() else ^only_b()

      ^only_a():
      za = builtin.constant <builtin.integer <55: i64>> : builtin.integer i64;
      llvm.br ^join()

      ^only_b():
      zb = builtin.constant <builtin.integer <66: i64>> : builtin.integer i64;
      llvm.br ^join()

      ^join():
      r = builtin.constant <builtin.integer <33: i64>> : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The entry branch is on a non-constant condition, so it cannot fold and
    // both ^a and ^b remain live.
    assert!(after.contains("llvm.cond_br"));
    assert!(after.contains("^a"));
    assert!(after.contains("^b"));
    // Each successor's constant-conditioned branch folds, culling its untaken
    // side along with the constant defined there.
    assert!(!after.contains("^only_a"));
    assert!(!after.contains("^only_b"));
    assert!(!after.contains("<55: i64>"));
    assert!(!after.contains("<66: i64>"));
    // The shared join block survives despite losing the ^only_a / ^only_b
    // predecessors, because ^a and ^b still branch to it. With two remaining
    // predecessors it is not merged into either, so the label is preserved.
    assert!(after.contains("^join"));
    assert!(after.contains("<33: i64>"));
    Ok(())
}

/// Cull-then-merge cascade: folding `^entry`'s branch makes `^b` unreachable, so
/// `^join` (originally reached from both `^a` and `^b`) drops to a single
/// predecessor `^a`. With one predecessor and `^a` having `^join` as its sole
/// successor, `^join` should subsequently merge into `^a`, forwarding `^a`'s
/// branch operand `va` to `^join`'s argument.
#[test]
fn simplify_cfg_cull_enables_subsequent_merge() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      cond = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if cond ^a() else ^b()

      ^a():
      va = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      llvm.br ^join(va)

      ^b():
      vb = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64;
      llvm.br ^join(vb)

      ^join(x: builtin.integer i64):
      llvm.return x
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // ^b is culled once the entry branch folds.
    assert!(!after.contains("^b"));
    assert!(!after.contains("<9: i64>"));
    // The cascade: with ^b gone, ^join has a single predecessor ^a and should
    // merge into it, forwarding va to x so `return x` becomes `return va`.
    assert!(!after.contains("^join"));
    assert!(after.contains("llvm.return va"));
    Ok(())
}

/// A `llvm.switch` on a constant condition that matches none of the case values
/// should fold to an unconditional branch to the default destination, after
/// which the (untaken) case blocks become unreachable and are culled.
#[test]
fn simplify_cfg_culls_cases_of_constant_switch_to_default() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      cond = builtin.constant <builtin.integer <5: i32>> : builtin.integer i32;
      llvm.switch cond, ^default()
      [
        { <0: i32> : ^bb0() },
        { <1: i32> : ^bb1() }
      ]

      ^default():
      d = builtin.constant <builtin.integer <100: i64>> : builtin.integer i64;
      llvm.return d

      ^bb0():
      z0 = builtin.constant <builtin.integer <0: i64>> : builtin.integer i64;
      llvm.return z0

      ^bb1():
      z1 = builtin.constant <builtin.integer <22: i64>> : builtin.integer i64;
      llvm.return z1
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The constant condition matches no case, so the switch folds to an
    // unconditional branch to ^default, which is then merged into ^entry.
    assert!(!after.contains("llvm.switch"));
    // The untaken case blocks become unreachable and are culled.
    assert!(!after.contains("^bb0"));
    assert!(!after.contains("^bb1"));
    assert!(!after.contains("<0: i64>"));
    assert!(!after.contains("<22: i64>"));
    // The default case's body survives, merged into the entry block.
    assert!(after.contains("<100: i64>"));
    Ok(())
}

/// A constant-conditioned branch in `^entry` skips an entire loop, making the
/// loop's blocks unreachable. The loop is a cycle (`^loop_header` <->
/// `^loop_body` via a back-edge), so culling it requires erasing a subgraph
/// of blocks that hold cyclic references to one another.
#[test]
fn simplify_cfg_culls_unreachable_loop() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      skip = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1;
      llvm.cond_br if skip ^exit() else ^loop_header()

      ^loop_header():
      llvm.cond_br if c ^loop_body() else ^exit()

      ^loop_body():
      dead = builtin.constant <builtin.integer <77: i64>> : builtin.integer i64;
      llvm.br ^loop_header()

      ^exit():
      r = builtin.constant <builtin.integer <88: i64>> : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // ^entry's branch folds to an unconditional jump to ^exit, so the loop is
    // never reached.
    assert!(!after.contains("llvm.cond_br"));
    // The cyclic loop subgraph is culled in its entirety.
    assert!(!after.contains("^loop_header"));
    assert!(!after.contains("^loop_body"));
    assert!(!after.contains("<77: i64>"));
    // The reachable exit survives (and is merged into the entry block).
    assert!(after.contains("<88: i64>"));
    Ok(())
}

/// A reachable trivial loop: `^loop` branches back to itself on a non-constant
/// condition. Nothing here is dead, so the whole loop must be preserved.
#[test]
fn simplify_cfg_preserves_reachable_trivial_loop() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 (builtin.integer i1) variadic = false> [] {
      ^entry(c: builtin.integer i1):
      llvm.br ^loop()

      ^loop():
      llvm.cond_br if c ^loop() else ^exit()

      ^exit():
      r = builtin.constant <builtin.integer <88: i64>> : builtin.integer i64;
      llvm.return r
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    // The non-constant back-edge can't fold and every block is reachable, so the
    // loop's structure is preserved.
    assert_eq!(status, IRStatus::Unchanged);
    assert!(after.contains("^loop"));
    assert!(after.contains("^exit"));
    // The self-referential conditional branch is still present.
    assert!(after.contains("llvm.cond_br"));
    assert!(after.contains("<88: i64>"));
    Ok(())
}

/// Simplification must descend into nested regions. The outer function region
/// has no dead blocks, but a nested `test.test_region` (an SSA region) contains
/// an unreachable block. That inner block should be culled while everything in
/// the outer region is left untouched.
#[test]
fn simplify_cfg_culls_inside_nested_region() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      test.test_region {
        ^region_entry():
        inner = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64;
        llvm.return inner

        ^inner_dead():
        gone = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
        llvm.return gone
      };
      outer = builtin.constant <builtin.integer <44: i64>> : builtin.integer i64;
      llvm.return outer
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The unreachable block inside the nested region is culled.
    assert!(!after.contains("^inner_dead"));
    assert!(!after.contains("<99: i64>"));
    // The reachable inner block and the entire outer region survive.
    assert!(after.contains("test.test_region"));
    assert!(after.contains("<11: i64>"));
    assert!(after.contains("<44: i64>"));
    Ok(())
}

/// Simplification must descend through graph regions to reach the SSA regions
/// nested inside them. A `builtin.module` holds a graph region (no reachability
/// semantics), so its own block is never culled; but the `llvm.func` it contains
/// has an SSA region with an unreachable block, which must still be culled even
/// though the enclosing module region is a graph region.
#[test]
fn simplify_cfg_descends_into_func_nested_in_module() -> Result<()> {
    let input = r#"
    builtin.module @m {
      ^module_block():
      llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
        ^entry():
        live = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64;
        llvm.return live

        ^dead():
        gone = builtin.constant <builtin.integer <99: i64>> : builtin.integer i64;
        llvm.return gone
      }
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The dead block in the func's SSA region is culled, even though the func
    // sits inside the module's graph region.
    assert!(!after.contains("^dead"));
    assert!(!after.contains("<99: i64>"));
    // The module, the func, and everything reachable survives.
    assert!(after.contains("builtin.module"));
    assert!(after.contains("llvm.func"));
    assert!(after.contains("<11: i64>"));
    Ok(())
}

/// A straight-line chain of blocks `^entry -> ^mid -> ^tail`, each with a single
/// successor and single predecessor, should collapse into a single block.
/// The block argument forwarded along each edge must be threaded through correctly.
#[test]
fn simplify_cfg_collapses_straight_line_chain() -> Result<()> {
    let input = r#"
    llvm.func @f: llvm.func <builtin.integer i64 () variadic = false> [] {
      ^entry():
      c = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64;
      llvm.br ^mid(c)

      ^mid(m: builtin.integer i64):
      llvm.br ^tail(m)

      ^tail(t: builtin.integer i64):
      llvm.return t
    }
  "#;

    let (status, _before, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // All three blocks collapse into the entry block: no branches and no
    // intermediate block labels remain.
    assert!(!after.contains("llvm.br"));
    assert!(!after.contains("^mid"));
    assert!(!after.contains("^tail"));
    // The value `c` is forwarded through both edges (c -> m -> t), so the final
    // return uses `c`.
    assert!(after.contains("llvm.return c"));
    Ok(())
}
