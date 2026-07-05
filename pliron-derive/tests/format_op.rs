//! Test format derive for `Op`s.

use ::pliron::combine::Parser;
use expect_test::expect;
use pliron::{
    builtin::op_interfaces::{
        IsTerminatorInterface, IsolatedFromAboveInterface, NOpdsInterface, NRegionsInterface,
        NResultsInterface, OneOpdInterface, OneRegionInterface, OneResultInterface,
    },
    context::Context,
    location,
    op::Op,
    operation::{Operation, verify_operation},
    parsable::{self, state_stream_from_iterator},
    printable::Printable,
};
use pliron_derive::{def_op, derive_op_interface_impl, format_op, verify_succ};
mod common;

#[format_op("")]
#[def_op("test.zero_results_zero_operands")]
#[derive_op_interface_impl(NOpdsInterface<0>, NResultsInterface<0>)]
#[verify_succ]
struct ZeroResultsZeroOperandsOp {}

#[test]
fn zero_results_zero_operands() {
    let ctx = &mut Context::new();

    let op = Operation::new(
        ctx,
        ZeroResultsZeroOperandsOp::get_concrete_op_info(),
        vec![],
        vec![],
        vec![],
        0,
    );
    let printed = op.disp(ctx).to_string();
    expect![r#"test.zero_results_zero_operands "#].assert_eq(&printed);

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );
    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("ZeroResultsZeroOperands parser failed");
    expect![[r#"
        test.zero_results_zero_operands  !0

        outlined_attributes:
        !0 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("`: ` type($0)")]
#[def_op("test.one_result_zero_operands")]
#[derive_op_interface_impl(NOpdsInterface<0>, NResultsInterface<1>, OneResultInterface)]
#[verify_succ]
struct OneResultZeroOperandsOp {}

#[test]
fn one_result_zero_operands() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc : builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands : builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("OneResultZeroOperands parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op]
#[def_op("test.one_result_zero_operands_canonical")]
#[derive_op_interface_impl(NOpdsInterface<0>, NResultsInterface<1>, OneResultInterface)]
#[verify_succ]
struct OneResultZeroOperandsCanonicalOp;

#[test]
fn one_result_zero_operands_canonical() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc : builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands_canonical () [] []: <() -> (builtin.integer si64)>;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("OneResultZeroOperandsCanonical parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands_canonical () [] []: <() -> (builtin.integer si64)> !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$0 `:` type($0)")]
#[def_op("test.one_result_one_operand")]
#[derive_op_interface_impl(NOpdsInterface<1>, NResultsInterface<1>, OneOpdInterface, OneResultInterface)]
#[verify_succ]
struct OneResultOneOperandOp {}

#[test]
fn one_result_one_operand() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands : builtin.integer si64;
            res1 = test.one_result_one_operand res0 : builtin.integer si64;
            test.return res1
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("OneResultOneOperand parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            res1_v1 = test.one_result_one_operand res0_v0:builtin.integer si64 !2;
            test.return res1_v1 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], [builtin_debug_info = builtin.debug_info [res1]]
        !3 = @[<in-memory>: line: 5, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$0 `,` $1 `:` `(` type($0) `,` type($1) `)`")]
#[def_op("test.two_results_two_operands")]
#[verify_succ]
struct TwoResultsTwoOperandsOp {}

#[test]
fn two_result_two_operands() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands :builtin.integer si64;
            res1a, res1b = test.two_results_two_operands res0, res0 : (builtin.integer si64, builtin.integer si64);
            test.return res1a
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("TwoResultTwoOperands parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            res1a_v1, res1b_v2 = test.two_results_two_operands res0_v0,res0_v0:(builtin.integer si64,builtin.integer si64) !2;
            test.return res1a_v1 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], [builtin_debug_info = builtin.debug_info [res1a, res1b]]
        !3 = @[<in-memory>: line: 5, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$0 `: ` `(` types(CharSpace(`,`)) `)`")]
#[def_op("test.two_results_one_operand")]
#[verify_succ]
struct TwoResultsOneOperandOp {}

#[test]
fn two_results_one_operand() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands : builtin.integer si64;
            res1a, res1b = test.two_results_one_operand res0 : (builtin.integer si64, builtin.integer si64);
            test.return res1a
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("TwoResultsOneOperand parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            res1a_v1, res1b_v2 = test.two_results_one_operand res0_v0: (builtin.integer si64, builtin.integer si64) !2;
            test.return res1a_v1 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], [builtin_debug_info = builtin.debug_info [res1a, res1b]]
        !3 = @[<in-memory>: line: 5, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$0 `, ` $1 `: (` opdtypes(CharSpace(`,`)) `) -> (` types(CharSpace(`,`)) `)`")]
#[def_op("test.one_result_two_operands")]
#[verify_succ]
struct OneResultTwoOperandsOp {}

#[test]
fn opdtypes() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands : builtin.integer si64;
            res1 = test.one_result_two_operands res0, res0 : (builtin.integer si64, builtin.integer si64) -> (builtin.integer si64);
            test.return res1
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("OneResultTwoOperands parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            res1_v1 = test.one_result_two_operands res0_v0, res0_v0: (builtin.integer si64, builtin.integer si64) -> (builtin.integer si64) !2;
            test.return res1_v1 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], [builtin_debug_info = builtin.debug_info [res1]]
        !3 = @[<in-memory>: line: 5, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$0 ` : ` typesig")]
#[def_op("test.one_result_one_operand_typesig")]
#[derive_op_interface_impl(NOpdsInterface<1>, NResultsInterface<1>, OneResultInterface)]
#[verify_succ]
struct OneResultOneOperandTypeSigOp {}

#[test]
fn one_result_one_operand_typesig() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands : builtin.integer si64;
            res1 = test.one_result_one_operand_typesig res0 : (builtin.integer si64) -> (builtin.integer si64);
            test.return res1
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("OneResultOneOperandTypeSig parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            res1_v1 = test.one_result_one_operand_typesig res0_v0 : (builtin.integer si64) -> (builtin.integer si64) !2;
            test.return res1_v1 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], [builtin_debug_info = builtin.debug_info [res1]]
        !3 = @[<in-memory>: line: 5, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$0 ` : (` opdtype($0) `)`")]
#[def_op("test.zero_results_one_operand")]
#[verify_succ]
struct ZeroResultsOneOperandOp {}

#[test]
fn opdtype() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.one_result_zero_operands : builtin.integer si64;
            test.zero_results_one_operand res0 : (builtin.integer si64);
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("OneResultTwoOperands parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.one_result_zero_operands : builtin.integer si64 !1;
            test.zero_results_one_operand res0_v0 : (builtin.integer si64) !2;
            test.return res0_v0 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 5, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

use pliron::builtin::attributes::IntegerAttr;

#[format_op("attr($attr, $IntegerAttr) `:` type($0)")]
#[def_op("test.attr_op")]
#[verify_succ]
struct AttrOp {}

#[test]
fn attr_op() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op <0: si64> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op(
    "attr($attr, `pliron::builtin::attributes::StringAttr`) ` ` \
    opt_attr($opt_attr, `pliron::builtin::attributes::IntegerAttr`) `:` type($0)"
)]
#[def_op("test.attr_op2")]
#[verify_succ]
struct AttrOp2 {}

#[test]
fn attr_op2() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2 \"Hello World\" :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op2 "Hello World" :builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());

    let printed_with_opt_attr = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2 \"Hello World\" <42: si64> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed_with_opt_attr.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed with optional attribute");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block2v1() !0:
            res0_v1 = test.attr_op2 "Hello World" <42: si64>:builtin.integer si64 !1;
            test.return res0_v1 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op(
    "attr($attr, `pliron::builtin::attributes::StringAttr`, label($name)) ` ` \
    opt_attr($opt_attr, `pliron::builtin::attributes::IntegerAttr`, label($value)) `:` type($0)"
)]
#[def_op("test.attr_op2_labeled")]
#[verify_succ]
struct AttrOp2Labeled {}

#[test]
fn attr_op2_labeled() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2_labeled name: \"Hello World\" :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op2_labeled name : "Hello World" :builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());

    let printed_with_opt_attr = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2_labeled name : \"Hello World\" value : <42: si64> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed_with_opt_attr.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed with optional attribute");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block2v1() !0:
            res0_v1 = test.attr_op2_labeled name : "Hello World" value : <42: si64>:builtin.integer si64 !1;
            test.return res0_v1 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op(
    "attr($attr, `pliron::builtin::attributes::StringAttr`, label($name), delimiters(`<`, `>`)) ` ` \
    opt_attr($opt_attr, `pliron::builtin::attributes::IntegerAttr`, label($value), delimiters(`<<`, `>>`)) `:` type($0)"
)]
#[def_op("test.attr_op2_labeled_delimited")]
#[verify_succ]
struct AttrOp2Labeleddelimited {}

#[test]
fn attr_op2_labeled_delimited() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2_labeled_delimited <name: \"Hello World\"> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op2_labeled_delimited <name : "Hello World"> :builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());

    let printed_with_opt_attr = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2_labeled_delimited <name : \"Hello World\"> <<value : <42: si64>>> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed_with_opt_attr.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed with optional attribute");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block2v1() !0:
            res0_v1 = test.attr_op2_labeled_delimited <name : "Hello World"> <<value : <42: si64>>>:builtin.integer si64 !1;
            test.return res0_v1 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op(
    "attr($attr, `pliron::builtin::attributes::StringAttr`, delimiters(`<`, `>`)) ` ` \
    opt_attr($opt_attr, `pliron::builtin::attributes::IntegerAttr`, delimiters(`[[`, `]]`)) `:` type($0)"
)]
#[def_op("test.attr_op2_delimited")]
#[verify_succ]
struct AttrOp2delimited {}

#[test]
fn attr_op2_delimited() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2_delimited <\"Hello World\"> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op2_delimited <"Hello World"> :builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());

    let printed_with_opt_attr = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op2_delimited <\"Hello World\"> [[<42: si64>]] :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed_with_opt_attr.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed with optional attribute");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block2v1() !0:
            res0_v1 = test.attr_op2_delimited <"Hello World"> [[<42: si64>]]:builtin.integer si64 !1;
            test.return res0_v1 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("$attr `:` type($0)")]
#[def_op("test.attr_op3")]
#[verify_succ]
struct AttrOp3 {}

#[test]
fn attr_op3() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op3 builtin.integer <0: si64> :builtin.integer si64;
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op3 builtin.integer <0: si64>:builtin.integer si64 !1;
            test.return res0_v0 !2
        } !3

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("`(`$0`)` region($0)")]
#[def_op("test.if_op")]
#[derive_op_interface_impl(NOpdsInterface<1>, OneOpdInterface, NResultsInterface<0>, NRegionsInterface<1>, OneRegionInterface)]
#[verify_succ]
struct IfOp {}

#[test]
fn if_op() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op <0: si64> :builtin.integer si64;
            test.if_op (res0) {
              ^then():
                res1 = test.attr_op <1: si64> :builtin.integer si64;
                test.return res1
            };
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("IfOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block2v1() !0:
            res0_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.if_op (res0_v0)
            {
              ^then_block1v1() !2:
                res1_v1 = test.attr_op <1: si64>:builtin.integer si64 !3;
                test.return res1_v1 !4
            } !5;
            test.return res0_v0 !6
        } !7

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 5, column: 15], []
        !3 = @[<in-memory>: line: 6, column: 17], [builtin_debug_info = builtin.debug_info [res1]]
        !4 = @[<in-memory>: line: 7, column: 17], []
        !5 = @[<in-memory>: line: 4, column: 13], []
        !6 = @[<in-memory>: line: 9, column: 13], []
        !7 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[format_op("`(`$0`)` region($0) `else` region($1)")]
#[def_op("test.if_else_op")]
#[derive_op_interface_impl(NOpdsInterface<1>, OneOpdInterface, NResultsInterface<0>, NRegionsInterface<2>)]
#[verify_succ]
struct IfElseOp {}

#[test]
fn if_else_op() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op <0: si64> :builtin.integer si64;
            test.if_else_op (res0) {
              ^then():
                res1 = test.attr_op <1: si64> :builtin.integer si64;
                test.return res1
            } else {
              ^else():
                res2 = test.attr_op <2: si64> :builtin.integer si64;
                test.return res2
            };
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("IfOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block3v1() !0:
            res0_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.if_else_op (res0_v0)
            {
              ^then_block1v1() !2:
                res1_v1 = test.attr_op <1: si64>:builtin.integer si64 !3;
                test.return res1_v1 !4
            }else
            {
              ^else_block2v1() !5:
                res2_v2 = test.attr_op <2: si64>:builtin.integer si64 !6;
                test.return res2_v2 !7
            } !8;
            test.return res0_v0 !9
        } !10

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 5, column: 15], []
        !3 = @[<in-memory>: line: 6, column: 17], [builtin_debug_info = builtin.debug_info [res1]]
        !4 = @[<in-memory>: line: 7, column: 17], []
        !5 = @[<in-memory>: line: 9, column: 15], []
        !6 = @[<in-memory>: line: 10, column: 17], [builtin_debug_info = builtin.debug_info [res2]]
        !7 = @[<in-memory>: line: 11, column: 17], []
        !8 = @[<in-memory>: line: 4, column: 13], []
        !9 = @[<in-memory>: line: 13, column: 13], []
        !10 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[def_op("test.br")]
#[format_op("succ($0) `(` operands(CharSpace(`,`)) `)`")]
#[derive_op_interface_impl(IsTerminatorInterface)]
#[verify_succ]
pub struct BrOp;

#[test]
fn br_op() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op <0: si64> :builtin.integer si64;
            test.br ^bb1(res0)
          ^bb1(arg0 : builtin.integer si64):
            test.return arg0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("BrOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block2v1() !0:
            res0_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.br ^bb1_block3v1(res0_v0) !2

          ^bb1_block3v1(arg0_v2: builtin.integer si64) !3:
            test.return arg0_v2 !4
        } !5

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 5, column: 11], [builtin_debug_info = builtin.debug_info [arg0]]
        !4 = @[<in-memory>: line: 6, column: 13], []
        !5 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[def_op("test.multiple_successors")]
#[format_op("`[` successors(CharSpace(`,`)) `]`")]
#[derive_op_interface_impl(IsTerminatorInterface)]
#[verify_succ]
pub struct MultipleSuccessorsOp;

#[test]
fn multiple_successors_op() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op <0: si64> :builtin.integer si64;
            test.multiple_successors [^bb1, ^bb2]
          ^bb1():
            test.return res0
          ^bb2():
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("MultipleSuccessorsOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block3v1() !0:
            res0_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.multiple_successors [^bb1_block4v1, ^bb2_block1v3] !2

          ^bb1_block4v1() !3:
            test.return res0_v0 !4

          ^bb2_block1v3() !5:
            test.return res0_v0 !6
        } !7

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 5, column: 11], []
        !4 = @[<in-memory>: line: 6, column: 13], []
        !5 = @[<in-memory>: line: 7, column: 11], []
        !6 = @[<in-memory>: line: 8, column: 13], []
        !7 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[def_op("test.multiple_regions")]
#[format_op("`[` regions(CharSpace(`,`)) `]`")]
#[verify_succ]
pub struct MultipleRegionsOp;

#[test]
fn multiple_regions_op() {
    let ctx = &mut Context::new();

    let printed = "
    builtin.func @testfunc: builtin.function <()->()> {
      ^entry():
        res = test.attr_op <0: si64> :builtin.integer si64;
        test.multiple_regions [
            {
                ^reg1_entry():
                    res0 = test.attr_op <0: si64> :builtin.integer si64;
                    test.return res0
            }, {
                ^reg2_entry():
                    res1 = test.attr_op <1: si64> :builtin.integer si64;
                    test.return res1
            }
        ];
        test.return res
    }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("MultipleRegionsOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block3v1() !0:
            res_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.multiple_regions [
            {
              ^reg1_entry_block1v1() !2:
                res0_v1 = test.attr_op <0: si64>:builtin.integer si64 !3;
                test.return res0_v1 !4
            }, 
            {
              ^reg2_entry_block2v1() !5:
                res1_v2 = test.attr_op <1: si64>:builtin.integer si64 !6;
                test.return res1_v2 !7
            }] !8;
            test.return res_v0 !9
        } !10

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 7], []
        !1 = @[<in-memory>: line: 4, column: 9], [builtin_debug_info = builtin.debug_info [res]]
        !2 = @[<in-memory>: line: 7, column: 17], []
        !3 = @[<in-memory>: line: 8, column: 21], [builtin_debug_info = builtin.debug_info [res0]]
        !4 = @[<in-memory>: line: 9, column: 21], []
        !5 = @[<in-memory>: line: 11, column: 17], []
        !6 = @[<in-memory>: line: 12, column: 21], [builtin_debug_info = builtin.debug_info [res1]]
        !7 = @[<in-memory>: line: 13, column: 21], []
        !8 = @[<in-memory>: line: 5, column: 9], []
        !9 = @[<in-memory>: line: 16, column: 9], []
        !10 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[def_op("test.attr_dict")]
#[format_op("attr_dict")]
#[verify_succ]
struct AttrDictOp;

#[test]
fn attr_dict_op() {
    let ctx = &mut Context::new();

    let printed = "builtin.func @testfunc: builtin.function <() -> ()> {
          ^entry():
            res0 = test.attr_op <3: si64> :builtin.integer si64;
            test.attr_dict [
                attr1: builtin.integer <0: si64>,
                attr2: builtin.integer <1: si64>
            ];
            test.return res0
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("AttrDictOp parser failed");

    expect![[r#"
        builtin.func @testfunc: builtin.function <() -> ()> 
        {
          ^entry_block1v1() !0:
            res0_v0 = test.attr_op <3: si64>:builtin.integer si64 !1;
            test.attr_dict [attr1: builtin.integer <0: si64>, attr2: builtin.integer <1: si64>] !2;
            test.return res0_v0 !3
        } !4

        outlined_attributes:
        !0 = @[<in-memory>: line: 2, column: 11], []
        !1 = @[<in-memory>: line: 3, column: 13], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 4, column: 13], []
        !3 = @[<in-memory>: line: 8, column: 13], []
        !4 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());
}

#[def_op("test.multiple_regions2")]
#[format_op]
#[verify_succ]
pub struct MultipleRegions2Op;

#[test]
fn multiple_regions2_op() {
    let ctx = &mut Context::new();

    let printed = "
        test.multiple_regions2 () [] [] : <()->()>
        {
            ^reg1_entry():
                res0 = test.attr_op <0: si64> :builtin.integer si64;
                test.return res0
        }, {
            ^reg2_entry():
                res1 = test.attr_op <1: si64> :builtin.integer si64;
                test.return res1
        }";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let actual = Operation::top_level_parser()
        .parse(state_stream)
        .err()
        .unwrap();
    let expected_err = expect![[r#"
        Parse error at line: 1, column: 1
        Regions in a top-level operation must be IsolatedFromAbove
    "#]];
    expected_err.assert_eq(&actual.to_string());
}

#[def_op("test.multiple_regions3")]
#[format_op("`[` regions(CharSpace(`,`)) `]`")]
#[verify_succ]
pub struct MultipleRegions3Op;

#[test]
fn multiple_regions3_op() {
    let ctx = &mut Context::new();

    let printed = "
        test.multiple_regions3 [
            {
                ^reg1_entry():
                    res0 = test.attr_op <0: si64> :builtin.integer si64;
                    test.return res0
            }, {
                ^reg2_entry():
                    res1 = test.attr_op <1: si64> :builtin.integer si64;
                    test.return res1
            }
        ]";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );

    let actual = Operation::top_level_parser()
        .parse(state_stream)
        .err()
        .unwrap();
    let expected_err = expect![[r#"
        Parse error at line: 1, column: 1
        Regions in a top-level operation must be IsolatedFromAbove
    "#]];
    expected_err.assert_eq(&actual.to_string());
}

#[def_op("test.multiple_regions4")]
#[format_op("`[` regions(CharSpace(`,`)) `]`")]
#[derive_op_interface_impl(IsolatedFromAboveInterface)]
#[verify_succ]
pub struct MultipleRegions4Op;

#[test]
fn multiple_regions4_op() {
    let ctx = &mut Context::new();

    let printed = "
        test.multiple_regions4 [
            {
                ^reg1_entry():
                    res0 = test.attr_op <0: si64> :builtin.integer si64;
                    test.return res0
            }, {
                ^reg2_entry():
                    res1 = test.attr_op <1: si64> :builtin.integer si64;
                    test.return res1
            }
        ]";

    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx, location::Source::InMemory),
    );
    let (res, _) = Operation::top_level_parser()
        .parse(state_stream)
        .expect("MultipleRegions4 parser failed");

    expect![[r#"
        test.multiple_regions4 [
        {
          ^reg1_entry_block1v1() !0:
            res0_v0 = test.attr_op <0: si64>:builtin.integer si64 !1;
            test.return res0_v0 !2
        }, 
        {
          ^reg2_entry_block2v1() !3:
            res1_v1 = test.attr_op <1: si64>:builtin.integer si64 !4;
            test.return res1_v1 !5
        }] !6

        outlined_attributes:
        !0 = @[<in-memory>: line: 4, column: 17], []
        !1 = @[<in-memory>: line: 5, column: 21], [builtin_debug_info = builtin.debug_info [res0]]
        !2 = @[<in-memory>: line: 6, column: 21], []
        !3 = @[<in-memory>: line: 8, column: 17], []
        !4 = @[<in-memory>: line: 9, column: 21], [builtin_debug_info = builtin.debug_info [res1]]
        !5 = @[<in-memory>: line: 10, column: 21], []
        !6 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&res.disp(ctx).to_string());

    assert!(verify_operation(res, ctx).is_ok());

    Operation::erase(res, ctx);
    assert!(ctx.is_ir_empty());
}
