// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Integration tests for op-related derive macros:
//! - `operands` / `results` (getter generation and typed interface derivation)
//! - `derive_attr_get_set` (attribute getter/setter generation)
//! - `derive_op_interface_impl` (interface implementation boilerplate derivation)

use pliron::{
    attribute::AttrObj,
    builtin::{
        attributes::{IntegerAttr, StringAttr, UnitAttr},
        op_interfaces::{
            NOpdsInterface, NResultsInterface, OperandNOfType, OperandNOfTypeError, ResultNOfType,
            ResultNOfTypeError,
        },
        types::{IntegerType, Signedness, UnitType},
    },
    context::Context,
    op::{Op, op_impls, verify_op},
    operation::Operation,
    result::{Error, ErrorKind, Result},
    r#type::TypeHandle,
    utils::apint::APInt,
    value::Value,
};
use pliron_derive::{
    def_op, derive_attr_get_set, derive_op_interface_impl, format_op, op_interface,
    op_interface_impl, operands, results, verify_succ,
};

// ---------------------------------------------------------------------------
// Shared helper: produce a single-result value of a given type.
// ---------------------------------------------------------------------------

#[def_op("test.value_producer")]
#[format_op]
#[derive_op_interface_impl(NOpdsInterface<0>, NResultsInterface<1>)]
#[verify_succ]
struct ValueProducerOp;

impl ValueProducerOp {
    fn new(ctx: &mut Context, ty: TypeHandle) -> Self {
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

    fn result(&self, ctx: &Context) -> Value {
        self.get_operation().deref(ctx).get_result(0)
    }
}

// ---------------------------------------------------------------------------
// Ops used by operands / results tests.
// ---------------------------------------------------------------------------

/// An op with typed+named operands and typed+named results, using `operands`
/// and `results` macros.
#[def_op("test.named_getter_op")]
#[format_op]
#[derive_op_interface_impl(NOpdsInterface<3>, NResultsInterface<2>)]
#[results(out: IntegerType, _: UnitType)]
#[operands(lhs: IntegerType, _, rhs)]
#[verify_succ]
struct NamedGetterOp;

impl NamedGetterOp {
    fn new(
        ctx: &mut Context,
        lhs: Value,
        mid: Value,
        rhs: Value,
        out_ty: TypeHandle,
        aux_ty: TypeHandle,
    ) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![out_ty, aux_ty],
            vec![lhs, mid, rhs],
            vec![],
            0,
        );
        Self { op }
    }
}

/// A single-operand, single-result op with typed entries in both `operands`
/// and `results`. Used for interface verification failure tests.
#[def_op("test.typed_checks_op")]
#[format_op]
#[derive_op_interface_impl(NOpdsInterface<1>, NResultsInterface<1>)]
#[results(out: IntegerType)]
#[operands(arg: IntegerType)]
#[verify_succ]
struct TypedChecksOp;

impl TypedChecksOp {
    fn new(ctx: &mut Context, arg: Value, res_ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![arg],
            vec![],
            0,
        );
        Self { op }
    }
}

/// A single-operand, single-result op where both entries are named but untyped.
#[def_op("test.untyped_named_op")]
#[format_op]
#[derive_op_interface_impl(NOpdsInterface<1>, NResultsInterface<1>)]
#[results(out)]
#[operands(arg)]
#[verify_succ]
struct UntypedNamedOp;

impl UntypedNamedOp {
    fn new(ctx: &mut Context, arg: Value, res_ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![arg],
            vec![],
            0,
        );
        Self { op }
    }
}

/// A single-operand, single-result op where both entries are skipped and
/// untyped.
#[def_op("test.skip_untyped_op")]
#[format_op]
#[derive_op_interface_impl(NOpdsInterface<1>, NResultsInterface<1>)]
#[results(_)]
#[operands(_)]
#[verify_succ]
struct SkipUntypedOp;

impl SkipUntypedOp {
    fn new(ctx: &mut Context, arg: Value, res_ty: TypeHandle) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res_ty],
            vec![arg],
            vec![],
            0,
        );
        Self { op }
    }
}

/// A mixed op with skipped and named untyped entries plus skipped typed
/// entries to exercise indexing and interface derivation.
#[def_op("test.mixed_skip_untyped_op")]
#[format_op]
#[derive_op_interface_impl(NOpdsInterface<3>, NResultsInterface<3>)]
#[results(_, out, _: IntegerType)]
#[operands(_, mid, _: IntegerType)]
#[verify_succ]
struct MixedSkipUntypedOp;

impl MixedSkipUntypedOp {
    fn new(
        ctx: &mut Context,
        op0: Value,
        op1: Value,
        op2: Value,
        res0_ty: TypeHandle,
        res1_ty: TypeHandle,
        res2_ty: TypeHandle,
    ) -> Self {
        let op = Operation::new(
            ctx,
            Self::get_concrete_op_info(),
            vec![res0_ty, res1_ty, res2_ty],
            vec![op0, op1, op2],
            vec![],
            0,
        );
        Self { op }
    }
}

// ---------------------------------------------------------------------------
// Tests: operands / results
// ---------------------------------------------------------------------------

#[test]
fn named_operand_and_result_getters_work() -> Result<()> {
    let ctx = &mut Context::new();

    let int_ty: TypeHandle = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let unit_ty: TypeHandle = UnitType::get(ctx).into();

    let lhs = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let mid = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let rhs = ValueProducerOp::new(ctx, int_ty).result(ctx);

    let op = NamedGetterOp::new(ctx, lhs, mid, rhs, int_ty, unit_ty);

    // Named operand getters return the correct values.
    assert_eq!(op.get_operand_lhs(ctx), lhs);
    assert_eq!(op.get_operand_rhs(ctx), rhs);

    // Named result getter returns the correct value.
    assert_eq!(
        op.get_result_out(ctx),
        op.get_operation().deref(ctx).get_result(0)
    );

    // The op passes verification.
    verify_op(&op, ctx)?;

    // Typed interfaces are castable.
    assert!(op_impls::<dyn OperandNOfType<0, IntegerType>>(&op));
    assert!(op_impls::<dyn ResultNOfType<0, IntegerType>>(&op));

    Ok(())
}

#[test]
fn operand_type_interface_verification_fails() {
    let ctx = &mut Context::new();

    let int_ty: TypeHandle = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let unit_ty: TypeHandle = UnitType::get(ctx).into();

    // Feed a UnitType value where IntegerType is expected by operand 0.
    let wrong_operand = ValueProducerOp::new(ctx, unit_ty).result(ctx);
    let op = TypedChecksOp::new(ctx, wrong_operand, int_ty);

    assert!(matches!(
        verify_op(&op, ctx),
        Err(Error {
            kind: ErrorKind::VerificationFailed,
            err,
            ..
        })
        if err.is::<OperandNOfTypeError>()
    ));
}

#[test]
fn result_type_interface_verification_fails() {
    let ctx = &mut Context::new();

    let int_ty: TypeHandle = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let unit_ty: TypeHandle = UnitType::get(ctx).into();

    // Feed a UnitType where IntegerType is expected as result 0.
    let operand = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let op = TypedChecksOp::new(ctx, operand, unit_ty);

    assert!(matches!(
        verify_op(&op, ctx),
        Err(Error {
            kind: ErrorKind::VerificationFailed,
            err,
            ..
        })
        if err.is::<ResultNOfTypeError>()
    ));
}

#[test]
fn untyped_named_entries_generate_getters_without_type_interfaces() -> Result<()> {
    let ctx = &mut Context::new();

    let int_ty: TypeHandle = IntegerType::get(ctx, 64, Signedness::Signed).into();

    let arg = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let op = UntypedNamedOp::new(ctx, arg, int_ty);

    assert_eq!(op.get_operand_arg(ctx), arg);
    assert_eq!(
        op.get_result_out(ctx),
        op.get_operation().deref(ctx).get_result(0)
    );

    verify_op(&op, ctx)?;

    assert!(!op_impls::<dyn OperandNOfType<0, IntegerType>>(&op));
    assert!(!op_impls::<dyn ResultNOfType<0, IntegerType>>(&op));

    Ok(())
}

#[test]
fn skip_untyped_entries_have_no_type_interfaces() -> Result<()> {
    let ctx = &mut Context::new();

    let int_ty: TypeHandle = IntegerType::get(ctx, 64, Signedness::Signed).into();

    let arg = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let op = SkipUntypedOp::new(ctx, arg, int_ty);

    verify_op(&op, ctx)?;

    assert!(!op_impls::<dyn OperandNOfType<0, IntegerType>>(&op));
    assert!(!op_impls::<dyn ResultNOfType<0, IntegerType>>(&op));

    Ok(())
}

#[test]
fn mixed_skip_and_untyped_entries_map_indices_correctly() -> Result<()> {
    let ctx = &mut Context::new();

    let int_ty: TypeHandle = IntegerType::get(ctx, 64, Signedness::Signed).into();
    let unit_ty: TypeHandle = UnitType::get(ctx).into();

    let op0 = ValueProducerOp::new(ctx, unit_ty).result(ctx);
    let op1 = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let op2 = ValueProducerOp::new(ctx, int_ty).result(ctx);
    let op = MixedSkipUntypedOp::new(ctx, op0, op1, op2, unit_ty, int_ty, int_ty);

    assert_eq!(op.get_operand_mid(ctx), op1);
    assert_eq!(
        op.get_result_out(ctx),
        op.get_operation().deref(ctx).get_result(1)
    );

    verify_op(&op, ctx)?;

    assert!(op_impls::<dyn OperandNOfType<2, IntegerType>>(&op));
    assert!(op_impls::<dyn ResultNOfType<2, IntegerType>>(&op));
    assert!(!op_impls::<dyn OperandNOfType<1, IntegerType>>(&op));
    assert!(!op_impls::<dyn ResultNOfType<1, IntegerType>>(&op));

    Ok(())
}

// ---------------------------------------------------------------------------
// Ops used by derive_attr_get_set tests.
// ---------------------------------------------------------------------------

/// An op with three attributes: a typed `IntegerAttr`, a typed `StringAttr`,
/// and an untyped (any) attribute slot.  Shares its op definition with all
/// attribute getter/setter tests.
#[def_op("test.attr_op")]
#[format_op]
#[derive_attr_get_set(count: IntegerAttr, label: StringAttr, extra)]
#[verify_succ]
struct AttrOp;

impl AttrOp {
    fn new(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        Self { op }
    }
}

// ---------------------------------------------------------------------------
// Tests: derive_attr_get_set
// ---------------------------------------------------------------------------

#[test]
fn typed_attr_getter_setter_round_trip() -> Result<()> {
    let ctx = &mut Context::new();
    let op = AttrOp::new(ctx);

    // Initially absent.
    assert!(op.get_attr_count(ctx).is_none());

    // Set a typed attribute then read it back.
    let int_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let attr = IntegerAttr::new(
        int_ty,
        APInt::from_u64(42, core::num::NonZero::new(64).unwrap()),
    );
    op.set_attr_count(ctx, attr.clone());

    let got = op
        .get_attr_count(ctx)
        .expect("count attribute should be present");
    assert_eq!(*got, attr);

    Ok(())
}

#[test]
fn second_typed_attr_independent_of_first() -> Result<()> {
    let ctx = &mut Context::new();
    let op = AttrOp::new(ctx);

    op.set_attr_label(ctx, StringAttr::new("hello".into()));

    // 'count' is still absent.
    assert!(op.get_attr_count(ctx).is_none());
    // 'label' is present.
    assert_eq!(
        *op.get_attr_label(ctx).unwrap(),
        StringAttr::new("hello".into())
    );

    Ok(())
}

#[test]
fn untyped_attr_getter_setter_round_trip() -> Result<()> {
    let ctx = &mut Context::new();
    let op = AttrOp::new(ctx);

    assert!(op.get_attr_extra(ctx).is_none());

    let boxed: AttrObj = Box::new(UnitAttr::new());
    op.set_attr_extra(ctx, boxed);

    assert!(op.get_attr_extra(ctx).is_some());

    Ok(())
}

#[test]
fn overwriting_attr_replaces_value() -> Result<()> {
    let ctx = &mut Context::new();
    let op = AttrOp::new(ctx);

    let int_ty = IntegerType::get(ctx, 64, Signedness::Signed);
    let attr1 = IntegerAttr::new(
        int_ty,
        APInt::from_u64(1, core::num::NonZero::new(64).unwrap()),
    );
    let attr2 = IntegerAttr::new(
        int_ty,
        APInt::from_u64(2, core::num::NonZero::new(64).unwrap()),
    );

    op.set_attr_count(ctx, attr1.clone());
    assert_eq!(*op.get_attr_count(ctx).unwrap(), attr1);

    op.set_attr_count(ctx, attr2.clone());
    assert_eq!(*op.get_attr_count(ctx).unwrap(), attr2);

    Ok(())
}

// ---------------------------------------------------------------------------
// Ops and interface used by derive_op_interface_impl tests.
// ---------------------------------------------------------------------------

#[op_interface]
trait ComputeInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> pliron::result::Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[op_interface]
trait ExtendedComputeInterface: ComputeInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> pliron::result::Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

/// An op that uses `derive_op_interface_impl` to implement both interfaces
/// without writing any impl body.
#[def_op("test.compute_op")]
#[format_op]
#[derive_op_interface_impl(ComputeInterface, ExtendedComputeInterface)]
#[verify_succ]
struct ComputeOp;

impl ComputeOp {
    fn new(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        Self { op }
    }
}

/// An op that implements an interface manually via `op_interface_impl` (the
/// "explicit" path) to contrast with the derived path above.
#[def_op("test.manual_compute_op")]
#[format_op]
#[verify_succ]
struct ManualComputeOp;

#[op_interface_impl]
impl ComputeInterface for ManualComputeOp {}

impl ManualComputeOp {
    fn new(ctx: &mut Context) -> Self {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        Self { op }
    }
}

// ---------------------------------------------------------------------------
// Tests: derive_op_interface_impl
// ---------------------------------------------------------------------------

#[test]
fn derived_interfaces_are_castable() -> Result<()> {
    let ctx = &mut Context::new();
    let op = ComputeOp::new(ctx);

    assert!(op_impls::<dyn ComputeInterface>(&op));
    assert!(op_impls::<dyn ExtendedComputeInterface>(&op));

    Ok(())
}

#[test]
fn derived_interfaces_verify_ok() -> Result<()> {
    let ctx = &mut Context::new();
    let op = ComputeOp::new(ctx);

    verify_op(&op, ctx)
}

#[test]
fn manual_interface_impl_is_castable() -> Result<()> {
    let ctx = &mut Context::new();
    let op = ManualComputeOp::new(ctx);

    assert!(op_impls::<dyn ComputeInterface>(&op));

    Ok(())
}

#[test]
fn op_without_interface_is_not_castable() -> Result<()> {
    let ctx = &mut Context::new();
    // ValueProducerOp does not implement ComputeInterface.
    let op = ValueProducerOp::new(ctx, UnitType::get(ctx).into());

    assert!(!op_impls::<dyn ComputeInterface>(&op));

    Ok(())
}
