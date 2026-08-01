// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! simplify-cfg integration tests using textual LLVM dialect IR parsing.

use expect_test::expect;
use pliron::{
    context::Context,
    init_env_logger_for_tests,
    irbuild::IRStatus,
    irfmt::parsers::spaced,
    operation::{Operation, verify_operation},
    opts::simplify_cfg::simplify_cfg,
    parsable::parse_from_str,
    result::{ExpectOk, Result},
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

fn run_simplify_cfg_on_text(input: &str) -> Result<(IRStatus, String)> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();
    let op = parse_from_str(spaced(Operation::top_level_parser()), ctx, input).expect_ok(ctx);

    verify_operation(op, ctx)?;

    let status = simplify_cfg(op, ctx)?;

    let after = Operation::get_op_dyn(op, ctx).disp(ctx).to_string();
    log::trace!("After simplify-cfg:\n{}", after);
    verify_operation(op, ctx)?;
    Ok((status, after))
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // ^bb1 should be merged into ^entry, so the unconditional branch goes away
    // and only the entry block remains.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !1;
            llvm.return c_v0 !2
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The unreachable ^dead block should be removed.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            a_v0 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !1;
            llvm.return a_v0 !2
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The cond_br folds away, leaving only an unconditional branch to ^taken.
    // The untaken branch becomes unreachable and is culled.
    // The taken branch survives.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            cond_v0 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            a_v1 = builtin.constant <builtin.integer <1: i64>> : builtin.integer i64 !2;
            llvm.return a_v1 !3
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The branch folds and the untaken edge is culled.
    // The taken-edge value `pt` is forwarded into the add instead of `pf`.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            cond_v0 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            pt_v1 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !2;
            pf_v2 = builtin.constant <builtin.integer <9: i64>> : builtin.integer i64 !3;
            sum_v4 = llvm.add pt_v1, pt_v1 <{nsw=false,nuw=false}>: builtin.integer i64 !4;
            llvm.return sum_v4 !5
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The switch folds away, leaving only an unconditional branch to ^bb1.
    // The default and untaken cases become unreachable and are culled.
    // The taken (non-default) case survives but gets merged into ^entry.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            cond_v0 = builtin.constant <builtin.integer <1: i32>> : builtin.integer i32 !1;
            z1_v3 = builtin.constant <builtin.integer <22: i64>> : builtin.integer i64 !2;
            llvm.return z1_v3 !3
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The entry branch is on a non-constant condition, so it cannot fold and
    // both ^a and ^b remain live.
    // Each successor's constant-conditioned branch folds, culling its untaken
    // side along with the constant defined there.
    // The shared join block survives despite losing the ^only_a / ^only_b
    // predecessors, because ^a and ^b still branch to it. With two remaining
    // predecessors it is not merged into either, so the label is preserved.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            llvm.cond_br if c_v0 ^a_block4v1() else ^b_block6v1() !1

          ^a_block4v1() !2:
            ta_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !3;
            llvm.br ^join_block3v5() !4

          ^b_block6v1() !5:
            tb_v2 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !6;
            llvm.br ^join_block3v5() !7

          ^join_block3v5() !8:
            r_v5 = builtin.constant <builtin.integer <33: i64>> : builtin.integer i64 !9;
            llvm.return r_v5 !10
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // ^b is culled once the entry branch folds.
    // The cascade: with ^b gone, ^join has a single predecessor ^a and should
    // merge into it, forwarding va to x so `return x` becomes `return va`.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            cond_v0 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            va_v1 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !2;
            llvm.return va_v1 !3
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The constant condition matches no case, so the switch folds to an
    // unconditional branch to ^default, which is then merged into ^entry.
    // The untaken case blocks become unreachable and are culled.
    // The default case's body survives, merged into the entry block.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            cond_v0 = builtin.constant <builtin.integer <5: i32>> : builtin.integer i32 !1;
            d_v1 = builtin.constant <builtin.integer <100: i64>> : builtin.integer i64 !2;
            llvm.return d_v1 !3
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // ^entry's branch folds to an unconditional jump to ^exit, so the loop is
    // never reached. The cyclic loop subgraph is culled in its entirety.
    // The reachable exit survives (and is merged into the entry block).
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            skip_v1 = builtin.constant <builtin.integer <1: i1>> : builtin.integer i1 !1;
            r_v3 = builtin.constant <builtin.integer <88: i64>> : builtin.integer i64 !2;
            llvm.return r_v3 !3
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    // The non-constant back-edge can't fold and every block is reachable, so the
    // loop's structure is preserved.
    // The self-referential conditional branch is still present.
    assert_eq!(status, IRStatus::Unchanged);
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64(builtin.integer i1) variadic = false>
          [] 
        {
          ^entry_block1v1(c_v0: builtin.integer i1) !0:
            llvm.br ^loop_block3v1() !1

          ^loop_block3v1() !2:
            llvm.cond_br if c_v0 ^loop_block3v1() else ^exit_block4v1() !3

          ^exit_block4v1() !4:
            r_v1 = builtin.constant <builtin.integer <88: i64>> : builtin.integer i64 !5;
            llvm.return r_v1 !6
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The unreachable block inside the nested region is culled.
    // The reachable inner block and the entire outer region survive.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            test.test_region 
            {
              ^region_entry_block2v1() !1:
                inner_v0 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64 !2;
                llvm.return inner_v0 !3
            } !4;
            outer_v2 = builtin.constant <builtin.integer <44: i64>> : builtin.integer i64 !5;
            llvm.return outer_v2 !6
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // The dead block in the func's SSA region is culled, even though the func
    // sits inside the module's graph region.
    // The module, the func, and everything reachable survives.
    expect![[r#"
        builtin.module @m 
        {
          ^module_block_block1v1() !0:
            llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
              [] 
            {
              ^entry_block2v1() !1:
                live_v0 = builtin.constant <builtin.integer <11: i64>> : builtin.integer i64 !2;
                llvm.return live_v0 !3
            } !4
        }"#]]
    .assert_eq(&after);
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

    let (status, after) = run_simplify_cfg_on_text(input)?;
    assert_eq!(status, IRStatus::Changed);
    // All three blocks collapse into the entry block: no branches and no
    // intermediate block labels remain. The value `c` is forwarded through
    // both edges (c -> m -> t), so the final return uses `c`.
    expect![[r#"
        llvm.func @f: llvm.func <builtin.integer i64() variadic = false>
          [] 
        {
          ^entry_block1v1() !0:
            c_v0 = builtin.constant <builtin.integer <7: i64>> : builtin.integer i64 !1;
            llvm.return c_v0 !2
        }"#]]
    .assert_eq(&after);
    Ok(())
}
