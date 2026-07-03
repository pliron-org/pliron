use crate::common::{ReturnOp, const_ret_in_mod};
use common::ConstantOp;
use core::panic;
use expect_test::expect;
use pliron::{
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::{
            IsolatedFromAboveInterface, NOpdsInterface, NResultsInterface, OneOpdInterface,
            OneRegionInterface, OneResultInterface, SingleBlockRegionInterface, SymbolOpInterface,
        },
        ops::FuncOp,
        types::{IntegerType, Signedness},
    },
    context::{Context, Ptr},
    graph::walkers::{
        WALKCONFIG_POSTORDER_REVERSE, WALKCONFIG_PREORDER_FORWARD, WALKCONFIG_PREORDER_REVERSE,
    },
    identifier::Identifier,
    irbuild::{
        inserter::{BlockInsertionPoint, Inserter, OpInsertionPoint},
        match_rewrite::{
            EnqueueOrder, MatchRewrite, MatchRewriter, RewriterOrder, apply_match_rewrite,
        },
        rewriter::{Rewriter, ScopedRewriter},
    },
    op::{Op, verify_op},
    operation::Operation,
    printable::Printable,
    result::Result,
    r#type::TypeHandle,
    value::Value,
};

use pliron::derive::{pliron_op, pliron_type};
#[cfg(target_family = "wasm")]
use wasm_bindgen_test::*;

mod common;

fn assert_rewrite_sequence_for_order(
    collect: pliron::graph::walkers::WalkConfig,
    enque: EnqueueOrder,
    expected_sequence: &[u64],
) -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _func_op, const0_op, ret_op) = const_ret_in_mod(ctx).unwrap();

    // Add one more constant so initial collect order differences are observable.
    let extra_const_op = ConstantOp::new(ctx, 1).get_operation();
    extra_const_op.insert_before(ctx, ret_op.get_operation());

    // Keep the original constant alive so replacing it remains meaningful.
    assert!(const0_op.get_result(ctx).num_uses(ctx) > 0);

    struct RecordAndBumpUntil3 {
        seen: Vec<u64>,
    }

    impl MatchRewrite for RecordAndBumpUntil3 {
        fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if let Some(const_op) = Operation::get_op::<ConstantOp>(op, ctx) {
                return const_op
                    .get_value(ctx)
                    .downcast_ref::<IntegerAttr>()
                    .is_some_and(|int_attr| int_attr.value().to_u64() < 3);
            }
            false
        }

        fn rewrite(
            &mut self,
            ctx: &mut Context,
            rewriter: &mut MatchRewriter,
            op: Ptr<Operation>,
        ) -> Result<()> {
            let value_attr = Operation::get_op::<ConstantOp>(op, ctx)
                .unwrap()
                .get_value(ctx);
            let Some(int_attr) = value_attr.downcast_ref::<IntegerAttr>() else {
                panic!("Expected ConstantOp to hold IntegerAttr");
            };

            let cur_val = int_attr.value().to_u64();
            self.seen.push(cur_val);

            let next_const = ConstantOp::new(ctx, cur_val + 1).get_operation();
            rewriter.insert_operation(ctx, next_const);
            rewriter.replace_operation(ctx, op, next_const);
            Ok(())
        }
    }

    let mut matcher = RecordAndBumpUntil3 { seen: Vec::new() };
    apply_match_rewrite(
        ctx,
        &mut matcher,
        RewriterOrder { collect, enque },
        module_op.get_operation(),
    )?;
    verify_op(&module_op, ctx)?;

    assert_eq!(&matcher.seen, expected_sequence);
    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn apply_match_rewrite_preorder_forward_enque_front_ordering() -> Result<()> {
    assert_rewrite_sequence_for_order(
        WALKCONFIG_PREORDER_FORWARD,
        EnqueueOrder::EnqueFront,
        &[0, 1, 2, 1, 2],
    )
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn apply_match_rewrite_preorder_reverse_enque_back_ordering() -> Result<()> {
    assert_rewrite_sequence_for_order(
        WALKCONFIG_PREORDER_REVERSE,
        EnqueueOrder::EnqueBack,
        &[1, 0, 2, 1, 2],
    )
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn apply_match_rewrite_postorder_reverse_enque_front_ordering() -> Result<()> {
    assert_rewrite_sequence_for_order(
        WALKCONFIG_POSTORDER_REVERSE,
        EnqueueOrder::EnqueFront,
        &[1, 2, 0, 1, 2],
    )
}

// Testing replacing all uses of c0 with c1.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn replace_c0_with_c1() -> Result<()> {
    let ctx = &mut Context::new();

    // const_ret_in_mod builds a module with a function.
    let (module_op, _, const_op, _) = const_ret_in_mod(ctx).unwrap();
    assert!(
        const_op
            .get_value(ctx)
            .downcast_ref::<IntegerAttr>()
            .unwrap()
            .value()
            .to_u64()
            == 0
    );

    struct ReplaceC0WithC1;

    impl MatchRewrite for ReplaceC0WithC1 {
        fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if let Some(const_op) = Operation::get_op::<ConstantOp>(op, ctx) {
                let val = const_op.get_value(ctx);
                return val
                    .downcast_ref::<IntegerAttr>()
                    .is_some_and(|int_attr| int_attr.value().to_u64() < 2);
            }
            false
        }

        fn rewrite(
            &mut self,
            ctx: &mut Context,
            rewriter: &mut MatchRewriter,
            op: Ptr<Operation>,
        ) -> Result<()> {
            let Some(const_op) =
                Operation::get_op::<ConstantOp>(op, ctx).map(|co| co.get_value(ctx))
            else {
                panic!("Expected ConstantOp");
            };
            let cur_val = const_op
                .downcast_ref::<IntegerAttr>()
                .unwrap()
                .value()
                .to_u64();
            if cur_val >= 2 {
                panic!("Expected match only on constant value less than 2");
            }
            let const1_op = ConstantOp::new(ctx, cur_val + 1).get_operation();
            rewriter.insert_operation(ctx, const1_op);
            rewriter.replace_operation(ctx, op, const1_op);
            Ok(())
        }
    }

    // Collect and rewrite must replace constant 0 with constant 1,
    // and then constant 1 with constant 2.
    let mut matcher = ReplaceC0WithC1;
    apply_match_rewrite(
        ctx,
        &mut matcher,
        RewriterOrder {
            collect: WALKCONFIG_PREORDER_FORWARD,
            enque: EnqueueOrder::EnqueBack,
        },
        module_op.get_operation(),
    )?;
    verify_op(&module_op, ctx)?;

    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> 
            {
              ^entry_block2v1():
                v2 = test.constant builtin.integer <2: si64>;
                test.return v2
            }
        }"#]]
    .assert_eq(&printed);

    Ok(())
}

/// A test global operation.
#[pliron_op(
    name = "test.global",
    format,
    interfaces = [
        IsolatedFromAboveInterface,
        NOpdsInterface<0>,
        NResultsInterface<1>,
        OneResultInterface,
        SymbolOpInterface,
        SingleBlockRegionInterface,
    ],
    attributes = (test_global_op_const_val: IntegerAttr),
    verifier = "succ",
)]
pub struct GlobalOp;

impl GlobalOp {
    /// Create a new [GlobalOp].
    pub fn new(ctx: &mut Context, name: Identifier, val: IntegerAttr) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![PointerType::get(ctx).into()],
            vec![],
            vec![],
            0,
        );
        let op = GlobalOp { op };
        op.set_symbol_name(ctx, name);
        op.set_attr_test_global_op_const_val(ctx, val);
        op
    }
}

/// An opaque pointer
#[pliron_type(name = "test.ptr", format, generate_get = true, verifier = "succ")]
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct PointerType;

#[pliron_op(
    name = "test.load",
    format = "$0 ` ` : ` type($0)",
    interfaces = [OneResultInterface, OneOpdInterface, NResultsInterface<1>, NOpdsInterface<1>],
    verifier = "succ",
)]
pub struct LoadOp;

impl LoadOp {
    /// Create a new [LoadOp]
    pub fn new(ctx: &mut Context, ptr: Value, res_ty: TypeHandle) -> Self {
        LoadOp {
            op: Operation::new(
                ctx,
                Self::get_concrete_op_info(),
                vec![res_ty],
                vec![ptr],
                vec![],
                0,
            ),
        }
    }
}

/// Test scoped rewriter that sets the insertion point for the duration of its lifetime.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn scoped_rewriter_test() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _func_op, _, _) = const_ret_in_mod(ctx).unwrap();

    // For each constant operation, create a global in the parent module and load from that.
    struct ConstToGlobal;
    impl MatchRewrite for ConstToGlobal {
        fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
            Operation::get_op::<ConstantOp>(op, ctx).is_some()
        }

        fn rewrite(
            &mut self,
            ctx: &mut Context,
            rewriter: &mut MatchRewriter,
            op: Ptr<Operation>,
        ) -> Result<()> {
            let const_op = Operation::get_op::<ConstantOp>(op, ctx).unwrap();
            let val = const_op.get_value(ctx).downcast::<IntegerAttr>().unwrap();

            // Create a global in the module.
            let func_op = op.deref(ctx).get_parent_op(ctx).unwrap();
            let module_op = func_op.deref(ctx).get_parent_op(ctx).unwrap();
            let module_op =
                Operation::get_op::<pliron::builtin::ops::ModuleOp>(module_op, ctx).unwrap();

            let name =
                Identifier::try_from("global_".to_string() + &val.value().to_string(10, true))
                    .unwrap();
            let global_op = GlobalOp::new(ctx, name, *val);
            {
                let mut module_inserter = ScopedRewriter::new(
                    rewriter,
                    OpInsertionPoint::AtBlockStart(module_op.get_body(ctx, 0)),
                );
                module_inserter.insert_operation(ctx, global_op.get_operation());
            }

            let int_ty = IntegerType::get(ctx, 32, Signedness::Signed).into();
            let load_op = LoadOp::new(ctx, global_op.get_result(ctx), int_ty).get_operation();
            rewriter.insert_operation(ctx, load_op);

            // Replace uses of the constant with the load.
            rewriter.replace_operation(ctx, op, load_op);
            Ok(())
        }
    }

    let mut matcher = ConstToGlobal;
    apply_match_rewrite(
        ctx,
        &mut matcher,
        RewriterOrder {
            collect: WALKCONFIG_PREORDER_FORWARD,
            enque: EnqueueOrder::EnqueBack,
        },
        module_op.get_operation(),
    )?;
    verify_op(&module_op, ctx)?;

    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            v1 = test.global () [] [test_global_op_const_val: builtin.integer <0: si64>, sym_name: builtin.identifier global_0]: <() -> (test.ptr )>;
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> 
            {
              ^entry_block2v1():
                v2 = test.load v1 ;
                test.return v2
            }
        }"#]].assert_eq(&printed);

    Ok(())
}

/// Test that erases a function containing a constant 0 operation.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn erase_func_with_const_zero() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _func_op, _, _) = const_ret_in_mod(ctx).unwrap();

    struct EraseFunc;
    impl MatchRewrite for EraseFunc {
        fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if let Some(const_op) = Operation::get_op::<ConstantOp>(op, ctx) {
                let val = const_op.get_value(ctx);
                return val
                    .downcast_ref::<IntegerAttr>()
                    .is_some_and(|int_attr| int_attr.value().to_u64() == 0);
            }
            false
        }

        fn rewrite(
            &mut self,
            ctx: &mut Context,
            rewriter: &mut MatchRewriter,
            op: Ptr<Operation>,
        ) -> Result<()> {
            // Insert a constant 1 operation before erasing the function.
            let const1_op = ConstantOp::new(ctx, 1).get_operation();
            rewriter.insert_operation(ctx, const1_op);
            // Insert a constant 0 operation before erasing the function.
            let const0_op = ConstantOp::new(ctx, 0).get_operation();
            rewriter.insert_operation(ctx, const0_op);

            let func_op = op.deref(ctx).get_parent_op(ctx).unwrap();
            rewriter.erase_operation(ctx, func_op);
            Ok(())
        }
    }

    let mut matcher = EraseFunc;
    apply_match_rewrite(
        ctx,
        &mut matcher,
        RewriterOrder {
            collect: WALKCONFIG_PREORDER_FORWARD,
            enque: EnqueueOrder::EnqueBack,
        },
        module_op.get_operation(),
    )?;
    verify_op(&module_op, ctx)?;

    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            
        }"#]]
    .assert_eq(&printed);

    Ok(())
}

/// Test that splits a block after finding a constant 0 operation.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn split_block_after_const_zero() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op, _func_op, _, _) = const_ret_in_mod(ctx).unwrap();

    struct SplitBlockAfterConstZero;
    impl MatchRewrite for SplitBlockAfterConstZero {
        fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if let Some(const_op) = Operation::get_op::<ConstantOp>(op, ctx) {
                let val = const_op.get_value(ctx);
                return val
                    .downcast_ref::<IntegerAttr>()
                    .is_some_and(|int_attr| int_attr.value().to_u64() == 0);
            }
            false
        }

        fn rewrite(
            &mut self,
            ctx: &mut Context,
            rewriter: &mut MatchRewriter,
            op: Ptr<Operation>,
        ) -> Result<()> {
            let const0_op = Operation::get_op::<ConstantOp>(op, ctx).unwrap();
            let block = op.deref(ctx).get_parent_block().unwrap();

            // Split the block after the constant 0 operation
            let new_block =
                rewriter.split_block(ctx, block, OpInsertionPoint::AfterOperation(op), None);

            // Insert constant 1 in the new block at the start
            let const1_op = ConstantOp::new(ctx, 1).get_operation();
            let const1_result = const1_op.deref(ctx).get_result(0);
            rewriter.set_insertion_point(OpInsertionPoint::AtBlockStart(new_block));
            rewriter.insert_operation(ctx, const1_op);
            // Modify the return operation in the new block to return constant 1
            const0_op
                .get_result(ctx)
                .replace_all_uses_with(ctx, &const1_result);

            // Add a return at the end of old block to return the const0_op's result
            let ret = ReturnOp::new(ctx, const0_op.get_result(ctx)).get_operation();
            {
                ScopedRewriter::new(rewriter, OpInsertionPoint::AtBlockEnd(block))
                    .insert_operation(ctx, ret);
            }

            Ok(())
        }
    }

    let mut matcher = SplitBlockAfterConstZero;
    apply_match_rewrite(
        ctx,
        &mut matcher,
        RewriterOrder {
            collect: WALKCONFIG_PREORDER_FORWARD,
            enque: EnqueueOrder::EnqueBack,
        },
        module_op.get_operation(),
    )?;
    verify_op(&module_op, ctx)?;

    let printed = format!("{}", module_op.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0

              ^entry_split_block3v1():
                v1 = test.constant builtin.integer <1: si64>;
                test.return v1
            }
        }"#]]
    .assert_eq(&printed);

    Ok(())
}

/// Test that inlines a region from one function into another.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn inline_region_on_const_zero() -> Result<()> {
    let ctx = &mut Context::new();
    let (module_op1, func_op1, _, _) = const_ret_in_mod(ctx).unwrap();
    let (module_op2, _func_op2, _, _) = const_ret_in_mod(ctx).unwrap();

    struct InlineRegionOnConstZero(FuncOp);
    impl MatchRewrite for InlineRegionOnConstZero {
        fn r#match(&mut self, ctx: &Context, op: Ptr<Operation>) -> bool {
            if let Some(const_op) = Operation::get_op::<ConstantOp>(op, ctx) {
                let val = const_op.get_value(ctx);
                return val
                    .downcast_ref::<IntegerAttr>()
                    .is_some_and(|int_attr| int_attr.value().to_u64() == 0);
            }
            false
        }

        fn rewrite(
            &mut self,
            ctx: &mut Context,
            rewriter: &mut MatchRewriter,
            op: Ptr<Operation>,
        ) -> Result<()> {
            let func_op = op.deref(ctx).get_parent_op(ctx).unwrap();
            // Get the region from the matched function
            let region = func_op.deref(ctx).get_region(0);

            // Inline the region at the end of the second function's block
            rewriter.inline_region(
                ctx,
                region,
                BlockInsertionPoint::AtRegionEnd(self.0.get_region(ctx)),
            );

            Ok(())
        }
    }

    let mut matcher = InlineRegionOnConstZero(func_op1);
    apply_match_rewrite(
        ctx,
        &mut matcher,
        RewriterOrder {
            collect: WALKCONFIG_PREORDER_FORWARD,
            enque: EnqueueOrder::EnqueBack,
        },
        module_op2.get_operation(),
    )?;
    verify_op(&module_op1, ctx)?;
    verify_op(&module_op2, ctx)?;

    let printed = format!("{}", module_op1.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v0 = test.constant builtin.integer <0: si64> !0;
                test.return c0_v0

              ^entry_block4v1():
                c0_v1 = test.constant builtin.integer <0: si64> !1;
                test.return c0_v1
            }
        }"#]]
    .assert_eq(&printed);

    let printed = format!("{}", module_op2.disp(ctx));
    expect![[r#"
        builtin.module @bar 
        {
          ^block3v1():
            builtin.func @foo: builtin.function <() -> (builtin.integer si64)> 
            {
              
            }
        }"#]]
    .assert_eq(&printed);

    Ok(())
}
