use common::{ConstantOp, ReturnOp, const_ret_in_mod};
use expect_test::{Expect, expect};
use pliron::{
    basic_block::{BasicBlock, BasicBlockVerifyErr},
    builtin::{
        attributes::StringAttr,
        op_interfaces::{
            IsTerminatorInterface, OneRegionInterface, OneResultInterface,
            SingleBlockRegionInterface,
        },
        types::{IntegerType, Signedness},
    },
    combine::parser::Parser,
    context::{Context, Ptr},
    debug_info::{
        get_block_arg_name, get_operation_result_name, set_block_arg_name,
        set_operation_result_name,
    },
    derive::pliron_op,
    dict_key,
    graph::walkers::{
        self, IRNode, WALKCONFIG_POSTORDER_FORWARD, WALKCONFIG_POSTORDER_REVERSE,
        WALKCONFIG_PREORDER_FORWARD,
        interruptible::{self, walk_advance, walk_break},
    },
    irfmt::parsers::spaced,
    location,
    op::{Op, verify_op},
    operation::{DefUseVerifyErr, Operation, verify_operation},
    parsable::{self, state_stream_from_iterator},
    printable::Printable,
    result::Result,
    r#type::TypeObj,
};

#[cfg(target_family = "wasm")]
use wasm_bindgen_test::*;

mod common;

// Test erasing the entire top module.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn construct_and_erase() -> Result<()> {
    let ctx = &mut Context::new();
    let module_op = const_ret_in_mod(ctx)?.0.get_operation();
    Operation::erase(module_op, ctx);
    assert!(ctx.is_ir_empty());
    Ok(())
}

// Ensure that erasing an op with uses panics.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
#[should_panic]
fn removed_used_op() {
    let ctx = &mut Context::new();

    // const_ret_in_mod builds a module with a function.
    let (_, _, const_op, _) = const_ret_in_mod(ctx).unwrap();

    // const_op is used in the return. Erasing it must panic.
    Operation::erase(const_op.get_operation(), ctx);
}

// Testing replacing all uses of c0 with c1.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn replace_c0_with_c1() -> Result<()> {
    let ctx = &mut Context::new();

    // const_ret_in_mod builds a module with a function.
    let (module_op, _, const_op, _) = const_ret_in_mod(ctx).unwrap();

    let const1_op = ConstantOp::new(ctx, 1);
    const1_op
        .get_operation()
        .insert_after(ctx, const_op.get_operation());
    set_operation_result_name(
        ctx,
        const1_op.get_operation(),
        0,
        Some("c1".try_into().unwrap()),
    );
    let const0_val = const_op.get_result(ctx);
    const0_val.replace_some_uses_with(ctx, |_, _| true, &const1_op.get_result(ctx));

    Operation::erase(const_op.get_operation(), ctx);

    verify_op(&module_op, ctx)?;

    Ok(())
}

// Replace ret_op's first operand (which is c0) with c1.
// Erase c0. Verify.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn replace_c0_with_c1_operand() -> Result<()> {
    let ctx = &mut Context::new();

    // const_ret_in_mod builds a module with a function.
    let (module_op, _, const_op, ret_op) = const_ret_in_mod(ctx).unwrap();

    let const1_op = ConstantOp::new(ctx, 1);
    const1_op
        .get_operation()
        .insert_after(ctx, const_op.get_operation());
    set_operation_result_name(
        ctx,
        const1_op.get_operation(),
        0,
        Some("c1".try_into().unwrap()),
    );

    let printed = format!("{}", module_op.get_operation().disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                c1_v1 = test.constant builtin.integer <1: si64> !1;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]
        !1 = [builtin_debug_info = builtin.debug_info [c1]]
    "#]]
    .assert_eq(&printed);

    Operation::replace_operand(ret_op.get_operation(), ctx, 0, const1_op.get_result(ctx));
    Operation::erase(const_op.get_operation(), ctx);

    let printed = format!("{}", module_op.get_operation().disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c1_v1 = test.constant builtin.integer <1: si64> !0;
                test.return c1_v1
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c1]]
    "#]]
    .assert_eq(&printed);

    verify_op(&module_op, ctx)?;

    Ok(())
}

#[pliron_op(name = "test.dual_def", format, verifier = "succ")]
struct DualDefOp {}

/// If an Op has multiple results, or a block multiple args,
/// replacing all uses of one with the other should work.
/// (since our RefCell is at the Op or block level, we shouldn't
/// end up with a multiple borrow panic).
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_replace_within_same_def_site() {
    let ctx = &mut Context::new();

    let u64_ty = IntegerType::get(ctx, 64, Signedness::Signed).into();

    let dual_def_op = Operation::new(
        ctx,
        DualDefOp::get_concrete_op_info(),
        vec![u64_ty, u64_ty],
        vec![],
        vec![],
        0,
    );
    let (res1, res2) = (
        dual_def_op.deref(ctx).get_result(0),
        dual_def_op.deref(ctx).get_result(1),
    );
    let (module_op, func_op, const_op, ret_op) = const_ret_in_mod(ctx).unwrap();
    dual_def_op.insert_before(ctx, ret_op.get_operation());
    const_op
        .get_result(ctx)
        .replace_some_uses_with(ctx, |_, _| true, &res1);
    res1.replace_some_uses_with(ctx, |_, _| true, &res2);
    let printed = format!("{}", module_op.get_operation().disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v2 = test.constant builtin.integer <0: si64> !0;
                v0, v1 = test.dual_def () [] []: <() -> (builtin.integer si64, builtin.integer si64)>;
                test.return v1
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]
    "#]]
    .assert_eq(&printed);

    let dual_arg_block = BasicBlock::new(ctx, None, vec![u64_ty, u64_ty]);
    let (arg1, arg2) = (
        dual_arg_block.deref(ctx).get_argument(0),
        dual_arg_block.deref(ctx).get_argument(1),
    );
    dual_arg_block.insert_after(ctx, func_op.get_entry_block(ctx));
    let ret_op = ReturnOp::new(ctx, arg1);
    ret_op.get_operation().insert_at_back(dual_arg_block, ctx);
    arg1.replace_some_uses_with(ctx, |_, _| true, &arg2);

    let printed = format!("{}", module_op.get_operation().disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v2 = test.constant builtin.integer <0: si64> !0;
                v0, v1 = test.dual_def () [] []: <() -> (builtin.integer si64, builtin.integer si64)>;
                test.return v1

              ^block3v1(v3: builtin.integer si64, v4: builtin.integer si64):
                test.return v4
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]
    "#]]
    .assert_eq(&printed);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_operand_push_pop_insert_remove() -> Result<()> {
    let ctx = &mut Context::new();

    let (module_op, _func_op, const0_op, ret_op) = const_ret_in_mod(ctx)?;
    let ret_ptr = ret_op.get_operation();

    let const1_op = ConstantOp::new(ctx, 1);
    const1_op
        .get_operation()
        .insert_before(ctx, ret_op.get_operation());
    let const2_op = ConstantOp::new(ctx, 2);
    const2_op
        .get_operation()
        .insert_before(ctx, ret_op.get_operation());

    let c0 = const0_op.get_result(ctx);
    let c1 = const1_op.get_result(ctx);
    let c2 = const2_op.get_result(ctx);

    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 1);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c0);

    let pushed_idx = Operation::push_operand(ret_ptr, ctx, c1);
    assert_eq!(pushed_idx, 1);
    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 2);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c0);
    assert_eq!(ret_ptr.deref(ctx).get_operand(1), c1);
    for r#use in c1.uses(ctx) {
        assert!(r#use.get_def(ctx) == c1);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 1);
    }
    for r#use in c0.uses(ctx) {
        assert!(r#use.get_def(ctx) == c0);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 0);
    }
    verify_op(&module_op, ctx)?;

    let popped = Operation::pop_operand(ret_ptr, ctx);
    assert_eq!(popped, c1);
    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 1);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c0);
    assert!(c1.uses(ctx).is_empty()); // c1 should have no uses now.
    for r#use in c0.uses(ctx) {
        assert!(r#use.get_def(ctx) == c0);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 0);
    }
    verify_op(&module_op, ctx)?;

    Operation::insert_operand(ret_ptr, ctx, 0, c1);
    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 2);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c1);
    assert_eq!(ret_ptr.deref(ctx).get_operand(1), c0);
    for r#use in c1.uses(ctx) {
        assert!(r#use.get_def(ctx) == c1);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 0);
    }
    for r#use in c0.uses(ctx) {
        assert!(r#use.get_def(ctx) == c0);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 1);
    }
    verify_op(&module_op, ctx)?;

    Operation::insert_operand(ret_ptr, ctx, 2, c2);
    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 3);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c1);
    assert_eq!(ret_ptr.deref(ctx).get_operand(1), c0);
    assert_eq!(ret_ptr.deref(ctx).get_operand(2), c2);
    for r#use in c1.uses(ctx) {
        assert!(r#use.get_def(ctx) == c1);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 0);
    }
    for r#use in c0.uses(ctx) {
        assert!(r#use.get_def(ctx) == c0);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 1);
    }
    for r#use in c2.uses(ctx) {
        assert!(r#use.get_def(ctx) == c2);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 2);
    }

    verify_op(&module_op, ctx)?;

    let removed_mid = Operation::remove_operand(ret_ptr, ctx, 1);
    assert_eq!(removed_mid, c0);
    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 2);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c1);
    assert_eq!(ret_ptr.deref(ctx).get_operand(1), c2);
    for r#use in c1.uses(ctx) {
        assert!(r#use.get_def(ctx) == c1);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 0);
    }
    for r#use in c2.uses(ctx) {
        assert!(r#use.get_def(ctx) == c2);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 1);
    }
    verify_op(&module_op, ctx)?;

    let removed_front = Operation::remove_operand(ret_ptr, ctx, 0);
    assert_eq!(removed_front, c1);
    assert_eq!(ret_ptr.deref(ctx).get_num_operands(), 1);
    assert_eq!(ret_ptr.deref(ctx).get_operand(0), c2);
    for r#use in c2.uses(ctx) {
        assert!(r#use.get_def(ctx) == c2);
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 0);
    }
    verify_op(&module_op, ctx)?;

    // Add c2 as an operand again,
    Operation::insert_operand(ret_ptr, ctx, 0, c2);
    assert!(c2.uses(ctx).len() == 2); // c2 should now have two uses.
    for r#use in c2.uses(ctx) {
        assert!(r#use.get_def(ctx) == c2);
        // c2 should now have two uses.
        assert!(
            r#use.user_op() == ret_ptr
                && (r#use.find_index(ctx) == 0 || r#use.find_index(ctx) == 1)
        );
    }

    // Add c0 now
    Operation::insert_operand(ret_ptr, ctx, 1, c0);
    assert!(c0.uses(ctx).len() == 1); // c0 should now have 1 use.
    assert!(c2.uses(ctx).len() == 2); // c2 should still have two uses.
    for r#use in c0.uses(ctx) {
        assert!(r#use.get_def(ctx) == c0);
        // c0 should now have one use.
        assert!(r#use.user_op() == ret_ptr && r#use.find_index(ctx) == 1);
    }
    for r#use in c2.uses(ctx) {
        assert!(r#use.get_def(ctx) == c2);
        // c2 should still have two uses.
        assert!(
            r#use.user_op() == ret_ptr
                && (r#use.find_index(ctx) == 0 || r#use.find_index(ctx) == 2)
        );
    }

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_result_push_pop_insert_remove() -> Result<()> {
    let ctx = &mut Context::new();

    let i64_ty: Ptr<TypeObj> = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let i32_ty: Ptr<TypeObj> = IntegerType::get(ctx, 32, Signedness::Signed).into();

    // Create a DualDefOp with two i64 results.
    let op = Operation::new(
        ctx,
        DualDefOp::get_concrete_op_info(),
        vec![i64_ty, i64_ty],
        vec![],
        vec![],
        0,
    );

    let r0 = op.deref(ctx).get_result(0);
    let r1 = op.deref(ctx).get_result(1);

    set_operation_result_name(ctx, op, 0, Some("r0".try_into().unwrap()));
    set_operation_result_name(ctx, op, 1, Some("r1".try_into().unwrap()));

    assert_eq!(op.deref(ctx).get_num_results(), 2);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);
    assert_eq!(op.deref(ctx).get_type(0), i64_ty);
    assert_eq!(op.deref(ctx).get_type(1), i64_ty);
    assert_eq!(
        get_operation_result_name(ctx, op, 0),
        Some("r0".try_into().unwrap())
    );
    assert_eq!(
        get_operation_result_name(ctx, op, 1),
        Some("r1".try_into().unwrap())
    );

    // Push a new i32 result at the end.
    let pushed_idx = Operation::push_result(op, ctx, i32_ty);
    assert_eq!(pushed_idx, 2);
    assert_eq!(op.deref(ctx).get_num_results(), 3);
    // r0 and r1 still have the same indices.
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);
    let r2 = op.deref(ctx).get_result(2);
    assert_eq!(r2.find_index(ctx), 2);
    assert_eq!(op.deref(ctx).get_type(2), i32_ty);
    assert_eq!(get_operation_result_name(ctx, op, 2), None);

    // Pop the last result (r2 has no uses).
    Operation::pop_result(op, ctx);
    assert_eq!(op.deref(ctx).get_num_results(), 2);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);
    assert_eq!(
        get_operation_result_name(ctx, op, 0),
        Some("r0".try_into().unwrap())
    );
    assert_eq!(
        get_operation_result_name(ctx, op, 1),
        Some("r1".try_into().unwrap())
    );

    // Insert an i32 result at index 0, shifting r0 -> 1 and r1 -> 2.
    Operation::insert_result(op, ctx, 0, i32_ty);
    assert_eq!(op.deref(ctx).get_num_results(), 3);
    assert_eq!(op.deref(ctx).get_type(0), i32_ty);
    assert_eq!(r0.find_index(ctx), 1);
    assert_eq!(r1.find_index(ctx), 2);
    assert_eq!(get_operation_result_name(ctx, op, 0), None);
    assert_eq!(
        get_operation_result_name(ctx, op, 1),
        Some("r0".try_into().unwrap())
    );
    assert_eq!(
        get_operation_result_name(ctx, op, 2),
        Some("r1".try_into().unwrap())
    );

    // Remove index 0 (the freshly inserted i32, which has no uses).
    Operation::remove_result(op, ctx, 0);
    assert_eq!(op.deref(ctx).get_num_results(), 2);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);
    assert_eq!(
        get_operation_result_name(ctx, op, 0),
        Some("r0".try_into().unwrap())
    );
    assert_eq!(
        get_operation_result_name(ctx, op, 1),
        Some("r1".try_into().unwrap())
    );

    // Insert an i32 result at index 1 (between r0 and r1), shifting r1 -> 2.
    Operation::insert_result(op, ctx, 1, i32_ty);
    assert_eq!(op.deref(ctx).get_num_results(), 3);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(op.deref(ctx).get_type(1), i32_ty);
    assert_eq!(r1.find_index(ctx), 2);

    // Remove index 1 (no uses), shifting r1 back to 1.
    Operation::remove_result(op, ctx, 1);
    assert_eq!(op.deref(ctx).get_num_results(), 2);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);

    // Insert at the end (index 2).
    Operation::insert_result(op, ctx, 2, i32_ty);
    assert_eq!(op.deref(ctx).get_num_results(), 3);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);
    assert_eq!(op.deref(ctx).get_type(2), i32_ty);
    assert_eq!(
        get_operation_result_name(ctx, op, 0),
        Some("r0".try_into().unwrap())
    );
    assert_eq!(
        get_operation_result_name(ctx, op, 1),
        Some("r1".try_into().unwrap())
    );
    assert_eq!(get_operation_result_name(ctx, op, 2), None);

    // Remove from the end.
    Operation::remove_result(op, ctx, 2);
    assert_eq!(op.deref(ctx).get_num_results(), 2);
    assert_eq!(r0.find_index(ctx), 0);
    assert_eq!(r1.find_index(ctx), 1);
    assert_eq!(
        get_operation_result_name(ctx, op, 0),
        Some("r0".try_into().unwrap())
    );
    assert_eq!(
        get_operation_result_name(ctx, op, 1),
        Some("r1".try_into().unwrap())
    );

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_block_arg_push_pop_insert_remove() -> Result<()> {
    let ctx = &mut Context::new();

    let i64_ty: Ptr<TypeObj> = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let i32_ty: Ptr<TypeObj> = IntegerType::get(ctx, 32, Signedness::Signed).into();

    // Create a block with two i64 arguments.
    let block = BasicBlock::new(ctx, None, vec![i64_ty, i64_ty]);

    let a0 = block.deref(ctx).get_argument(0);
    let a1 = block.deref(ctx).get_argument(1);

    set_block_arg_name(ctx, block, 0, Some("a0".try_into().unwrap()));
    set_block_arg_name(ctx, block, 1, Some("a1".try_into().unwrap()));

    assert_eq!(block.deref(ctx).get_num_arguments(), 2);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);
    assert_eq!(
        get_block_arg_name(ctx, block, 0),
        Some("a0".try_into().unwrap())
    );
    assert_eq!(
        get_block_arg_name(ctx, block, 1),
        Some("a1".try_into().unwrap())
    );

    // Push a new i32 argument at the end.
    let pushed_idx = BasicBlock::push_argument(block, ctx, i32_ty);
    assert_eq!(pushed_idx, 2);
    assert_eq!(block.deref(ctx).get_num_arguments(), 3);
    // a0 and a1 retain their indices.
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);
    let a2 = block.deref(ctx).get_argument(2);
    assert_eq!(a2.find_index(ctx), 2);
    assert_eq!(get_block_arg_name(ctx, block, 2), None);

    // Pop the last argument (a2 has no uses).
    BasicBlock::pop_argument(block, ctx);
    assert_eq!(block.deref(ctx).get_num_arguments(), 2);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);

    // Insert an i32 argument at index 0, shifting a0 -> 1 and a1 -> 2.
    BasicBlock::insert_argument(block, ctx, 0, i32_ty);
    assert_eq!(block.deref(ctx).get_num_arguments(), 3);
    assert_eq!(a0.find_index(ctx), 1);
    assert_eq!(a1.find_index(ctx), 2);
    assert_eq!(get_block_arg_name(ctx, block, 0), None);
    assert_eq!(
        get_block_arg_name(ctx, block, 1),
        Some("a0".try_into().unwrap())
    );
    assert_eq!(
        get_block_arg_name(ctx, block, 2),
        Some("a1".try_into().unwrap())
    );

    // Remove index 0 (no uses), restoring a0 -> 0 and a1 -> 1.
    BasicBlock::remove_argument(block, ctx, 0);
    assert_eq!(block.deref(ctx).get_num_arguments(), 2);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);
    assert_eq!(
        get_block_arg_name(ctx, block, 0),
        Some("a0".try_into().unwrap())
    );
    assert_eq!(
        get_block_arg_name(ctx, block, 1),
        Some("a1".try_into().unwrap())
    );

    // Insert an i32 argument at index 1 (between a0 and a1), shifting a1 -> 2.
    BasicBlock::insert_argument(block, ctx, 1, i32_ty);
    assert_eq!(block.deref(ctx).get_num_arguments(), 3);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 2);

    // Remove index 1 (no uses), shifting a1 back to 1.
    BasicBlock::remove_argument(block, ctx, 1);
    assert_eq!(block.deref(ctx).get_num_arguments(), 2);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);

    // Insert at the end (index 2).
    BasicBlock::insert_argument(block, ctx, 2, i32_ty);
    assert_eq!(block.deref(ctx).get_num_arguments(), 3);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);
    assert_eq!(
        get_block_arg_name(ctx, block, 0),
        Some("a0".try_into().unwrap())
    );
    assert_eq!(
        get_block_arg_name(ctx, block, 1),
        Some("a1".try_into().unwrap())
    );
    assert_eq!(get_block_arg_name(ctx, block, 2), None);

    // Remove from the end.
    BasicBlock::remove_argument(block, ctx, 2);
    assert_eq!(block.deref(ctx).get_num_arguments(), 2);
    assert_eq!(a0.find_index(ctx), 0);
    assert_eq!(a1.find_index(ctx), 1);
    assert_eq!(
        get_block_arg_name(ctx, block, 0),
        Some("a0".try_into().unwrap())
    );
    assert_eq!(
        get_block_arg_name(ctx, block, 1),
        Some("a1".try_into().unwrap())
    );

    Ok(())
}

/// Tests that `Value` objects held as operands correctly track their
/// index within the defining operation's result list when results are
/// inserted, removed, pushed, or popped ahead of / behind them.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_result_index_tracking_with_uses() -> Result<()> {
    let ctx = &mut Context::new();

    let i64_ty: Ptr<TypeObj> = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let i32_ty: Ptr<TypeObj> = IntegerType::get(ctx, 32, Signedness::Signed).into();

    // Create a DualDefOp with two i64 results: r0 at index 0, r1 at index 1.
    let dual_op = Operation::new(
        ctx,
        DualDefOp::get_concrete_op_info(),
        vec![i64_ty, i64_ty],
        vec![],
        vec![],
        0,
    );

    let r0 = dual_op.deref(ctx).get_result(0);
    let r1 = dual_op.deref(ctx).get_result(1);

    // Build a user op that holds r0 as its first operand, r1 as its second.
    let user_op = ReturnOp::new(ctx, r0).get_operation();
    Operation::push_operand(user_op, ctx, r1);

    // Sanity: operands are r0 and r1; their result indices are 0 and 1.
    assert_eq!(user_op.deref(ctx).get_operand(0), r0);
    assert_eq!(user_op.deref(ctx).get_operand(1), r1);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Insert a new i32 result at index 0 of dual_op, shifting r0 -> 1 and r1 -> 2.
    Operation::insert_result(dual_op, ctx, 0, i32_ty);
    assert_eq!(dual_op.deref(ctx).get_num_results(), 3);
    assert_eq!(user_op.deref(ctx).get_operand(0), r0);
    assert_eq!(user_op.deref(ctx).get_operand(1), r1);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 1);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 2);

    // Remove the new result at index 0, restoring r0 -> 0 and r1 -> 1.
    Operation::remove_result(dual_op, ctx, 0);
    assert_eq!(dual_op.deref(ctx).get_num_results(), 2);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Insert a new result between r0 and r1 (at index 1), shifting r1 -> 2.
    Operation::insert_result(dual_op, ctx, 1, i32_ty);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 2);

    // Remove at index 1, restoring r1 -> 1.
    Operation::remove_result(dual_op, ctx, 1);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Push a result at the end (after r1); r0 and r1 indices are unchanged.
    let pushed_idx = Operation::push_result(dual_op, ctx, i32_ty);
    assert_eq!(pushed_idx, 2);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Pop the trailing result; indices still unchanged.
    Operation::pop_result(dual_op, ctx);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    Ok(())
}

/// Tests that `Value` objects held as operands correctly track their
/// index within the defining block's argument list when arguments are
/// inserted, removed, pushed, or popped ahead of / behind them.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_block_arg_index_tracking_with_uses() -> Result<()> {
    let ctx = &mut Context::new();

    let i64_ty: Ptr<TypeObj> = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let i32_ty: Ptr<TypeObj> = IntegerType::get(ctx, 32, Signedness::Signed).into();

    // Create a block with two i64 arguments: a0 at index 0, a1 at index 1.
    let block = BasicBlock::new(ctx, None, vec![i64_ty, i64_ty]);

    let a0 = block.deref(ctx).get_argument(0);
    let a1 = block.deref(ctx).get_argument(1);

    // Build a user op that holds a0 as its first operand, a1 as its second.
    let user_op = ReturnOp::new(ctx, a0).get_operation();
    Operation::push_operand(user_op, ctx, a1);

    // Sanity: operands are a0 and a1; their argument indices are 0 and 1.
    assert_eq!(user_op.deref(ctx).get_operand(0), a0);
    assert_eq!(user_op.deref(ctx).get_operand(1), a1);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Insert a new i32 argument at index 0, shifting a0 -> 1 and a1 -> 2.
    BasicBlock::insert_argument(block, ctx, 0, i32_ty);
    assert_eq!(block.deref(ctx).get_num_arguments(), 3);
    assert_eq!(user_op.deref(ctx).get_operand(0), a0);
    assert_eq!(user_op.deref(ctx).get_operand(1), a1);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 1);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 2);

    // Remove the new argument at index 0, restoring a0 -> 0 and a1 -> 1.
    BasicBlock::remove_argument(block, ctx, 0);
    assert_eq!(block.deref(ctx).get_num_arguments(), 2);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Insert a new argument between a0 and a1 (at index 1), shifting a1 -> 2.
    BasicBlock::insert_argument(block, ctx, 1, i32_ty);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 2);

    // Remove at index 1, restoring a1 -> 1.
    BasicBlock::remove_argument(block, ctx, 1);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Push an argument at the end (after a1); a0 and a1 indices are unchanged.
    let pushed_idx = BasicBlock::push_argument(block, ctx, i32_ty);
    assert_eq!(pushed_idx, 2);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    // Pop the trailing argument; indices still unchanged.
    BasicBlock::pop_argument(block, ctx);
    assert_eq!(user_op.deref(ctx).get_operand(0).find_index(ctx), 0);
    assert_eq!(user_op.deref(ctx).get_operand(1).find_index(ctx), 1);

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_successor_push_pop_insert_remove() -> Result<()> {
    let ctx = &mut Context::new();

    let (module_op, func_op, const_op, ret_op) = const_ret_in_mod(ctx)?;
    let entry_block = func_op.get_entry_block(ctx);

    let common_pred = BasicBlock::new(ctx, None, vec![]);
    common_pred.insert_after(ctx, entry_block);
    let succ0 = BasicBlock::new(ctx, None, vec![]);
    succ0.insert_after(ctx, common_pred);
    let succ1 = BasicBlock::new(ctx, None, vec![]);
    succ1.insert_after(ctx, succ0);
    let succ2 = BasicBlock::new(ctx, None, vec![]);
    succ2.insert_after(ctx, succ1);

    let succ0_ret = ReturnOp::new(ctx, const_op.get_result(ctx));
    succ0_ret.get_operation().insert_at_back(succ0, ctx);
    let succ1_ret = ReturnOp::new(ctx, const_op.get_result(ctx));
    succ1_ret.get_operation().insert_at_back(succ1, ctx);
    let succ2_ret = ReturnOp::new(ctx, const_op.get_result(ctx));
    succ2_ret.get_operation().insert_at_back(succ2, ctx);

    let branch_like = Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![succ0, succ1, succ2],
        0,
    );
    branch_like.insert_before(ctx, ret_op.get_operation());
    Operation::erase(ret_op.get_operation(), ctx);

    let branch_like = Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![succ0],
        0,
    );
    branch_like.insert_at_back(common_pred, ctx);

    fn assert_block_pred_uses(ctx: &Context, block: Ptr<BasicBlock>) {
        for block_use in block.uses(ctx) {
            assert!(block_use.get_def(ctx) == block);
        }
    }
    // We now have where entry_block branches to common_pred, succ0, succ1, and succ2.
    // So all blocks are reachable, and hence pass the verifier dominance checks.
    verify_op(&module_op, ctx)?;

    assert_eq!(branch_like.deref(ctx).get_num_successors(), 1);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ0);
    assert_block_pred_uses(ctx, succ0);
    assert_eq!(succ0.num_preds(ctx), 2);
    assert_eq!(succ1.num_preds(ctx), 1);
    assert_eq!(succ2.num_preds(ctx), 1);

    let pushed_idx = Operation::push_successor(branch_like, ctx, succ1);
    assert_eq!(pushed_idx, 1);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 2);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ0);
    assert_block_pred_uses(ctx, succ0);
    assert_eq!(branch_like.deref(ctx).get_successor(1), succ1);
    assert_block_pred_uses(ctx, succ1);
    assert_eq!(succ0.num_preds(ctx), 2);
    assert_eq!(succ1.num_preds(ctx), 2);
    let succ0_preds = succ0.preds(ctx);
    assert!(succ0_preds.contains(&common_pred) && succ0_preds.contains(&entry_block));
    let succ1_preds = succ1.preds(ctx);
    assert!(succ1_preds.contains(&common_pred) && succ1_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    let popped = Operation::pop_successor(branch_like, ctx);
    assert_eq!(popped, succ1);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 1);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ0);
    assert_block_pred_uses(ctx, succ0);
    assert_eq!(succ0.num_preds(ctx), 2);
    assert_eq!(succ1.num_preds(ctx), 1);
    let succ0_preds = succ0.preds(ctx);
    assert!(succ0_preds.contains(&common_pred) && succ0_preds.contains(&entry_block));
    let succ1_preds = succ1.preds(ctx);
    assert!(succ1_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    Operation::insert_successor(branch_like, ctx, 0, succ1);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 2);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ1);
    assert_block_pred_uses(ctx, succ1);
    assert_eq!(branch_like.deref(ctx).get_successor(1), succ0);
    assert_block_pred_uses(ctx, succ0);
    assert_eq!(succ0.num_preds(ctx), 2);
    assert_eq!(succ1.num_preds(ctx), 2);
    let succ0_preds = succ0.preds(ctx);
    assert!(succ0_preds.contains(&common_pred) && succ0_preds.contains(&entry_block));
    let succ1_preds = succ1.preds(ctx);
    assert!(succ1_preds.contains(&common_pred) && succ1_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    Operation::insert_successor(branch_like, ctx, 2, succ2);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 3);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ1);
    assert_block_pred_uses(ctx, succ1);
    assert_eq!(branch_like.deref(ctx).get_successor(1), succ0);
    assert_block_pred_uses(ctx, succ0);
    assert_eq!(branch_like.deref(ctx).get_successor(2), succ2);
    assert_block_pred_uses(ctx, succ2);
    assert_eq!(succ0.num_preds(ctx), 2);
    assert_eq!(succ1.num_preds(ctx), 2);
    assert_eq!(succ2.num_preds(ctx), 2);
    let succ2_preds = succ2.preds(ctx);
    assert!(succ2_preds.contains(&common_pred) && succ2_preds.contains(&entry_block));
    let succ1_preds = succ1.preds(ctx);
    assert!(succ1_preds.contains(&common_pred) && succ1_preds.contains(&entry_block));
    let succ0_preds = succ0.preds(ctx);
    assert!(succ0_preds.contains(&common_pred) && succ0_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    let removed_mid = Operation::remove_successor(branch_like, ctx, 1);
    assert_eq!(removed_mid, succ0);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 2);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ1);
    assert_block_pred_uses(ctx, succ1);
    assert_eq!(branch_like.deref(ctx).get_successor(1), succ2);
    assert_block_pred_uses(ctx, succ2);
    assert_eq!(succ0.num_preds(ctx), 1);
    assert_eq!(succ1.num_preds(ctx), 2);
    assert_eq!(succ2.num_preds(ctx), 2);
    let succ2_preds = succ2.preds(ctx);
    assert!(succ2_preds.contains(&common_pred) && succ2_preds.contains(&entry_block));
    let succ1_preds = succ1.preds(ctx);
    assert!(succ1_preds.contains(&common_pred) && succ1_preds.contains(&entry_block));
    let succ0_preds = succ0.preds(ctx);
    assert!(succ0_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    let removed_front = Operation::remove_successor(branch_like, ctx, 0);
    assert_eq!(removed_front, succ1);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 1);
    assert_eq!(branch_like.deref(ctx).get_successor(0), succ2);
    assert_block_pred_uses(ctx, succ2);
    assert_eq!(succ0.num_preds(ctx), 1);
    assert_eq!(succ1.num_preds(ctx), 1);
    assert_eq!(succ2.num_preds(ctx), 2);
    let succ2_preds = succ2.preds(ctx);
    assert!(succ2_preds.contains(&common_pred) && succ2_preds.contains(&entry_block));
    let succ1_preds = succ1.preds(ctx);
    assert!(succ1_preds.contains(&entry_block));
    let succ0_preds = succ0.preds(ctx);
    assert!(succ0_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    // Add another branch to succ2 so that it has 3 preds, with common_pred being occurring twice.
    Operation::insert_successor(branch_like, ctx, 0, succ2);
    assert_eq!(branch_like.deref(ctx).get_num_successors(), 2);
    assert_eq!(succ2.num_preds(ctx), 3);
    let succ2_preds = succ2.preds(ctx);
    assert!(succ2_preds.contains(&common_pred) && succ2_preds.contains(&entry_block));
    verify_op(&module_op, ctx)?;

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
/// A test to just print a constructed IR to stdout.
fn print_simple() -> Result<()> {
    let ctx = &mut Context::new();
    let module_op = const_ret_in_mod(ctx)?.0.get_operation();
    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]
    "#]]
    .assert_eq(&printed);
    println!("{printed}");
    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn parse_simple() -> Result<()> {
    let input = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0():
                c0_op_2_0_res0 = test.constant builtin.integer <0: si64>;
                test.return c0_op_2_0_res0
            ^exit(a : builtin.integer si32):
            }
        }"#;

    let ctx = &mut Context::new();
    let op = {
        let state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        spaced(Operation::top_level_parser())
            .parse(state_stream)
            .unwrap()
            .0
    };
    println!("{}", op.disp(ctx));
    Ok(())
}

dict_key!(ATTR_KEY_TEST_ON_FUNC_VALUE, "test_on_func_value");
dict_key!(ATTR_KEY_BLOCK_TEST, "block_test_attr");

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn parse_function_with_attrs() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _, _const_op, ret_op) = const_ret_in_mod(ctx).unwrap();

    let func_op = ret_op
        .get_operation()
        .deref(ctx)
        .get_parent_op(ctx)
        .unwrap();
    func_op.deref_mut(ctx).attributes.set(
        ATTR_KEY_TEST_ON_FUNC_VALUE.clone(),
        StringAttr::new("func_attr_value".into()),
    );

    let printed = format!("{}", module_op.get_operation().disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
              [test_on_func_value: builtin.string "func_attr_value"]
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]
    "#]]
    .assert_eq(&printed);

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );
    let parsed = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .unwrap()
        .0;

    let print_parsed = format!("{}", parsed.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1_block4v1() !0:
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
              [test_on_func_value: builtin.string "func_attr_value"]
            {
              ^entry_block2v1_block3v1() !1:
                c0_v1 = test.constant builtin.integer <0: si64> !2;
                test.return c0_v1 !3
            } !4
        } !5

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 3], []
        !1 = @[<in-memory>: line: 7, column: 7], []
        !2 = @[<in-memory>: line: 8, column: 9], [builtin_debug_info = builtin.debug_info [c0]]
        !3 = @[<in-memory>: line: 9, column: 9], []
        !4 = @[<in-memory>: line: 4, column: 5], []
        !5 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&print_parsed);

    Ok(())
}

fn expect_parse_error(input: &str, expected_err: Expect) {
    let ctx = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        input.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );
    let actual_err = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .err()
        .unwrap();

    expected_err.assert_eq(&actual_err.to_string());
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn parse_err_multiple_def() {
    let input_multiple_ssa_defs = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0():
                c0_op_2_0_res0 = test.constant builtin.integer <0 : si64>;
                c0_op_2_0_res0 = test.constant builtin.integer <0 : si64>;
                test.return c0_op_2_0_res0
            ^exit():
            }
        }"#;

    let expected_err = expect![[r#"
        Parse error at line: 7, column: 17
        Identifier c0_op_2_0_res0 defined more than once in the scope
    "#]];
    expect_parse_error(input_multiple_ssa_defs, expected_err);

    let input_multiple_label_defs = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0():
                c0_op_2_0_res0 = test.constant builtin.integer <0: si64>;
                test.return c0_op_2_0_res0
            ^entry_block_1_0():
            }
        }"#;

    let expected_err = expect![[r#"
        Parse error at line: 8, column: 13
        Identifier entry_block_1_0 defined more than once in the scope
    "#]];
    expect_parse_error(input_multiple_label_defs, expected_err);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn parse_err_unresolved_def() {
    let input_multiple_defs = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0():
                test.return c0_op_2_0_res0
            }
        }"#;

    let expected_err = expect![[r#"
        Parse error at line: 4, column: 80
        Identifier c0_op_2_0_res0 was not resolved to any definition in the scope
    "#]];
    expect_parse_error(input_multiple_defs, expected_err);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn parse_err_block_label_colon() {
    let input_label_colon_missing = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0():
                c0_op_2_0_res0 = test.constant builtin.integer <0: si64>;
                test.return c0_op_2_0_res0
            ^exit()
            }
        }"#;

    let expected_err = expect![[r#"
        Parse error at line: 9, column: 13
        Unexpected `}`
        Expected whitespaces or `:`
    "#]];

    expect_parse_error(input_label_colon_missing, expected_err);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn parse_err_block_args() {
    let input_label_colon_missing = r#"
        builtin.module @bar {
        ^block_0_0(a : builtin.integer si32, b):
        }"#;

    let expected_err = expect![[r#"
        Parse error at line: 3, column: 47
        Unexpected `)`
        Expected whitespaces or `:`
    "#]];

    expect_parse_error(input_label_colon_missing, expected_err);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_preorder_forward_walk() {
    let ctx = &mut Context::new();
    let module_op = const_ret_in_mod(ctx).unwrap().0.get_operation();

    let mut state = Vec::new();

    walkers::uninterruptible::immutable::walk_op(
        ctx,
        &mut state,
        &WALKCONFIG_PREORDER_FORWARD,
        module_op,
        |_ctx, state, node| {
            if let IRNode::Operation(op) = node {
                state.push(op);
            }
        },
    );

    let ops = state.into_iter().fold("".to_string(), |accum, op| {
        accum + &op.disp(ctx).to_string() + "\n"
    });
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]

        builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
        {
          ^entry_block2v1():
            c0_v0 = test.constant builtin.integer <0: si64> !0;
            test.return c0_v0
        }
        c0_v0 = test.constant builtin.integer <0: si64> !0
        test.return c0_v0
    "#]]
    .assert_eq(&ops);

    Operation::erase(module_op, ctx);
    assert!(ctx.is_ir_empty());
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn walker_print() {
    let ctx = &mut Context::new();
    let module_op = const_ret_in_mod(ctx).unwrap().0.get_operation();

    fn print_op(
        ctx: &Context,
        root: Ptr<Operation>,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        struct State<'a, 'b> {
            f: &'a mut core::fmt::Formatter<'b>,
        }
        let mut state = State { f };
        match walkers::interruptible::immutable::walk_op(
            ctx,
            &mut state,
            &WALKCONFIG_PREORDER_FORWARD,
            root,
            |ctx, state, node| {
                if let IRNode::Operation(op) = node {
                    match writeln!(state.f, "{}", op.disp(ctx)) {
                        Ok(_) => return walk_advance(),
                        Err(e) => return walk_break(e),
                    }
                }
                walk_advance()
            },
        ) {
            interruptible::WalkResult::Continue(_) => Ok(()),
            interruptible::WalkResult::Break(e) => Err(e),
        }
    }

    struct OpPrinter {
        root: Ptr<Operation>,
    }
    impl Printable for OpPrinter {
        fn fmt(
            &self,
            ctx: &Context,
            _state: &pliron::printable::State,
            f: &mut core::fmt::Formatter<'_>,
        ) -> core::fmt::Result {
            print_op(ctx, self.root, f)
        }
    }
    let op_printer = OpPrinter { root: module_op };
    let printed = format!("{}", op_printer.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]

        builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
        {
          ^entry_block2v1():
            c0_v0 = test.constant builtin.integer <0: si64> !0;
            test.return c0_v0
        }
        c0_v0 = test.constant builtin.integer <0: si64> !0
        test.return c0_v0
    "#]]
    .assert_eq(&printed);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_postorder_forward_walk() {
    let ctx = &mut Context::new();
    let module_op = const_ret_in_mod(ctx).unwrap().0.get_operation();

    let mut state = Vec::new();

    walkers::uninterruptible::immutable::walk_op(
        ctx,
        &mut state,
        &WALKCONFIG_POSTORDER_FORWARD,
        module_op,
        |_ctx, state, node| {
            if let IRNode::Operation(op) = node {
                state.push(op);
            }
        },
    );

    let ops = state.into_iter().fold("".to_string(), |accum, op| {
        accum + &op.disp(ctx).to_string() + "\n"
    });
    expect![[r#"
        c0_v0 = test.constant builtin.integer <0: si64> !0
        test.return c0_v0
        builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
        {
          ^entry_block2v1():
            c0_v0 = test.constant builtin.integer <0: si64> !0;
            test.return c0_v0
        }
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]

    "#]]
    .assert_eq(&ops);
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_walker_find_op() {
    let ctx = &mut Context::new();
    let (module_op, _, const_op, _) = const_ret_in_mod(ctx).unwrap();

    let const1_op = ConstantOp::new(ctx, 1);
    const1_op
        .get_operation()
        .insert_after(ctx, const_op.get_operation());
    set_operation_result_name(
        ctx,
        const1_op.get_operation(),
        0,
        Some("c1".try_into().unwrap()),
    );

    // A function to breaks the walk when a [ConstantOp] is found.
    fn finder(ctx: &Context, _: &mut (), node: IRNode) -> interruptible::WalkResult<ConstantOp> {
        if let IRNode::Operation(op) = node
            && let Some(const_op) = Operation::get_op::<ConstantOp>(op, ctx)
        {
            return walk_break(const_op);
        }
        walk_advance()
    }

    let res1 = walkers::interruptible::immutable::walk_op(
        ctx,
        &mut (),
        &WALKCONFIG_PREORDER_FORWARD,
        module_op.get_operation(),
        finder,
    );
    assert!(matches!(res1, interruptible::WalkResult::Break(c) if c == const_op));

    let res2 = walkers::interruptible::immutable::walk_op(
        ctx,
        &mut (),
        &WALKCONFIG_POSTORDER_REVERSE,
        module_op.get_operation(),
        finder,
    );
    assert!(matches!(res2, interruptible::WalkResult::Break(c) if c == const1_op));
}

#[test]
fn test_verify_missing_terminator() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _, _, ret_op) = const_ret_in_mod(ctx)?;

    verify_op(&module_op, ctx)?;

    Operation::erase(ret_op.get_operation(), ctx);

    let verify_res = verify_op(&module_op, ctx);
    let err = verify_res.unwrap_err();
    let err = err.err.downcast_ref::<BasicBlockVerifyErr>().unwrap();
    assert!(matches!(err, BasicBlockVerifyErr::MissingTerminator(_)));

    Ok(())
}

#[test]
fn test_verify_multiple_terminator() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _, const_op, ret_op) = const_ret_in_mod(ctx)?;

    verify_op(&module_op, ctx)?;

    let ret2_op = ReturnOp::new(ctx, const_op.get_result(ctx));
    ret2_op
        .get_operation()
        .insert_before(ctx, ret_op.get_operation());

    let verify_res = verify_op(&module_op, ctx);
    let err = verify_res.unwrap_err();
    let err = err.err.downcast_ref::<BasicBlockVerifyErr>().unwrap();
    assert!(matches!(err, BasicBlockVerifyErr::TerminatorNotLast(_)));

    Ok(())
}

#[pliron_op(
    name = "test.branch",
    format,
    interfaces = [IsTerminatorInterface],
    verifier = "succ",
)]
struct BranchOp {}

#[test]
fn test_verify_operation_fails_on_undominated_op_result_use() {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let func_ty = pliron::builtin::types::FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let module = pliron::builtin::ops::ModuleOp::new(ctx, "bar".try_into().unwrap());
    let func = pliron::builtin::ops::FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);

    let entry = func.get_entry_block(ctx);
    let func_region = func.get_region(ctx);
    let b1 = BasicBlock::new(ctx, None, vec![]);
    b1.insert_at_back(func_region, ctx);
    let b2 = BasicBlock::new(ctx, None, vec![]);
    b2.insert_at_back(func_region, ctx);

    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![b1, b2],
        0,
    )
    .insert_at_back(entry, ctx);

    let def_op = ConstantOp::new(ctx, 0);
    def_op.get_operation().insert_at_back(b1, ctx);
    ReturnOp::new(ctx, def_op.get_result(ctx))
        .get_operation()
        .insert_at_back(b1, ctx);

    ReturnOp::new(ctx, def_op.get_result(ctx))
        .get_operation()
        .insert_at_back(b2, ctx);

    println!("{}", module.get_operation().disp(ctx));

    let err = verify_operation(module.get_operation(), ctx).unwrap_err();
    let err = err.err.downcast_ref::<DefUseVerifyErr>().unwrap();
    assert!(matches!(err, DefUseVerifyErr::UseNotDominatedByDef(_)));
}

#[test]
fn test_verify_operation_fails_on_undominated_block_argument_use() {
    let ctx = &mut Context::new();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let func_ty = pliron::builtin::types::FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let module = pliron::builtin::ops::ModuleOp::new(ctx, "bar".try_into().unwrap());
    let func = pliron::builtin::ops::FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);

    let entry = func.get_entry_block(ctx);
    let func_region = func.get_region(ctx);
    let b1 = BasicBlock::new(ctx, None, vec![i64_ty.into()]);
    b1.insert_at_back(func_region, ctx);
    let b2 = BasicBlock::new(ctx, None, vec![]);
    b2.insert_at_back(func_region, ctx);

    Operation::new(
        ctx,
        BranchOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![b1, b2],
        0,
    )
    .insert_at_back(entry, ctx);

    let b1_arg0 = b1.deref(ctx).get_argument(0);

    ReturnOp::new(ctx, b1_arg0)
        .get_operation()
        .insert_at_back(b1, ctx);
    ReturnOp::new(ctx, b1_arg0)
        .get_operation()
        .insert_at_back(b2, ctx);

    println!("{}", module.get_operation().disp(ctx));

    let err = verify_operation(module.get_operation(), ctx).unwrap_err();
    let err = err.err.downcast_ref::<DefUseVerifyErr>().unwrap();
    assert!(matches!(err, DefUseVerifyErr::UseNotDominatedByDef(_)));
}

#[test]
fn block_inline_attrs_print() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, func_op, _const_op, _ret_op) = const_ret_in_mod(ctx)?;

    let entry_block = func_op.get_entry_block(ctx);

    // Set an inline attribute on the block
    entry_block.deref_mut(ctx).attributes.set(
        ATTR_KEY_BLOCK_TEST.clone(),
        StringAttr::new("test_value".into()),
    );

    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1() [block_test_attr: builtin.string "test_value"]:
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }"#]]
    .assert_eq(&printed);

    Ok(())
}

#[test]
fn block_inline_attrs_roundtrip() -> Result<()> {
    let input = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0() [block_attr: builtin.string "hello"]:
                c0_op_2_0_res0 = test.constant builtin.integer <0: si64>;
                test.return c0_op_2_0_res0
            }
        }"#;

    let ctx = &mut Context::new();
    let op = {
        let state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        spaced(Operation::top_level_parser())
            .parse(state_stream)
            .unwrap()
            .0
    };

    verify_operation(op, ctx)?;

    let printed = format!("{}", op.disp(ctx));

    // Parse again and print to confirm roundtrip
    let ctx2 = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx2, location::Source::InMemory),
    );
    let parsed2 = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .unwrap()
        .0;

    verify_operation(parsed2, ctx2)?;

    Ok(())
}

#[test]
fn block_multiple_inline_attrs() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, func_op, _const_op, _ret_op) = const_ret_in_mod(ctx)?;

    // Get the function's entry block and set attributes on it
    let func_entry_block = func_op.get_entry_block(ctx);

    // Set multiple inline attributes on the function's entry block
    func_entry_block.deref_mut(ctx).attributes.set(
        "attr1".try_into().unwrap(),
        StringAttr::new("value1".into()),
    );
    func_entry_block.deref_mut(ctx).attributes.set(
        "attr2".try_into().unwrap(),
        StringAttr::new("value2".into()),
    );

    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1() [attr1: builtin.string "value1", attr2: builtin.string "value2"]:
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0
            }
        }"#]]
    .assert_eq(&printed);

    Ok(())
}

#[test]
fn block_attrs_parse_roundtrip() -> Result<()> {
    // Test that block inline attributes survive a parse-print-parse roundtrip.
    let input = r#"
        builtin.module @bar {
        ^block_0_0():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> {
            ^entry_block_1_0() [block_attr: builtin.string "hello"]:
                c0_op_2_0_res0 = test.constant builtin.integer <0: si64>;
                test.return c0_op_2_0_res0
            }
        }"#;

    let ctx = &mut Context::new();
    let op = {
        let state_stream = state_stream_from_iterator(
            input.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        spaced(Operation::top_level_parser())
            .parse(state_stream)
            .unwrap()
            .0
    };

    verify_operation(op, ctx)?;

    let printed = format!("{}", op.deref(ctx).disp(ctx));

    // After parsing from source, blocks get locations. Second print has outline indices.
    // The outlined_attributes section only has locations; inline (non-outlined) attrs stay inline.
    expect![[r#"
        builtin.module @bar 
        {
          ^block_0_0_block2v1() !0:
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block_1_0_block1v1() [block_attr: builtin.string "hello"] !1:
                c0_op_2_0_res0_v0 = test.constant builtin.integer <0: si64> !2;
                test.return c0_op_2_0_res0_v0 !3
            } !4
        } !5

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 9], []
        !1 = @[<in-memory>: line: 5, column: 13], []
        !2 = @[<in-memory>: line: 6, column: 17], [builtin_debug_info = builtin.debug_info [c0_op_2_0_res0]]
        !3 = @[<in-memory>: line: 7, column: 17], []
        !4 = @[<in-memory>: line: 4, column: 13], []
        !5 = @[<in-memory>: line: 2, column: 9], []
    "#]]
    .assert_eq(&printed);

    // Parse again and verify it's still valid
    let ctx2 = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx2, location::Source::InMemory),
    );
    let parsed2 = spaced(Operation::top_level_parser())
        .parse(state_stream)
        .unwrap()
        .0;

    verify_operation(parsed2, ctx2)?;

    // The third print should still have the attributes
    let print3 = format!("{}", parsed2.deref(ctx2).disp(ctx2));
    expect![[r#"
        builtin.module @bar 
        {
          ^block_0_0_block2v1_block2v1() !0:
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block_1_0_block1v1_block1v1() [block_attr: builtin.string "hello"] !1:
                c0_op_2_0_res0_v0 = test.constant builtin.integer <0: si64> !2;
                test.return c0_op_2_0_res0_v0 !3
            } !4
        } !5

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 9], []
        !1 = @[<in-memory>: line: 5, column: 13], []
        !2 = @[<in-memory>: line: 6, column: 17], [builtin_debug_info = builtin.debug_info [c0_op_2_0_res0]]
        !3 = @[<in-memory>: line: 7, column: 17], []
        !4 = @[<in-memory>: line: 4, column: 13], []
        !5 = @[<in-memory>: line: 2, column: 9], []
    "#]]
    .assert_eq(&print3);

    Ok(())
}
