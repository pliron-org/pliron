//! Tests for IR cloning ([pliron::irbuild::cloning]).

use common::{ConstantOp, ReturnOp, const_ret_in_mod};
use pliron::{
    basic_block::BasicBlock,
    builtin::{
        op_interfaces::{
            IsTerminatorInterface, OneRegionInterface, OneResultInterface,
            SingleBlockRegionInterface,
        },
        ops::{FuncOp, ModuleOp},
        types::{FunctionType, IntegerType, Signedness},
    },
    context::{Context, Ptr},
    derive::pliron_op,
    irbuild::cloning::{IrMapping, clone_blocks_into, clone_operation},
    op::Op,
    operation::{Operation, verify_operation},
    result::Result,
};

#[cfg(target_family = "wasm")]
use wasm_bindgen_test::*;

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

/// Cloning a function deep-copies its body and remaps intra-region operands:
/// the cloned `return` must use the cloned constant, while the original is left
/// untouched.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn clone_function_remaps_operands() -> Result<()> {
    let ctx = &mut Context::new();

    // Builds a module with `fn foo() { c0 = const 0; return c0 }`.
    let (_module, func, const_op, ret_op) = const_ret_in_mod(ctx)?;

    let mut mapper = IrMapping::new();
    let cloned_func = clone_operation(func.get_operation(), ctx, &mut mapper);

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
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn clone_is_independent_per_mapping() -> Result<()> {
    let ctx = &mut Context::new();
    let (_module, func, _const_op, _ret_op) = const_ret_in_mod(ctx)?;

    let first = clone_operation(func.get_operation(), ctx, &mut IrMapping::new());
    let second = clone_operation(func.get_operation(), ctx, &mut IrMapping::new());

    assert_ne!(first, func.get_operation());
    assert_ne!(second, func.get_operation());
    assert_ne!(first, second);

    Ok(())
}

/// Cloning a set of blocks is two-phase: every clone block and its block
/// arguments are recorded first, then ops are cloned. So a branch that points
/// "forward" to a later block, and an op that uses a block argument, both resolve
/// to their clones. We build
///
/// ```text
///   A:        c = const 7;  br [c] -> B
///   B(arg):   return arg
/// ```
///
/// clone both blocks, and check the clone of A branches to the clone of B (the
/// forward reference is resolved), carries the cloned constant (operand
/// remapped), and the clone of B returns its own fresh argument (block-arg
/// remapped).
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
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

    // Clone both blocks (A before B, i.e. reverse-post-order) into the region.
    let mut mapper = IrMapping::new();
    clone_blocks_into(&[block_a, block_b], region, ctx, &mut mapper);

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

/// The two-phase clone also resolves *back* references (a block branching to an
/// earlier block in the list), not just forward ones. Build a two-block loop
/// `A -> B -> A`, clone both, and check the clone of B branches back to the clone
/// of A, not the original A.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
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
    clone_blocks_into(&[block_a, block_b], region, ctx, &mut mapper);

    let a2 = mapper.lookup_block(block_a).expect("A should be mapped");
    let b2 = mapper.lookup_block(block_b).expect("B should be mapped");

    // A' -> B' (forward) and B' -> A' (back-edge), both resolved to the clones.
    let a2_term = a2.deref(ctx).get_terminator(ctx).expect("A' terminator");
    let b2_term = b2.deref(ctx).get_terminator(ctx).expect("B' terminator");
    assert_eq!(sole_successor(ctx, a2_term), b2);
    assert_eq!(sole_successor(ctx, b2_term), a2);

    Ok(())
}

/// A value defined outside the cloned set must stay shared (MLIR's
/// `lookupOrDefault`): the clone keeps pointing at the original, and the mapping
/// has no entry for it. Build `A: c = const 7; br -> B` and `B: return c`, but
/// clone ONLY B. B's clone must still return A's original constant.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
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
    clone_blocks_into(&[block_b], region, ctx, &mut mapper);

    let b2 = mapper.lookup_block(block_b).expect("B should be mapped");
    let b2_term = b2.deref(ctx).get_terminator(ctx).expect("B' terminator");
    // The clone still returns A's original constant (shared, not remapped)...
    assert_eq!(b2_term.deref(ctx).get_operand(0), c_val);
    // ...and the mapping has no entry for that external value.
    assert_eq!(mapper.lookup_value(c_val), None);

    Ok(())
}
