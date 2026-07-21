// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use pliron::{
    basic_block::BasicBlock,
    builtin::{
        op_interfaces::{
            IsTerminatorInterface, NOpdsInterface, NResultsInterface, OneOpdInterface,
            OneResultInterface, SingleBlockRegionInterface,
        },
        ops::ModuleOp,
        types::{IntegerType, Signedness},
    },
    context::{Context, Ptr},
    identifier::Identifier,
    init_env_logger_for_tests,
    irbuild::{
        dialect_conversion::{
            DialectConversion, DialectConversionRewriter, OperandsInfo, apply_dialect_conversion,
        },
        inserter::Inserter,
        rewriter::Rewriter,
    },
    op::Op,
    operation::Operation,
    result::Result,
    r#type::{TypeHandle, Typed},
};

use pliron::derive::pliron_op;
#[cfg(target_family = "wasm")]
use wasm_bindgen_test::*;

#[pliron_op(
    name = "test.producer",
    format,
    interfaces = [NOpdsInterface<0>, OneResultInterface],
    verifier = "succ",
)]
pub struct ProducerOp;

impl ProducerOp {
    fn new(ctx: &mut Context, width: u32) -> Self {
        let ty = IntegerType::get(ctx, width, Signedness::Signed).into();
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![ty],
            vec![],
            vec![],
            0,
        );
        Self { op }
    }
}

#[pliron_op(
    name = "test.consumer",
    format,
    interfaces = [OneOpdInterface, NResultsInterface<0>],
    verifier = "succ",
)]
pub struct ConsumerOp;

impl ConsumerOp {
    fn new(ctx: &mut Context, value: pliron::value::Value) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![value],
            vec![],
            0,
        );
        Self { op }
    }
}

#[pliron_op(
    name = "test.forward_to_succ",
    format,
    interfaces = [
        IsTerminatorInterface,
        OneOpdInterface,
        NResultsInterface<0>
    ],
    verifier = "succ",
)]
pub struct ForwardToSuccOp;

impl ForwardToSuccOp {
    fn new(ctx: &mut Context, value: pliron::value::Value, succ: Ptr<BasicBlock>) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![value],
            vec![succ],
            0,
        );
        Self { op }
    }
}

#[derive(Default)]
struct WidthConversion {
    saw_consumer: bool,
}

impl DialectConversion for WidthConversion {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        if let Some(producer) = Operation::get_op::<ProducerOp>(op, ctx) {
            let ty = producer.get_result(ctx).get_type(ctx);
            let width = ty
                .deref(ctx)
                .downcast_ref::<IntegerType>()
                .expect("expected integer type")
                .width();
            return width > 16;
        }
        Operation::get_op::<ConsumerOp>(op, ctx).is_some()
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()> {
        if let Some(producer) = Operation::get_op::<ProducerOp>(op, ctx) {
            let current_ty = producer.get_result(ctx).get_type(ctx);
            let current_width = current_ty
                .deref(ctx)
                .downcast_ref::<IntegerType>()
                .expect("expected integer type")
                .width();
            let converted = ProducerOp::new(ctx, current_width / 2).get_operation();
            rewriter.insert_operation(ctx, converted);
            rewriter.replace_operation(ctx, op, converted);
            return Ok(());
        }

        if let Some(consumer) = Operation::get_op::<ConsumerOp>(op, ctx) {
            let operand = consumer.get_operand(ctx);
            let final_width = operand
                .get_type(ctx)
                .deref(ctx)
                .downcast_ref::<IntegerType>()
                .expect("expected integer type")
                .width();
            assert_eq!(final_width, 16);

            let operand_type_history = operands_info.lookup_operand_history(operand);
            assert_eq!(operand_type_history.len(), 2);
            let previous_widths = operand_type_history
                .iter()
                .map(|ty| {
                    ty.deref(ctx)
                        .downcast_ref::<IntegerType>()
                        .expect("expected integer type")
                        .width()
                })
                .collect::<Vec<u32>>();
            assert_eq!(previous_widths, vec![64, 32]);

            self.saw_consumer = true;
        }

        Ok(())
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn dialect_conversion_defs_before_uses() -> Result<()> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();

    let module = ModuleOp::new(
        ctx,
        Identifier::try_from("dialect_conversion_test").unwrap(),
    );
    let body = module.get_body(ctx, 0);

    let producer = ProducerOp::new(ctx, 64);
    producer.get_operation().insert_at_back(body, ctx);
    let consumer = ConsumerOp::new(ctx, producer.get_result(ctx));
    consumer.get_operation().insert_at_back(body, ctx);

    let mut conversion = WidthConversion::default();
    apply_dialect_conversion(ctx, &mut conversion, module.get_operation())?;
    assert!(conversion.saw_consumer);

    Ok(())
}

#[derive(Default)]
struct WidthConversionViaValueReplacement {
    saw_consumer: bool,
}

impl DialectConversion for WidthConversionViaValueReplacement {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        if let Some(producer) = Operation::get_op::<ProducerOp>(op, ctx) {
            let ty = producer.get_result(ctx).get_type(ctx);
            let width = ty
                .deref(ctx)
                .downcast_ref::<IntegerType>()
                .expect("expected integer type")
                .width();
            return width > 16;
        }
        Operation::get_op::<ConsumerOp>(op, ctx).is_some()
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()> {
        if let Some(producer) = Operation::get_op::<ProducerOp>(op, ctx) {
            let current_ty = producer.get_result(ctx).get_type(ctx);
            let current_width = current_ty
                .deref(ctx)
                .downcast_ref::<IntegerType>()
                .expect("expected integer type")
                .width();
            let converted = ProducerOp::new(ctx, current_width / 2);
            rewriter.insert_operation(ctx, converted.get_operation());
            rewriter.replace_value_uses_with(
                ctx,
                producer.get_result(ctx),
                converted.get_result(ctx),
            );
            rewriter.erase_operation(ctx, op);
            return Ok(());
        }

        if let Some(consumer) = Operation::get_op::<ConsumerOp>(op, ctx) {
            let operand = consumer.get_operand(ctx);
            let final_width = operand
                .get_type(ctx)
                .deref(ctx)
                .downcast_ref::<IntegerType>()
                .expect("expected integer type")
                .width();
            assert_eq!(final_width, 16);

            let operand_type_history = operands_info.lookup_operand_history(operand);
            assert_eq!(operand_type_history.len(), 2);
            let previous_widths = operand_type_history
                .iter()
                .map(|ty| {
                    ty.deref(ctx)
                        .downcast_ref::<IntegerType>()
                        .expect("expected integer type")
                        .width()
                })
                .collect::<Vec<u32>>();
            assert_eq!(previous_widths, vec![64, 32]);

            self.saw_consumer = true;
        }

        Ok(())
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn dialect_conversion_value_replacement_preserves_type_history() -> Result<()> {
    let ctx = &mut Context::new();

    let module = ModuleOp::new(
        ctx,
        Identifier::try_from("dialect_conversion_value_replacement_test").unwrap(),
    );
    let body = module.get_body(ctx, 0);

    let producer = ProducerOp::new(ctx, 64);
    producer.get_operation().insert_at_back(body, ctx);
    let consumer = ConsumerOp::new(ctx, producer.get_result(ctx));
    consumer.get_operation().insert_at_back(body, ctx);

    let mut conversion = WidthConversionViaValueReplacement::default();
    apply_dialect_conversion(ctx, &mut conversion, module.get_operation())?;
    assert!(conversion.saw_consumer);

    Ok(())
}

#[derive(Default)]
struct ConsumerOnlyConversion {
    saw_consumer: bool,
}

impl DialectConversion for ConsumerOnlyConversion {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        Operation::get_op::<ConsumerOp>(op, ctx).is_some()
    }

    fn can_convert_type(&self, ctx: &Context, ty: TypeHandle) -> bool {
        ty.deref(ctx)
            .downcast_ref::<IntegerType>()
            .is_some_and(|int_ty| int_ty.width() > 16)
    }

    fn convert_type(&mut self, ctx: &mut Context, ty: TypeHandle) -> Result<TypeHandle> {
        let width = ty
            .deref(ctx)
            .downcast_ref::<IntegerType>()
            .expect("expected integer type")
            .width();
        Ok(IntegerType::get(ctx, width / 2, Signedness::Signed).into())
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        _rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        operands_info: &OperandsInfo,
    ) -> Result<()> {
        let consumer = Operation::get_op::<ConsumerOp>(op, ctx).expect("expected consumer op");
        let operand = consumer.get_operand(ctx);

        let final_width = operand
            .get_type(ctx)
            .deref(ctx)
            .downcast_ref::<IntegerType>()
            .expect("expected integer type")
            .width();
        assert_eq!(final_width, 16);

        let previous_width = operands_info
            .lookup_most_recent_of_type::<IntegerType>(ctx, operand)
            .expect("Previous integer type missing");
        assert_eq!(previous_width.width(), 32);

        self.saw_consumer = true;
        Ok(())
    }
}

#[derive(Default)]
struct ForwardOnlyConversion {
    saw_forward: bool,
}

impl DialectConversion for ForwardOnlyConversion {
    fn can_convert_op(&self, ctx: &Context, op: Ptr<Operation>) -> bool {
        Operation::get_op::<ForwardToSuccOp>(op, ctx).is_some()
    }

    fn can_convert_type(&self, ctx: &Context, ty: TypeHandle) -> bool {
        ty.deref(ctx)
            .downcast_ref::<IntegerType>()
            .is_some_and(|int_ty| int_ty.width() > 16)
    }

    fn convert_type(&mut self, ctx: &mut Context, ty: TypeHandle) -> Result<TypeHandle> {
        let width = ty
            .deref(ctx)
            .downcast_ref::<IntegerType>()
            .expect("expected integer type")
            .width();
        Ok(IntegerType::get(ctx, width / 2, Signedness::Signed).into())
    }

    fn rewrite(
        &mut self,
        ctx: &mut Context,
        _rewriter: &mut DialectConversionRewriter,
        op: Ptr<Operation>,
        _operands_info: &OperandsInfo,
    ) -> Result<()> {
        let succ = op.deref(ctx).get_successor(0);
        let succ_arg = succ.deref(ctx).get_argument(0);
        let succ_arg_width = succ_arg
            .get_type(ctx)
            .deref(ctx)
            .downcast_ref::<IntegerType>()
            .expect("expected integer type")
            .width();
        assert_eq!(succ_arg_width, 16);

        self.saw_forward = true;
        Ok(())
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn dialect_conversion_block_arg_type_conversion() -> Result<()> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();

    let module = ModuleOp::new(
        ctx,
        Identifier::try_from("block_arg_type_conversion").unwrap(),
    );
    let body = module.get_body(ctx, 0);
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let arg_idx = BasicBlock::push_argument(body, ctx, i64_ty);
    let block_arg = body.deref(ctx).get_argument(arg_idx);

    let consumer = ConsumerOp::new(ctx, block_arg);
    consumer.get_operation().insert_at_back(body, ctx);

    let mut conversion = ConsumerOnlyConversion::default();
    apply_dialect_conversion(ctx, &mut conversion, module.get_operation())?;
    assert!(conversion.saw_consumer);

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn dialect_conversion_dead_block_arg_type_conversion() -> Result<()> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();

    let module = ModuleOp::new(
        ctx,
        Identifier::try_from("dead_block_arg_type_conversion").unwrap(),
    );
    let body = module.get_body(ctx, 0);
    let region = body
        .deref(ctx)
        .get_parent_region()
        .expect("module body block must be in a region");

    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed).into();

    // Neither the entry block nor the successor of any operation, and its
    // argument is never used: nothing would ever visit this block under the
    // old "convert on use / convert on successor" scheme.
    let dead_block = BasicBlock::new(
        ctx,
        Some(Identifier::try_from("dead").unwrap()),
        vec![i64_ty],
    );
    dead_block.insert_at_back(region, ctx);

    let mut conversion = ConsumerOnlyConversion::default();
    apply_dialect_conversion(ctx, &mut conversion, module.get_operation())?;

    let dead_arg = dead_block.deref(ctx).get_argument(0);
    let dead_arg_width = dead_arg
        .get_type(ctx)
        .deref(ctx)
        .downcast_ref::<IntegerType>()
        .expect("expected integer type")
        .width();
    assert_eq!(dead_arg_width, 16);

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn dialect_conversion_successor_block_arg_type_conversion_without_uses() -> Result<()> {
    init_env_logger_for_tests!();
    let ctx = &mut Context::new();

    let module = ModuleOp::new(
        ctx,
        Identifier::try_from("block_arg_successor_type_conversion").unwrap(),
    );
    let pred_block = module.get_body(ctx, 0);
    let region = pred_block
        .deref(ctx)
        .get_parent_region()
        .expect("module body block must be in a region");

    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed).into();

    let producer = ProducerOp::new(ctx, 64);
    producer.get_operation().insert_at_back(pred_block, ctx);

    let succ_block = BasicBlock::new(
        ctx,
        Some(Identifier::try_from("succ").unwrap()),
        vec![i64_ty],
    );
    succ_block.insert_at_back(region, ctx);

    let forward = ForwardToSuccOp::new(ctx, producer.get_result(ctx), succ_block);
    forward.get_operation().insert_at_back(pred_block, ctx);

    let mut conversion = ForwardOnlyConversion::default();
    apply_dialect_conversion(ctx, &mut conversion, module.get_operation())?;
    assert!(conversion.saw_forward);

    let succ_arg = succ_block.deref(ctx).get_argument(0);
    let succ_arg_width = succ_arg
        .get_type(ctx)
        .deref(ctx)
        .downcast_ref::<IntegerType>()
        .expect("expected integer type")
        .width();
    assert_eq!(succ_arg_width, 16);

    Ok(())
}
