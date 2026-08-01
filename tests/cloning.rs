// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Tests for IR cloning ([pliron::irbuild::cloning]) and equivalence
//! ([pliron::irbuild::equivalence]). The two are exercised together where it
//! makes sense: a clone must always be equivalent to its original.

use common::{ConstantOp, ReturnOp, const_ret_in_mod};
use pliron::{
    attribute::Attribute,
    basic_block::BasicBlock,
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::{
            IsTerminatorInterface, OneRegionInterface, OneResultInterface,
            SingleBlockRegionInterface,
        },
        ops::{FuncOp, ModuleOp},
        types::{FunctionType, IntegerType, Signedness},
    },
    common_traits::Named,
    context::{Context, Ptr},
    derive::pliron_op,
    identifier::Identifier,
    irbuild::{
        cloning::{IrMapping, clone_blocks_into, clone_operation},
        equivalence::{EqResult, IgnoreConfig, basic_block_eq, operation_eq, region_eq},
        listener::{DummyListener, Recorder, RecorderEvent},
        rewriter::IRRewriter,
    },
    location::{Located, Location},
    op::Op,
    operation::{Operation, verify_operation},
    printable::Printable,
    result::Result,
    utils::apint::{APInt, bw},
};

mod common;

/// A minimal terminator that carries successors, so a test can build a small CFG
/// to clone. (The test dialect's other ops are not branch-like.)
#[pliron_op(name = "test.br", format, interfaces = [IsTerminatorInterface], verifier = "succ")]
struct BranchOp {}

/// The single successor of a terminator (asserting there is exactly one).
fn sole_successor(ctx: &Context, term: Ptr<Operation>) -> Ptr<BasicBlock> {
    let term_ref = term.deref(ctx);
    let mut succs = term_ref.successors();
    let first = succs.next().expect("terminator should have a successor");
    assert!(succs.next().is_none(), "expected exactly one successor");
    first
}

/// Build, in a fresh module named `mod_name`,
/// `fn foo() { A: c = const val; br -> B;  B: return c }`
fn branch_and_return_fn(
    ctx: &mut Context,
    mod_name: &str,
    val: u64,
) -> Result<(FuncOp, Ptr<Operation>)> {
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let module = ModuleOp::new(ctx, mod_name.try_into().unwrap());
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let region = func.get_region(ctx);

    let block_a = func.get_entry_block(ctx);
    let c = ConstantOp::new(ctx, val);
    c.get_operation().insert_at_back(block_a, ctx);
    let block_b = BasicBlock::new(ctx, None, vec![]);
    block_b.insert_at_back(region, ctx);
    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![block_b],
        0,
    )
    .insert_at_back(block_a, ctx);
    ReturnOp::new(ctx, c.get_result(ctx))
        .get_operation()
        .insert_at_back(block_b, ctx);

    Ok((func, c.get_operation()))
}

/// Never ignore any attribute; used as the default [IgnoreConfig::ignore_attr].
fn never_ignore(_ctx: &Context, _attr: &dyn Attribute) -> bool {
    false
}

/// Ignore any [IntegerAttr], regardless of its value.
fn ignore_integer_attrs(_ctx: &Context, attr: &dyn Attribute) -> bool {
    attr.downcast_ref::<IntegerAttr>().is_some()
}

/// Panic with a useful message unless `result` is [EqResult::Eq].
fn assert_equivalent(ctx: &Context, result: EqResult) {
    match result {
        EqResult::Eq => {}
        _ => panic!("{}", result.disp(ctx)),
    }
}

/// Panic unless `result` reports a mismatch.
fn assert_not_equivalent(result: EqResult) {
    assert!(
        !matches!(result, EqResult::Eq),
        "expected a mismatch, but the two were reported equivalent"
    );
}

/// Cloning a function deep-copies its body and remaps intra-region operands:
/// the cloned `return` must use the cloned constant, while the original is left
/// untouched.
#[test]
fn clone_function_remaps_operands() -> Result<()> {
    let ctx = &mut Context::new();

    // Builds a module with `fn foo() { c0 = const 0; return c0 }`.
    let (_module, func, const_op, ret_op) = const_ret_in_mod(ctx)?;

    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    let cloned_func = clone_operation(func.get_operation(), ctx, &mut rewriter, &mut mapper);

    // The clone is a distinct operation, recorded in the mapping.
    assert_ne!(cloned_func, func.get_operation());
    assert_eq!(mapper.lookup_op(func.get_operation()), Some(cloned_func));

    // The constant's result maps to a fresh value in the clone.
    let orig_const_val = const_op.get_operation().deref(ctx).get_result(0);
    let cloned_const_val = mapper
        .lookup_value(orig_const_val)
        .expect("constant result should be mapped");
    assert_ne!(orig_const_val, cloned_const_val);

    // The cloned return uses the cloned constant; the original is unchanged.
    let cloned_ret = mapper
        .lookup_op(ret_op.get_operation())
        .expect("return should be mapped");
    assert_eq!(cloned_ret.deref(ctx).get_operand(0), cloned_const_val);
    assert_eq!(
        ret_op.get_operation().deref(ctx).get_operand(0),
        orig_const_val
    );

    // The clone is a structurally valid operation in its own right.
    verify_operation(cloned_func, ctx)?;

    Ok(())
}

/// Cloning the same op twice with independent mappings yields independent
/// clones (no shared state leaks through [IrMapping]).
#[test]
fn clone_is_independent_per_mapping() -> Result<()> {
    let ctx = &mut Context::new();
    let (_module, func, _const_op, _ret_op) = const_ret_in_mod(ctx)?;

    let mut rewriter = IRRewriter::<DummyListener>::default();
    let first = clone_operation(
        func.get_operation(),
        ctx,
        &mut rewriter,
        &mut IrMapping::new(),
    );
    let second = clone_operation(
        func.get_operation(),
        ctx,
        &mut rewriter,
        &mut IrMapping::new(),
    );

    assert_ne!(first, func.get_operation());
    assert_ne!(second, func.get_operation());
    assert_ne!(first, second);

    let ignore = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };
    assert_equivalent(
        ctx,
        operation_eq(ctx, &mut IrMapping::new(), first, second, &ignore),
    );

    Ok(())
}

/// Check that a branch that points "forward" to a later block,
/// and an op that uses a block argument, both resolve to their clones.
/// We build:
///
/// ```text
///   A:        c = const 7;  br [c] -> B
///   B(arg):   return arg
/// ```
#[test]
fn clone_blocks_remaps_branches_and_block_args() -> Result<()> {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);

    let module = ModuleOp::new(ctx, "m".try_into().unwrap());
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let region = func.get_region(ctx);

    // Block A is the entry: `c = const 7; br [c] -> B`.
    let block_a = func.get_entry_block(ctx);
    let c = ConstantOp::new(ctx, 7);
    c.get_operation().insert_at_back(block_a, ctx);

    // Block B takes one argument and returns it.
    let block_b = BasicBlock::new(ctx, None, vec![i64_ty.into()]);
    block_b.insert_at_back(region, ctx);
    let b_arg = block_b.deref(ctx).get_argument(0);
    ReturnOp::new(ctx, b_arg)
        .get_operation()
        .insert_at_back(block_b, ctx);

    // A's branch carries `c` to B (B is listed after A, so this is a forward ref).
    let br = Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![c.get_result(ctx)],
        vec![block_b],
        0,
    );
    br.insert_at_back(block_a, ctx);

    // Clone both blocks into the region.
    // The order should be irrelevant so pass them A-before-B here.
    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    clone_blocks_into(&[block_a, block_b], region, ctx, &mut rewriter, &mut mapper);

    let a2 = mapper.lookup_block(block_a).expect("A should be mapped");
    let b2 = mapper.lookup_block(block_b).expect("B should be mapped");
    assert_ne!(a2, block_a);
    assert_ne!(b2, block_b);

    // The constant was cloned to a fresh value.
    let c2 = mapper
        .lookup_value(c.get_result(ctx))
        .expect("constant result should be mapped");
    assert_ne!(c2, c.get_result(ctx));

    // A's clone branches to B's clone (forward reference resolved), passing the
    // cloned constant (operand remapped).
    let a2_term = a2
        .deref(ctx)
        .get_terminator(ctx)
        .expect("A's clone has a terminator");
    assert_eq!(sole_successor(ctx, a2_term), b2);
    assert_eq!(a2_term.deref(ctx).get_operand(0), c2);

    // B's clone has its own fresh argument, and its return reads that argument.
    let b2_arg = b2.deref(ctx).get_argument(0);
    assert_eq!(mapper.lookup_value(b_arg), Some(b2_arg));
    let b2_term = b2
        .deref(ctx)
        .get_terminator(ctx)
        .expect("B's clone has a terminator");
    assert_eq!(b2_term.deref(ctx).get_operand(0), b2_arg);

    Ok(())
}

/// Check that back references are resolved.
#[test]
fn clone_blocks_resolves_back_edge() -> Result<()> {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);

    let module = ModuleOp::new(ctx, "m".try_into().unwrap());
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let region = func.get_region(ctx);

    // A branches to B; B branches back to A.
    let block_a = func.get_entry_block(ctx);
    let block_b = BasicBlock::new(ctx, None, vec![]);
    block_b.insert_at_back(region, ctx);
    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![block_b],
        0,
    )
    .insert_at_back(block_a, ctx);
    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![block_a],
        0,
    )
    .insert_at_back(block_b, ctx);

    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    clone_blocks_into(&[block_a, block_b], region, ctx, &mut rewriter, &mut mapper);

    let a2 = mapper.lookup_block(block_a).expect("A should be mapped");
    let b2 = mapper.lookup_block(block_b).expect("B should be mapped");

    // A' -> B' (forward) and B' -> A' (back-edge), both resolved to the clones.
    let a2_term = a2.deref(ctx).get_terminator(ctx).expect("A' terminator");
    let b2_term = b2.deref(ctx).get_terminator(ctx).expect("B' terminator");
    assert_eq!(sole_successor(ctx, a2_term), b2);
    assert_eq!(sole_successor(ctx, b2_term), a2);

    Ok(())
}

/// Check that clone is **order-independent** for op results too.
///
/// We Build:
///
/// ```text
///   A:   c = const 7;  br -> B
///   B:   return c
/// ```
#[test]
fn clone_blocks_resolves_op_result_forward_ref_in_any_order() -> Result<()> {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);

    let module = ModuleOp::new(ctx, "m".try_into().unwrap());
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let region = func.get_region(ctx);

    // A defines the constant and branches to B; B returns it. `c` is an op result
    // in A used by an op in B (a cross-block use, legal because A dominates B).
    let block_a = func.get_entry_block(ctx);
    let c = ConstantOp::new(ctx, 7);
    c.get_operation().insert_at_back(block_a, ctx);
    let c_val = c.get_result(ctx);
    let block_b = BasicBlock::new(ctx, None, vec![]);
    block_b.insert_at_back(region, ctx);
    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![block_b],
        0,
    )
    .insert_at_back(block_a, ctx);
    ReturnOp::new(ctx, c_val)
        .get_operation()
        .insert_at_back(block_b, ctx);

    // Pass the blocks in the "wrong" (non-dominance) order: B before A.
    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    clone_blocks_into(&[block_b, block_a], region, ctx, &mut rewriter, &mut mapper);

    let c2 = mapper
        .lookup_value(c_val)
        .expect("constant result should be mapped");
    assert_ne!(c2, c_val, "the constant must be cloned to a fresh value");

    let b2 = mapper.lookup_block(block_b).expect("B should be mapped");
    let b2_term = b2.deref(ctx).get_terminator(ctx).expect("B' terminator");
    // B's clone returns the *cloned* constant, not A's original.
    assert_eq!(b2_term.deref(ctx).get_operand(0), c2);
    assert_ne!(b2_term.deref(ctx).get_operand(0), c_val);

    Ok(())
}

/// Check listner notifications
#[test]

fn clone_blocks_notifies_rewriter_listener() -> Result<()> {
    let ctx = &mut Context::new();

    let (_module, func, _const_op, _ret_op) = const_ret_in_mod(ctx)?;
    let region = func.get_region(ctx);
    let src = func.get_entry_block(ctx);

    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<Recorder>::default();
    clone_blocks_into(&[src], region, ctx, &mut rewriter, &mut mapper);

    let mut inserted_blocks = 0;
    let mut inserted_ops = 0;
    for event in &rewriter.get_listener().events {
        match event {
            RecorderEvent::InsertedBlock(_) => inserted_blocks += 1,
            RecorderEvent::InsertedOperation(_) => inserted_ops += 1,
            other => panic!("unexpected event during cloning: {other:?}"),
        }
    }
    assert_eq!(inserted_blocks, 1, "one cloned block should be notified");
    assert_eq!(
        inserted_ops, 2,
        "both cloned ops (constant + return) should be notified"
    );

    Ok(())
}

/// References to external (outside the cloned region) values must remain as-is.
#[test]
fn clone_blocks_keeps_external_value_shared() -> Result<()> {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);

    let module = ModuleOp::new(ctx, "m".try_into().unwrap());
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let region = func.get_region(ctx);

    // A defines the constant and branches to B; B returns it.
    let block_a = func.get_entry_block(ctx);
    let c = ConstantOp::new(ctx, 7);
    c.get_operation().insert_at_back(block_a, ctx);
    let c_val = c.get_result(ctx);
    let block_b = BasicBlock::new(ctx, None, vec![]);
    block_b.insert_at_back(region, ctx);
    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![block_b],
        0,
    )
    .insert_at_back(block_a, ctx);
    ReturnOp::new(ctx, c_val)
        .get_operation()
        .insert_at_back(block_b, ctx);

    // Clone ONLY B; the constant `c` lives in A, outside the cloned set.
    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    clone_blocks_into(&[block_b], region, ctx, &mut rewriter, &mut mapper);

    let b2 = mapper.lookup_block(block_b).expect("B should be mapped");
    let b2_term = b2.deref(ctx).get_terminator(ctx).expect("B' terminator");
    // The clone still returns A's original constant.
    assert_eq!(b2_term.deref(ctx).get_operand(0), c_val);
    // ...and the mapping has no entry for that external value.
    assert_eq!(mapper.lookup_value(c_val), None);

    // B and its clone are equivalent.
    let ignore = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };
    assert_equivalent(
        ctx,
        basic_block_eq(ctx, &mut IrMapping::new(), block_b, b2, &ignore),
    );

    Ok(())
}

/// Check that attributes are cloned correctly.
#[test]
fn clone_blocks_copies_block_label_and_attributes() -> Result<()> {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);

    let module = ModuleOp::new(ctx, "m".try_into().unwrap());
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let region = func.get_region(ctx);

    // Give the source block a label and an attribute (a stand-in for block
    // debug info).
    let src = func.get_entry_block(ctx);
    let label: Identifier = "myblock".try_into().unwrap();
    src.deref_mut(ctx).set_label(Some(label.clone()));
    let key: Identifier = "test_block_attr".try_into().unwrap();
    src.deref_mut(ctx).attributes.set(
        key.clone(),
        IntegerAttr::new(i64_ty, APInt::from_u64(42, bw(64))),
    );

    let mut mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    clone_blocks_into(&[src], region, ctx, &mut rewriter, &mut mapper);
    let clone = mapper.lookup_block(src).expect("block should be mapped");

    let clone_ref = clone.deref(ctx);
    // The clone carries the same attribute.
    let copied = clone_ref
        .attributes
        .get::<IntegerAttr>(&key)
        .expect("block attribute should be copied to the clone");
    assert_eq!(copied.value().to_u64(), 42);
    // ... and the same label (given_name), but a distinct unique_name.
    assert_eq!(clone_ref.given_name(ctx), Some(label));
    assert_ne!(
        clone_ref.unique_name(ctx),
        src.deref(ctx).unique_name(ctx),
        "the clone must be a distinct block with its own unique_name"
    );

    Ok(())
}

#[test]
fn clone_is_equivalent_to_original() -> Result<()> {
    let ctx = &mut Context::new();
    let (_module, func, _const_op, _ret_op) = const_ret_in_mod(ctx)?;

    let mut clone_mapper = IrMapping::new();
    let mut rewriter = IRRewriter::<DummyListener>::default();
    let cloned_func = clone_operation(func.get_operation(), ctx, &mut rewriter, &mut clone_mapper);

    let ignore = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };
    let mut eq_mapper = IrMapping::new();
    assert_equivalent(
        ctx,
        operation_eq(
            ctx,
            &mut eq_mapper,
            func.get_operation(),
            cloned_func,
            &ignore,
        ),
    );

    Ok(())
}

/// Two structurally identical functions, built independently, must be are equivalent.
#[test]
fn equivalent_functions_with_a_forward_branch_are_equal() -> Result<()> {
    let ctx = &mut Context::new();
    let (func1, _) = branch_and_return_fn(ctx, "m1", 7)?;
    let (func2, _) = branch_and_return_fn(ctx, "m2", 7)?;

    let ignore = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };

    // The whole function, through operation_eq (which recurses into the
    // region and both blocks).
    let mut mapper = IrMapping::new();
    assert_equivalent(
        ctx,
        operation_eq(
            ctx,
            &mut mapper,
            func1.get_operation(),
            func2.get_operation(),
            &ignore,
        ),
    );

    // The same, driven directly through region_eq.
    let mut mapper = IrMapping::new();
    assert_equivalent(
        ctx,
        region_eq(
            ctx,
            &mut mapper,
            func1.get_region(ctx),
            func2.get_region(ctx),
            &ignore,
        ),
    );

    Ok(())
}

/// Check that an attribute difference deep down is flagged.
#[test]
fn differing_constant_value_is_not_equivalent() -> Result<()> {
    let ctx = &mut Context::new();
    let (func1, _) = branch_and_return_fn(ctx, "m1", 7)?;
    let (func2, _) = branch_and_return_fn(ctx, "m2", 9)?;

    let ignore = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };
    let mut mapper = IrMapping::new();
    assert_not_equivalent(operation_eq(
        ctx,
        &mut mapper,
        func1.get_operation(),
        func2.get_operation(),
        &ignore,
    ));

    Ok(())
}

#[test]
fn region_and_block_mismatches_report_their_own_variant() -> Result<()> {
    let ctx = &mut Context::new();
    let ignore = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };

    // Two regions with a differing number of blocks.
    let single_block_func = const_ret_in_mod(ctx).map(|(_, f, _, _)| f)?;
    let (branching_func, _) = branch_and_return_fn(ctx, "m1", 7)?;
    let mut mapper = IrMapping::new();
    let result = region_eq(
        ctx,
        &mut mapper,
        single_block_func.get_region(ctx),
        branching_func.get_region(ctx),
        &ignore,
    );
    assert!(
        matches!(result, EqResult::FirstNEQRegions(_)),
        "a block-count mismatch should be reported at the region level"
    );

    // Two blocks with a differing number of arguments.
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let with_arg = BasicBlock::new(ctx, None, vec![i64_ty.into()]);
    let without_arg = BasicBlock::new(ctx, None, vec![]);
    let mut mapper = IrMapping::new();
    let result = basic_block_eq(ctx, &mut mapper, with_arg, without_arg, &ignore);
    assert!(
        matches!(result, EqResult::FirstNEQBlocks(_)),
        "an argument-count mismatch should be reported at the block level"
    );

    Ok(())
}

/// Test [IgnoreConfig] behaviour.
#[test]
fn ignore_config_skips_location_and_chosen_attributes() -> Result<()> {
    let ctx = &mut Context::new();
    let (func1, const1) = branch_and_return_fn(ctx, "m1", 7)?;
    let (func2, const2) = branch_and_return_fn(ctx, "m2", 9)?;

    const1.deref_mut(ctx).set_loc(Location::Named {
        name: "one".into(),
        child_loc: Box::new(Location::Unknown),
    });
    const2.deref_mut(ctx).set_loc(Location::Named {
        name: "two".into(),
        child_loc: Box::new(Location::Unknown),
    });

    // Ignoring both the location and the (only) attribute in play: equivalent.
    let ignore_both = IgnoreConfig {
        ignore_loc: true,
        ignore_attr: ignore_integer_attrs,
    };
    let mut mapper = IrMapping::new();
    assert_equivalent(
        ctx,
        operation_eq(
            ctx,
            &mut mapper,
            func1.get_operation(),
            func2.get_operation(),
            &ignore_both,
        ),
    );

    // Ignoring neither: the location (and the attribute) differences surface.
    let ignore_neither = IgnoreConfig {
        ignore_loc: false,
        ignore_attr: never_ignore,
    };
    let mut mapper = IrMapping::new();
    assert_not_equivalent(operation_eq(
        ctx,
        &mut mapper,
        func1.get_operation(),
        func2.get_operation(),
        &ignore_neither,
    ));

    Ok(())
}
