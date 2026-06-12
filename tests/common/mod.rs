use awint::bw;
use pliron::{
    attribute::AttrObj,
    builtin::{
        attributes::IntegerAttr,
        op_interfaces::{
            IsTerminatorInterface, NOpdsInterface, NResultsInterface, NResultsVerifyErr,
            OneResultInterface, SingleBlockRegionInterface,
        },
        ops::{FuncOp, ModuleOp},
        types::{FunctionType, IntegerType, Signedness},
    },
    context::Context,
    debug_info::set_operation_result_name,
    derive::pliron_op,
    identifier::Identifier,
    init_env_logger_for_tests, input_err,
    irfmt::parsers::{attr_parser, process_parsed_ssa_defs},
    location::{Located, Location},
    op::{Op, OpObj, verify_op},
    operation::Operation,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    result::Result,
    utils::apint::APInt,
    value::Value,
};

#[pliron_op(
    name = "test.return",
    format = "$0",
    interfaces = [IsTerminatorInterface],
    verifier = "succ",
)]
pub struct ReturnOp;
impl ReturnOp {
    pub fn new(ctx: &mut Context, value: Value) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![],
            vec![value],
            vec![],
            0,
        );
        ReturnOp { op }
    }
}

mod constant_op {
    use pliron::dict_key;
    dict_key!(ATTR_KEY_VALUE, "test_constant_value");
}

#[pliron_op(
    name = "test.constant",
    interfaces = [NOpdsInterface<0>, OneResultInterface, NResultsInterface<1>],
    verifier = "succ",
)]
pub struct ConstantOp;
impl ConstantOp {
    pub fn new(ctx: &mut Context, value: u64) -> Self {
        let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
        let int_attr = IntegerAttr::new(i64_ty, APInt::from_u64(value, bw(64)));
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![i64_ty.into()],
            vec![],
            vec![],
            0,
        );
        op.deref_mut(ctx)
            .attributes
            .0
            .insert(constant_op::ATTR_KEY_VALUE.clone(), Box::new(int_attr));
        ConstantOp { op }
    }

    pub fn get_value(&self, ctx: &Context) -> AttrObj {
        let op = self.get_operation().deref(ctx);
        op.attributes
            .0
            .get(&*constant_op::ATTR_KEY_VALUE)
            .unwrap()
            .clone()
    }
}
impl Printable for ConstantOp {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(
            f,
            "{} = {} {}",
            self.get_result(ctx).disp(ctx),
            self.get_opid().disp(ctx),
            self.get_value(ctx).disp(ctx)
        )
    }
}
impl Parsable for ConstantOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        results: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        let loc = state_stream.loc();

        if results.len() != 1 {
            input_err!(loc.clone(), NResultsVerifyErr(1, results.len()))?
        }

        let attr = attr_parser().parse_stream(state_stream).into_result()?.0;
        let int_attr = match attr.downcast::<IntegerAttr>() {
            Ok(int_attr) => int_attr,
            Err(attr) => input_err!(
                loc,
                "Expected integer attribute, but found {}",
                attr.disp(state_stream.state.ctx)
            )?,
        };
        let int_val: u64 = Into::<APInt>::into(*int_attr).to_u64();
        let op = Self::new(state_stream.state.ctx, int_val);
        process_parsed_ssa_defs(state_stream, &results, op.get_operation())?;

        Ok(OpObj::new(op)).into_parse_result()
    }
}

// Create a print a module "bar", with a function "foo"
// containing a single `return 0`.
pub fn const_ret_in_mod(ctx: &mut Context) -> Result<(ModuleOp, FuncOp, ConstantOp, ReturnOp)> {
    init_env_logger_for_tests!();
    let i64_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let module = ModuleOp::new(ctx, "bar".try_into().unwrap());
    // Our function is going to have type () -> ().
    let func_ty = FunctionType::get(ctx, vec![], vec![i64_ty.into()]);
    let func = FuncOp::new(ctx, "foo".try_into().unwrap(), func_ty);
    module.append_operation(ctx, func.get_operation(), 0);
    let bb = func.get_entry_block(ctx);

    // Create a `const 0` op and add it to bb.
    let const_op = ConstantOp::new(ctx, 0);
    const_op.get_operation().insert_at_front(bb, ctx);
    set_operation_result_name(
        ctx,
        const_op.get_operation(),
        0,
        Some("c0".try_into().unwrap()),
    );

    // Return the constant.
    let ret_op = ReturnOp::new(ctx, const_op.get_result(ctx));
    ret_op.get_operation().insert_at_back(bb, ctx);

    verify_op(&module, ctx)?;

    Ok((module, func, const_op, ret_op))
}
