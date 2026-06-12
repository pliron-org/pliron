mod common;

use std::sync::{LazyLock, Mutex};

use common::{ConstantOp, ReturnOp};
use expect_test::expect;

use pliron::{
    attribute::{Attribute, attr_cast, verify_attr},
    builtin::{
        attr_interfaces::{OutlinedAttr, PrintOnceAttr, TypedAttrInterface},
        attributes::{IntegerAttr, StringAttr},
        op_interfaces::{NResultsInterface, NResultsVerifyErr, OneResultInterface},
        ops::ModuleOp,
        types::{IntegerType, UnitType},
    },
    combine::{Parser, stream::position::SourcePosition},
    common_traits::Verify,
    context::{Context, Ptr},
    derive::{
        attr_interface, attr_interface_impl, op_interface, op_interface_impl, pliron_attr,
        pliron_op, pliron_type, type_interface, type_interface_impl,
    },
    identifier::Identifier,
    input_error,
    location::{self, Located, Location, Source},
    op::{Op, OpObj, op_cast, verify_op},
    operation::{Operation, verify_operation},
    parsable::{self, Parsable, ParseResult, StateStream, state_stream_from_iterator},
    printable::{self, Printable},
    result::{Error, ErrorKind, ExpectOk, Result},
    storage_uniquer::TypeValueHash,
    r#type::{Type, TypeObj, type_cast, verify_type},
    utils::trait_cast::any_to_trait,
    verify_err,
};
use thiserror::Error;

use crate::common::const_ret_in_mod;

#[cfg(target_family = "wasm")]
use wasm_bindgen_test::*;

#[pliron_op(name = "test.zero_result", verifier = "succ")]
struct ZeroResultOp {}

// This is setup to fail.
#[op_interface_impl]
impl OneResultInterface for ZeroResultOp {}

#[op_interface_impl]
impl NResultsInterface<1> for ZeroResultOp {}

impl Printable for ZeroResultOp {
    fn fmt(
        &self,
        _ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "zero_results",)
    }
}

impl Parsable for ZeroResultOp {
    type Arg = Vec<(Identifier, Location)>;
    type Parsed = OpObj;
    fn parse<'a>(
        _state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        unimplemented!()
    }
}

impl ZeroResultOp {
    fn new(ctx: &mut Context) -> ZeroResultOp {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        Self { op }
    }
}

// A simple test to trigger an interface verification error.
#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn check_intrf_verfiy_errs() {
    let ctx = &mut Context::new();

    let zero_res_op = ZeroResultOp::new(ctx).get_operation();
    let (module_op, _, _, ret_op) = const_ret_in_mod(ctx).unwrap();
    zero_res_op.insert_before(ctx, ret_op.get_operation());

    assert!(matches!(
        verify_op(&module_op, ctx),
        Err(Error {
            kind: ErrorKind::VerificationFailed,
            err,
            ..
        })
        if err.is::<NResultsVerifyErr>()
    ))
}

static TEST_OP_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new("".into()));

#[op_interface]
trait TestOpInterfaceX {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[op_interface_impl]
impl TestOpInterfaceX for ReturnOp {}
#[op_interface_impl]
impl TestOpInterfaceX for ModuleOp {}

#[op_interface]
trait TestOpInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT.lock().unwrap() += "TestOpInterface verified\n";
        Ok(())
    }
}

#[op_interface]
trait TestOpInterface2: TestOpInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT.lock().unwrap() += "TestOpInterface2 verified\n";
        Ok(())
    }
}

#[pliron_op(
    name = "test.verify_intr_op",
    format,
    interfaces = [TestOpInterface, TestOpInterface2],
    verifier = "succ",
)]
struct VerifyIntrOp {}
impl VerifyIntrOp {
    fn new(ctx: &mut Context) -> VerifyIntrOp {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        Self { op }
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_op_intr_verify_order() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrOp::new(ctx);

    verify_op(&vio, ctx)?;

    expect![[r#"
        TestOpInterface verified
        TestOpInterface2 verified
    "#]]
    .assert_eq(&TEST_OP_VERIFIERS_OUTPUT.lock().unwrap());

    // Ad-hoc op interface conversions test.
    let x = op_cast::<dyn TestOpInterface>(&vio).unwrap();
    any_to_trait::<dyn TestOpInterface2>(x.as_any()).unwrap();

    Ok(())
}

static TEST_OP_VERIFIERS_OUTPUT_GENERIC: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[op_interface]
trait TestOpInterfaceGeneric<T: Clone> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() += "TestOpInterfaceGeneric verified\n";
        Ok(())
    }
}

#[op_interface]
trait TestOpInterfaceGeneric2<T: Clone>: TestOpInterfaceGeneric<T> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() += "TestOpInterfaceGeneric2 verified\n";
        Ok(())
    }
}

#[op_interface]
trait TestOpInterfaceGeneric3<T: Clone>:
    TestOpInterfaceGeneric<T> + TestOpInterfaceGeneric2<T>
{
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() += "TestOpInterfaceGeneric3 verified\n";
        Ok(())
    }
}

#[pliron_op(
    name = "test.verify_intr_op_generic",
    format,
    interfaces = [TestOpInterfaceGeneric<i32>, TestOpInterfaceGeneric2<i32>, TestOpInterfaceGeneric3<i32>],
    verifier = "succ",
)]
struct VerifyIntrOpGeneric {}
impl VerifyIntrOpGeneric {
    fn new(ctx: &mut Context) -> VerifyIntrOpGeneric {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        Self { op }
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_op_intr_verify_order_generic() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrOpGeneric::new(ctx);
    verify_op(&vio, ctx)?;

    expect![[r#"
        TestOpInterfaceGeneric verified
        TestOpInterfaceGeneric2 verified
        TestOpInterfaceGeneric3 verified
    "#]]
    .assert_eq(&TEST_OP_VERIFIERS_OUTPUT_GENERIC.lock().unwrap());

    // Ad-hoc op interface conversions test.
    let x = op_cast::<dyn TestOpInterfaceGeneric<i32>>(&vio).unwrap();
    any_to_trait::<dyn TestOpInterfaceGeneric2<i32>>(x.as_any()).unwrap();

    Ok(())
}

static TEST_OP_VERIFIERS_OUTPUT_CONST_GENERIC: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[op_interface]
trait TestOpInterfaceConstGeneric<const N: usize> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT_CONST_GENERIC.lock().unwrap() +=
            "TestOpInterfaceConstGeneric verified\n";
        Ok(())
    }
}

#[op_interface]
trait TestOpInterfaceConstGeneric2<const N: usize>: TestOpInterfaceConstGeneric<N> {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT_CONST_GENERIC.lock().unwrap() +=
            "TestOpInterfaceConstGeneric2 verified\n";
        Ok(())
    }
}

#[op_interface]
trait TestOpInterfaceConstGeneric3<const N: usize>:
    TestOpInterfaceConstGeneric<N> + TestOpInterfaceConstGeneric2<N>
{
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_OP_VERIFIERS_OUTPUT_CONST_GENERIC.lock().unwrap() +=
            "TestOpInterfaceConstGeneric3 verified\n";
        Ok(())
    }
}

#[pliron_op(
    name = "test.verify_intr_op_const_generic",
    format,
    interfaces = [TestOpInterfaceConstGeneric<42>, TestOpInterfaceConstGeneric2<42>, TestOpInterfaceConstGeneric3<42>],
    verifier = "succ",
)]
struct VerifyIntrOpConstGeneric {}
impl VerifyIntrOpConstGeneric {
    fn new(ctx: &mut Context) -> VerifyIntrOpConstGeneric {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        Self { op }
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_op_intr_verify_order_const_generic() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrOpConstGeneric::new(ctx);
    vio.get_operation().deref(ctx).verify(ctx)?;

    expect![[r#"
        TestOpInterfaceConstGeneric verified
        TestOpInterfaceConstGeneric2 verified
        TestOpInterfaceConstGeneric3 verified
    "#]]
    .assert_eq(&TEST_OP_VERIFIERS_OUTPUT_CONST_GENERIC.lock().unwrap());

    // Ad-hoc op interface conversions test.
    let x = op_cast::<dyn TestOpInterfaceConstGeneric<42>>(&vio).unwrap();
    any_to_trait::<dyn TestOpInterfaceConstGeneric2<42>>(x.as_any()).unwrap();

    Ok(())
}

#[attr_interface]
trait TestAttrInterfaceX {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[attr_interface_impl]
impl TestAttrInterfaceX for StringAttr {}
#[attr_interface_impl]
impl TestAttrInterfaceX for IntegerAttr {}

#[pliron_attr(name = "test.my_attr", format = "`<` $ty `>`", verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct MyAttr {
    ty: Ptr<TypeObj>,
}

#[attr_interface_impl]
impl TypedAttrInterface for MyAttr {
    fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.ty
    }
}

static TEST_ATTR_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[pliron_attr(name = "test.verify_intr_attr", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct VerifyIntrAttr {}

#[attr_interface_impl]
impl TestAttrInterface for VerifyIntrAttr {}
#[attr_interface_impl]
impl TestAttrInterface2 for VerifyIntrAttr {}

#[attr_interface]
trait TestAttrInterface {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_VERIFIERS_OUTPUT.lock().unwrap() += "TestAttrInterface verified\n";
        Ok(())
    }
}

#[attr_interface]
trait TestAttrInterface2: TestAttrInterface {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_VERIFIERS_OUTPUT.lock().unwrap() += "TestAttrInterface2 verified\n";
        Ok(())
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_attr_intr_verify_order() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrAttr {};
    verify_attr(&vio, ctx)?;

    expect![[r#"
        TestAttrInterface verified
        TestAttrInterface2 verified
    "#]]
    .assert_eq(&TEST_ATTR_VERIFIERS_OUTPUT.lock().unwrap());

    // Ad-hoc attr interface conversions test.
    let x = attr_cast::<dyn TestAttrInterface>(&vio).unwrap();
    any_to_trait::<dyn TestAttrInterface2>(x.as_any()).unwrap();

    Ok(())
}

static TEST_ATTR_VERIFIERS_OUTPUT_GENERIC: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[attr_interface]
trait TestAttrInterfaceGeneric<T: Clone> {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() +=
            "TestAttrInterfaceGeneric verified\n";
        Ok(())
    }
}

#[attr_interface]
trait TestAttrInterfaceGeneric2<T: Clone>: TestAttrInterfaceGeneric<T> {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() +=
            "TestAttrInterfaceGeneric2 verified\n";
        Ok(())
    }
}

#[attr_interface]
trait TestAttrInterfaceGeneric3<T: Clone>:
    TestAttrInterfaceGeneric<T> + TestAttrInterfaceGeneric2<T>
{
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() +=
            "TestAttrInterfaceGeneric3 verified\n";
        Ok(())
    }
}

#[pliron_attr(name = "test.verify_intr_attr_generic", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct VerifyIntrAttrGeneric {}

#[attr_interface_impl]
impl TestAttrInterfaceGeneric<i32> for VerifyIntrAttrGeneric {}
#[attr_interface_impl]
impl TestAttrInterfaceGeneric2<i32> for VerifyIntrAttrGeneric {}
#[attr_interface_impl]
impl TestAttrInterfaceGeneric3<i32> for VerifyIntrAttrGeneric {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_attr_intr_verify_order_generic() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrAttrGeneric {};
    verify_attr(&vio, ctx)?;

    expect![[r#"
        TestAttrInterfaceGeneric verified
        TestAttrInterfaceGeneric2 verified
        TestAttrInterfaceGeneric3 verified
    "#]]
    .assert_eq(&TEST_ATTR_VERIFIERS_OUTPUT_GENERIC.lock().unwrap());

    // Ad-hoc attr interface conversions test.
    let x = attr_cast::<dyn TestAttrInterfaceGeneric<i32>>(&vio).unwrap();
    any_to_trait::<dyn TestAttrInterfaceGeneric2<i32>>(x.as_any()).unwrap();

    Ok(())
}

static TEST_ATTR_ENUM_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[attr_interface]
trait TestAttrEnumInterface {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_ENUM_VERIFIERS_OUTPUT.lock().unwrap() += "TestAttrEnumInterface verified\n";
        Ok(())
    }
}

#[pliron_attr(name = "test.verify_intr_attr_enum", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
enum VerifyIntrAttrEnum {
    Unit,
    Payload(i32),
}

#[attr_interface_impl]
impl TestAttrEnumInterface for VerifyIntrAttrEnum {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_attr_intr_verify_order_enum() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrAttrEnum::Payload(7);
    verify_attr(&vio, ctx)?;

    expect![[r#"
        TestAttrEnumInterface verified
    "#]]
    .assert_eq(&TEST_ATTR_ENUM_VERIFIERS_OUTPUT.lock().unwrap());

    let _ = attr_cast::<dyn TestAttrEnumInterface>(&vio).unwrap();

    Ok(())
}

static TEST_ATTR_ENUM_GENERIC_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[attr_interface]
trait TestAttrEnumGenericInterface<T: Clone> {
    fn verify(_op: &dyn Attribute, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_ATTR_ENUM_GENERIC_VERIFIERS_OUTPUT.lock().unwrap() += &format!(
            "TestAttrEnumGenericInterface<{}> verified\n",
            core::any::type_name::<T>()
        );
        Ok(())
    }
}

#[pliron_attr(name = "test.verify_intr_attr_enum_generic", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
enum VerifyIntrAttrEnumGeneric {
    Unit,
}

#[attr_interface_impl]
impl TestAttrEnumGenericInterface<i64> for VerifyIntrAttrEnumGeneric {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_attr_intr_verify_order_enum_generic() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrAttrEnumGeneric::Unit;
    verify_attr(&vio, ctx)?;

    expect![[r#"
        TestAttrEnumGenericInterface<i64> verified
    "#]]
    .assert_eq(&TEST_ATTR_ENUM_GENERIC_VERIFIERS_OUTPUT.lock().unwrap());

    let _ = attr_cast::<dyn TestAttrEnumGenericInterface<i64>>(&vio).unwrap();

    Ok(())
}

#[type_interface]
trait TestTypeInterfaceX {
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}

#[type_interface_impl]
impl TestTypeInterfaceX for UnitType {}
#[type_interface_impl]
impl TestTypeInterfaceX for IntegerType {}

static TEST_TYPE_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[pliron_type(name = "test.verify_intr_type", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct VerifyIntrType {}
#[type_interface_impl]
impl TestTypeInterface for VerifyIntrType {}
#[type_interface_impl]
impl TestTypeInterface2 for VerifyIntrType {}

#[type_interface]
trait TestTypeInterface {
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_VERIFIERS_OUTPUT.lock().unwrap() += "TestTypeInterface verified\n";
        Ok(())
    }
}

#[type_interface]
trait TestTypeInterface2: TestTypeInterface {
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_VERIFIERS_OUTPUT.lock().unwrap() += "TestTypeInterface2 verified\n";
        Ok(())
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_type_intr_verify_order() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrType {};
    verify_type(&vio, ctx)?;

    expect![[r#"
        TestTypeInterface verified
        TestTypeInterface2 verified
    "#]]
    .assert_eq(&TEST_TYPE_VERIFIERS_OUTPUT.lock().unwrap());

    // Ad-hoc type interface conversions test.
    let x = type_cast::<dyn TestTypeInterface>(&vio).unwrap();
    any_to_trait::<dyn TestTypeInterface2>(x.as_any()).unwrap();

    Ok(())
}

static TEST_TYPE_VERIFIERS_OUTPUT_GENERIC: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[type_interface]
trait TestTypeInterfaceGeneric<T: Clone> {
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() += &format!(
            "TestTypeInterfaceGeneric<{}> verified\n",
            core::any::type_name::<T>()
        );
        Ok(())
    }
}

#[type_interface]
trait TestTypeInterfaceGeneric2<T: Clone>: TestTypeInterfaceGeneric<T> {
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() += &format!(
            "TestTypeInterfaceGeneric2<{}> verified\n",
            core::any::type_name::<T>()
        );
        Ok(())
    }
}

#[type_interface]
trait TestTypeInterfaceGeneric3<T: Clone>:
    TestTypeInterfaceGeneric<T> + TestTypeInterfaceGeneric2<T>
{
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_VERIFIERS_OUTPUT_GENERIC.lock().unwrap() += &format!(
            "TestTypeInterfaceGeneric3<{}> verified\n",
            core::any::type_name::<T>()
        );
        Ok(())
    }
}

#[pliron_type(name = "test.verify_intr_type_generic", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
struct VerifyIntrTypeGeneric {}

#[type_interface_impl]
impl TestTypeInterfaceGeneric<i32> for VerifyIntrTypeGeneric {}
#[type_interface_impl]
impl TestTypeInterfaceGeneric2<i32> for VerifyIntrTypeGeneric {}
#[type_interface_impl]
impl TestTypeInterfaceGeneric3<i32> for VerifyIntrTypeGeneric {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_type_intr_verify_order_generic() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrTypeGeneric {};
    verify_type(&vio, ctx)?;

    expect![[r#"
        TestTypeInterfaceGeneric<i32> verified
        TestTypeInterfaceGeneric2<i32> verified
        TestTypeInterfaceGeneric3<i32> verified
    "#]]
    .assert_eq(&TEST_TYPE_VERIFIERS_OUTPUT_GENERIC.lock().unwrap());

    // Ad-hoc type interface conversions test.
    let x = type_cast::<dyn TestTypeInterfaceGeneric<i32>>(&vio).unwrap();
    any_to_trait::<dyn TestTypeInterfaceGeneric2<i32>>(x.as_any()).unwrap();

    Ok(())
}

static TEST_TYPE_ENUM_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[type_interface]
trait TestTypeEnumInterface {
    fn verify(_ty: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_ENUM_VERIFIERS_OUTPUT.lock().unwrap() += "TestTypeEnumInterface verified\n";
        Ok(())
    }
}

#[pliron_type(name = "test.verify_intr_type_enum", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
enum VerifyIntrTypeEnum {
    Unit,
    Payload(i32),
}

#[type_interface_impl]
impl TestTypeEnumInterface for VerifyIntrTypeEnum {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_type_intr_verify_order_enum() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrTypeEnum::Payload(7);
    verify_type(&vio, ctx)?;

    expect![[r#"
        TestTypeEnumInterface verified
    "#]]
    .assert_eq(&TEST_TYPE_ENUM_VERIFIERS_OUTPUT.lock().unwrap());

    let _ = type_cast::<dyn TestTypeEnumInterface>(&vio).unwrap();

    Ok(())
}

static TEST_TYPE_ENUM_GENERIC_VERIFIERS_OUTPUT: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("".into()));

#[type_interface]
trait TestTypeEnumGenericInterface<T: Clone> {
    fn verify(_op: &dyn Type, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        *TEST_TYPE_ENUM_GENERIC_VERIFIERS_OUTPUT.lock().unwrap() += &format!(
            "TestTypeEnumGenericInterface<{}> verified\n",
            core::any::type_name::<T>()
        );
        Ok(())
    }
}

#[pliron_type(name = "test.verify_intr_type_enum_generic", format, verifier = "succ")]
#[derive(PartialEq, Clone, Debug, Hash)]
enum VerifyIntrTypeEnumGeneric {
    Unit,
}

#[type_interface_impl]
impl TestTypeEnumGenericInterface<i64> for VerifyIntrTypeEnumGeneric {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_type_intr_verify_order_enum_generic() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = VerifyIntrTypeEnumGeneric::Unit;
    verify_type(&vio, ctx)?;

    expect![[r#"
        TestTypeEnumGenericInterface<i64> verified
    "#]]
    .assert_eq(&TEST_TYPE_ENUM_GENERIC_VERIFIERS_OUTPUT.lock().unwrap());

    let _ = type_cast::<dyn TestTypeEnumGenericInterface<i64>>(&vio).unwrap();

    Ok(())
}

#[op_interface]
trait TestNoInbuiltVerifyInterface {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized;
}

#[pliron_op(name = "test.no_inbuilt_verify_op", format, verifier = "succ")]
struct NoInbuiltVerifyOp {}
impl NoInbuiltVerifyOp {
    fn new(ctx: &mut Context) -> NoInbuiltVerifyOp {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        Self { op }
    }
}

#[op_interface_impl]
impl TestNoInbuiltVerifyInterface for NoInbuiltVerifyOp {
    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_no_inbuilt_verify() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = NoInbuiltVerifyOp::new(ctx);

    verify_op(&vio, ctx)?;

    Ok(())
}

#[pliron_op(name = "test.no_inbuilt_verify_op2", format, verifier = "succ")]
struct NoInbuiltVerifyOp2 {}
impl NoInbuiltVerifyOp2 {
    fn new(ctx: &mut Context) -> NoInbuiltVerifyOp2 {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 1);
        Self { op }
    }
}

#[derive(Error, Debug)]
#[error("No inbuilt verify op2 error")]
pub struct NoInbuiltVerifyOp2Error;

#[op_interface_impl]
impl TestNoInbuiltVerifyInterface for NoInbuiltVerifyOp2 {
    fn verify(op: &dyn Op, ctx: &Context) -> Result<()> {
        verify_err!(op.loc(ctx), NoInbuiltVerifyOp2Error)
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_no_inbuilt_verify2() -> Result<()> {
    let ctx = &mut Context::new();

    let vio = NoInbuiltVerifyOp2::new(ctx);

    assert!(matches!(
        verify_op(&vio, ctx),
        Err(Error {
            kind: ErrorKind::VerificationFailed,
            err,
            ..
        })
        if err.is::<NoInbuiltVerifyOp2Error>()
    ));

    Ok(())
}

#[pliron_attr(
    name = "test.outline_test_attr",
    format = "`<` $ty `>`",
    verifier = "succ"
)]
#[derive(PartialEq, Clone, Debug, Hash)]
pub struct OutlineTestAttr {
    pub ty: Ptr<TypeObj>,
}

#[attr_interface_impl]
impl OutlinedAttr for OutlineTestAttr {}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_outline_attr() -> Result<()> {
    let ctx = &mut Context::new();
    let attr = OutlineTestAttr {
        ty: IntegerType::get(ctx, 32, pliron::builtin::types::Signedness::Signed).into(),
    };

    let op = ConstantOp::new(ctx, 42);
    op.get_operation()
        .deref_mut(ctx)
        .attributes
        .0
        .insert("test_attr".try_into().unwrap(), Box::new(attr));

    // Print the op
    let printed = op.get_operation().deref(ctx).disp(ctx).to_string();

    expect![[r#"
        v0 = test.constant builtin.integer <42: si64> !0

        outlined_attributes:
        !0 = [test_attr = test.outline_test_attr <builtin.integer si32>]
    "#]]
    .assert_eq(&printed);

    Ok(())
}

#[pliron_attr(
    name = "test.outline_print_once_test_attr",
    format = "`<` $ty `>`",
    verifier = "succ"
)]
#[derive(PartialEq, Clone, Debug, Hash)]
pub struct OulinePrintOnceTestAttr {
    pub ty: Ptr<TypeObj>,
}

#[attr_interface_impl]
impl OutlinedAttr for OulinePrintOnceTestAttr {}
#[attr_interface_impl]
impl PrintOnceAttr for OulinePrintOnceTestAttr {
    fn hash_attr(&self) -> TypeValueHash {
        TypeValueHash::new(self)
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_outline_printonce_attr() -> Result<()> {
    let ctx = &mut Context::new();

    let attr = OulinePrintOnceTestAttr {
        ty: IntegerType::get(ctx, 32, pliron::builtin::types::Signedness::Signed).into(),
    };

    let src = Source::new_from_file(ctx, "/tmp/test.pliron");

    let pos1 = SourcePosition::default();
    let loc1 = Location::SrcPos { src, pos: pos1 };

    let op42 = ConstantOp::new(ctx, 42);
    op42.get_operation().deref_mut(ctx).set_loc(loc1);
    let op44 = ConstantOp::new(ctx, 44);
    op42.get_operation().deref_mut(ctx).attributes.0.insert(
        "test_print_once_attr".try_into().unwrap(),
        Box::new(attr.clone()),
    );
    op44.get_operation()
        .deref_mut(ctx)
        .attributes
        .0
        .insert("test_print_once_attr".try_into().unwrap(), Box::new(attr));

    // Put them both in a function.
    let (module_op, _, _, ret_op) = const_ret_in_mod(ctx).unwrap();
    op42.get_operation()
        .insert_before(ctx, ret_op.get_operation());
    op44.get_operation()
        .insert_before(ctx, ret_op.get_operation());

    // Print the op
    let printed = module_op.get_operation().deref(ctx).disp(ctx).to_string();

    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1():
                c0_v2 = test.constant builtin.integer <0: si64> !0;
                v0 = test.constant builtin.integer <42: si64> !1;
                v1 = test.constant builtin.integer <44: si64> !2;
                test.return c0_v2
            }
        }

        outlined_attributes:
        !0 = [builtin_debug_info = builtin.debug_info [c0]]
        !1 = @["/tmp/test.pliron": line: 1, column: 1], [test_print_once_attr = !3]
        !2 = [test_print_once_attr = !3]
        !3 = test.outline_print_once_test_attr <builtin.integer si32>
    "#]]
    .assert_eq(&printed);

    // Try parsing the just printed output.

    let parsed_op = {
        let mut state_stream = state_stream_from_iterator(
            printed.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        Operation::top_level_parser()
            .parse_stream(&mut state_stream)
            .into_result()
            .map_err(|err| {
                let err = err.into_inner();
                let pos = err.error.position;
                input_error!(Location::SrcPos { src, pos }, err.error)
            })
    };

    let parsed_op = parsed_op.expect_ok(ctx).0;

    verify_operation(parsed_op, ctx)?;
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1_block4v1() !0:
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1_block3v1() !1:
                c0_v3 = test.constant builtin.integer <0: si64> !2;
                v0_v4 = test.constant builtin.integer <42: si64> !3;
                v1_v5 = test.constant builtin.integer <44: si64> !4;
                test.return c0_v3 !5
            } !6
        } !7

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 3], []
        !1 = @[<in-memory>: line: 6, column: 7], []
        !2 = @[<in-memory>: line: 7, column: 9], [builtin_debug_info = builtin.debug_info [c0]]
        !3 = @["/tmp/test.pliron": line: 1, column: 1], [builtin_debug_info = builtin.debug_info [v0], test_print_once_attr = !8]
        !4 = @[<in-memory>: line: 9, column: 9], [builtin_debug_info = builtin.debug_info [v1], test_print_once_attr = !8]
        !5 = @[<in-memory>: line: 10, column: 9], []
        !6 = @[<in-memory>: line: 4, column: 5], []
        !7 = @[<in-memory>: line: 1, column: 1], []
        !8 = test.outline_print_once_test_attr <builtin.integer si32>
    "#]]
    .assert_eq(&parsed_op.disp(ctx).to_string());

    Ok(())
}

#[pliron_op(name = "test.canonical_op", format, verifier = "succ")]
pub struct CanonicalOp;

impl CanonicalOp {
    pub fn new(ctx: &mut Context) -> CanonicalOp {
        let op = Operation::new(ctx, Self::get_concrete_op_info(), vec![], vec![], vec![], 0);
        Self { op }
    }
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_outline_attr_canonical_op() -> Result<()> {
    let ctx = &mut Context::new();

    let attr = OutlineTestAttr {
        ty: IntegerType::get(ctx, 32, pliron::builtin::types::Signedness::Signed).into(),
    };

    let op = CanonicalOp::new(ctx);
    op.get_operation()
        .deref_mut(ctx)
        .attributes
        .0
        .insert("test_attr".try_into().unwrap(), Box::new(attr));

    // Print the op
    let printed = op.get_operation().deref(ctx).disp(ctx).to_string();

    expect![[r#"
        test.canonical_op () [] []: <() -> ()> !0

        outlined_attributes:
        !0 = [test_attr = test.outline_test_attr <builtin.integer si32>]
    "#]]
    .assert_eq(&printed);

    Ok(())
}

#[test]
#[cfg_attr(target_family = "wasm", wasm_bindgen_test)]
fn test_outline_attr_on_block() -> Result<()> {
    // Verify that OutlinedAttr on a BasicBlock is printed in the outlined_attributes section,
    // not inline, and is correctly restored on re-parse.
    use pliron::irfmt::parsers::spaced;

    let ctx = &mut Context::new();
    let (module_op, func_op, _const_op, _ret_op) = const_ret_in_mod(ctx)?;

    let attr = OutlineTestAttr {
        ty: IntegerType::get(ctx, 32, pliron::builtin::types::Signedness::Signed).into(),
    };

    // Set the OutlinedAttr on the function's entry block.
    let entry_block = func_op.get_entry_block(ctx);
    entry_block
        .deref_mut(ctx)
        .attributes
        .0
        .insert("test_attr".try_into().unwrap(), Box::new(attr));

    // The outlined attr should NOT appear inline; the block gets an outline index.
    let printed = module_op.get_operation().deref(ctx).disp(ctx).to_string();
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1():
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1() !0:
                c0_v0 = test.constant builtin.integer <0: si64> !1;
                test.return c0_v0
            }
        }

        outlined_attributes:
        !0 = [test_attr = test.outline_test_attr <builtin.integer si32>]
        !1 = [builtin_debug_info = builtin.debug_info [c0]]
    "#]]
    .assert_eq(&printed);

    // Parse the printed IR back and verify the outlined attr is restored on the block.
    let parsed_op = {
        let state_stream = state_stream_from_iterator(
            printed.chars(),
            parsable::State::new(ctx, location::Source::InMemory),
        );
        spaced(Operation::top_level_parser())
            .parse(state_stream)
            .unwrap()
            .0
    };
    verify_operation(parsed_op, ctx)?;

    // After re-parse the block now has a source location too, so indices shift.
    let print2 = parsed_op.deref(ctx).disp(ctx).to_string();
    expect![[r#"
        builtin.module @bar 
        {
          ^block1v1_block4v1() !0:
            builtin.func @foo: builtin.function <()->(builtin.integer si64)> 
            {
              ^entry_block2v1_block3v1() !1:
                c0_v1 = test.constant builtin.integer <0: si64> !2;
                test.return c0_v1 !3
            } !4
        } !5

        outlined_attributes:
        !0 = @[<in-memory>: line: 3, column: 3], []
        !1 = @[<in-memory>: line: 6, column: 7], [test_attr = test.outline_test_attr <builtin.integer si32>]
        !2 = @[<in-memory>: line: 7, column: 9], [builtin_debug_info = builtin.debug_info [c0]]
        !3 = @[<in-memory>: line: 8, column: 9], []
        !4 = @[<in-memory>: line: 4, column: 5], []
        !5 = @[<in-memory>: line: 1, column: 1], []
    "#]]
    .assert_eq(&print2);

    Ok(())
}
