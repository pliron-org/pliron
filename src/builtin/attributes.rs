use core::hash::{Hash, Hasher};

use alloc::{boxed::Box, string::String, vec::Vec};
use pliron::derive::{attr_interface_impl, pliron_attr};
use rustc_apfloat::Float;
use thiserror::Error;

use crate::{
    attribute::{AttrObj, AttributeDict},
    builtin::{
        attr_interfaces::FloatAttr,
        types::{FP16Type, FP32Type, FP64Type},
    },
    combine::{
        Parser, between, many1,
        parser::char::{char, digit, spaces},
        token,
    },
    common_traits::Verify,
    context::{Context, Ptr},
    identifier::Identifier,
    input_err,
    irfmt::{
        parsers::{quoted_string_parser, spaced},
        printers::quoted,
    },
    location::Located,
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    result::Result,
    r#type::{TypeObj, TypePtr, Typed},
    utils::{
        apfloat::{self, double_to_f64, f32_to_single, f64_to_double, single_to_f32},
        apint::APInt,
    },
    verify_err_noloc,
};

use super::{
    attr_interfaces::{MaterializableAttr, TypedAttrInterface},
    ops::ConstantOp,
    types::{IntegerType, Signedness},
};
use crate::{op::Op, operation::Operation};

#[pliron_attr(name = "builtin.identifier", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct IdentifierAttr(Identifier);

impl IdentifierAttr {
    /// Create a new [IdentifierAttr]
    pub fn new(value: Identifier) -> Self {
        IdentifierAttr(value)
    }
}

impl From<IdentifierAttr> for Identifier {
    fn from(value: IdentifierAttr) -> Self {
        value.0
    }
}

/// An attribute containing a string.
/// Similar to MLIR's [StringAttr](https://mlir.llvm.org/docs/Dialects/Builtin/#stringattr).
#[pliron_attr(name = "builtin.string", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct StringAttr(String);

impl StringAttr {
    /// Create a new [StringAttr].
    pub fn new(value: String) -> Self {
        StringAttr(value)
    }
}

impl From<String> for StringAttr {
    fn from(value: String) -> Self {
        StringAttr::new(value)
    }
}

impl From<StringAttr> for String {
    fn from(value: StringAttr) -> Self {
        value.0
    }
}

impl Printable for StringAttr {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        quoted(&self.0).fmt(ctx, state, f)
    }
}

impl Parsable for StringAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        quoted_string_parser()
            .map(StringAttr)
            .parse_stream(state_stream)
            .into_result()
    }
}

/// A boolean attribute
#[pliron_attr(name = "builtin.bool", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct BoolAttr(bool);

impl BoolAttr {
    /// Create a new [BoolAttr].
    pub fn new(value: bool) -> Self {
        BoolAttr(value)
    }
}

impl From<BoolAttr> for bool {
    fn from(value: BoolAttr) -> Self {
        value.0
    }
}

impl From<bool> for BoolAttr {
    fn from(value: bool) -> Self {
        BoolAttr::new(value)
    }
}

/// An attribute containing an integer.
/// Similar to MLIR's [IntegerAttr](https://mlir.llvm.org/docs/Dialects/Builtin/#integerattr).
#[pliron_attr(name = "builtin.integer")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct IntegerAttr {
    ty: TypePtr<IntegerType>,
    val: APInt,
}

impl Printable for IntegerAttr {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let ty = &*self.ty.deref(ctx);
        write!(
            f,
            "<{}: {}>",
            self.val
                .to_string_decimal(ty.signedness() == Signedness::Signed),
            ty.disp(ctx)
        )
    }
}

#[derive(Debug, Error)]
#[error("The bitwidth type does not match the bitwidth of the value.")]
pub struct IntegerAttrBitwidthErr;

impl Verify for IntegerAttr {
    fn verify(&self, ctx: &Context) -> Result<()> {
        if self.ty.deref(ctx).width() as usize != self.val.bw() {
            return verify_err_noloc!(IntegerAttrBitwidthErr);
        }
        Ok(())
    }
}

impl IntegerAttr {
    /// Create a new [IntegerAttr].
    pub fn new(ty: TypePtr<IntegerType>, val: APInt) -> Self {
        IntegerAttr { ty, val }
    }

    /// Get the value of the attribute.
    pub fn value(&self) -> APInt {
        self.val.clone()
    }

    /// Get the type of the attribute.
    pub fn get_type(&self) -> TypePtr<IntegerType> {
        self.ty
    }
}

impl From<IntegerAttr> for APInt {
    fn from(value: IntegerAttr) -> Self {
        value.val
    }
}

impl Parsable for IntegerAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        between(
            token('<'),
            token('>'),
            spaces()
                .with(many1::<String, _, _>(digit().or(char('-').or(char('+')))))
                .skip(spaced(token(':')))
                .and(IntegerType::parser(())),
        )
        .then(|(digits, ty)| {
            combine::parser(move |state_stream: &mut StateStream<'a>| {
                let ty_ref = &*ty.deref(state_stream.state.ctx);
                let apint = match APInt::from_str(&digits, ty_ref.width() as usize, 10) {
                    Ok(val) => Ok(val).into_parse_result(),
                    Err(err) => input_err!(state_stream.loc(), "{}", err).into_parse_result(),
                }?;
                Ok(IntegerAttr::new(ty, apint.0)).into_parse_result()
            })
        })
        .parse_stream(state_stream)
        .into_result()
    }
}

impl Typed for IntegerAttr {
    fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.ty.into()
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for IntegerAttr {
    fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.ty.into()
    }
}

#[attr_interface_impl]
impl MaterializableAttr for IntegerAttr {
    fn materialize(&self, ctx: &mut Context) -> Ptr<Operation> {
        let const_op = ConstantOp::new(ctx, Box::new(self.clone()));
        const_op.get_operation()
    }
}

#[pliron_attr(name = "builtin.half", format = "$0", verifier = "succ")]
#[derive(PartialEq, Clone, Debug)]
/// An attribute that is a half-precision (16-bit) floating-point number.
pub struct FPHalfAttr(pub apfloat::Half);

impl Hash for FPHalfAttr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for FPHalfAttr {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        FP16Type::get(ctx).into()
    }
}

#[attr_interface_impl]
impl FloatAttr for FPHalfAttr {
    fn get_inner(&self) -> &dyn apfloat::DynFloat {
        &self.0
    }

    fn build_from(&self, df: Box<dyn apfloat::DynFloat>) -> Box<dyn FloatAttr> {
        let df = df
            .downcast::<apfloat::Half>()
            .expect("Expected a Half precision float");
        Box::new(FPHalfAttr(*df))
    }

    fn get_semantics(&self) -> apfloat::Semantics {
        Self::get_semantics_static()
    }

    fn get_semantics_static() -> apfloat::Semantics
    where
        Self: Sized,
    {
        <apfloat::Half as apfloat::GetSemantics>::get_semantics()
    }
}

#[pliron_attr(name = "builtin.single", format = "$0", verifier = "succ")]
#[derive(PartialEq, Clone, Debug)]
/// An attribute that is a single-precision floating-point number.
pub struct FPSingleAttr(pub apfloat::Single);

impl From<f32> for FPSingleAttr {
    fn from(value: f32) -> Self {
        FPSingleAttr(f32_to_single(value))
    }
}

impl From<FPSingleAttr> for f32 {
    fn from(value: FPSingleAttr) -> Self {
        single_to_f32(value.0)
    }
}

impl Hash for FPSingleAttr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for FPSingleAttr {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        FP32Type::get(ctx).into()
    }
}

#[attr_interface_impl]
impl FloatAttr for FPSingleAttr {
    fn get_inner(&self) -> &dyn apfloat::DynFloat {
        &self.0
    }

    fn build_from(&self, df: Box<dyn apfloat::DynFloat>) -> Box<dyn FloatAttr> {
        let df = df
            .downcast::<apfloat::Single>()
            .expect("Expected a Single precision float");
        Box::new(FPSingleAttr(*df))
    }

    fn get_semantics(&self) -> apfloat::Semantics {
        Self::get_semantics_static()
    }

    fn get_semantics_static() -> apfloat::Semantics
    where
        Self: Sized,
    {
        <apfloat::Single as apfloat::GetSemantics>::get_semantics()
    }
}

#[pliron_attr(name = "builtin.double", format = "$0", verifier = "succ")]
#[derive(PartialEq, Clone, Debug)]
/// An attribute that is a double-precision floating-point number.
pub struct FPDoubleAttr(pub apfloat::Double);

impl From<f64> for FPDoubleAttr {
    fn from(value: f64) -> Self {
        FPDoubleAttr(f64_to_double(value))
    }
}

impl From<FPDoubleAttr> for f64 {
    fn from(value: FPDoubleAttr) -> Self {
        double_to_f64(value.0)
    }
}

impl Hash for FPDoubleAttr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for FPDoubleAttr {
    fn get_type(&self, ctx: &Context) -> Ptr<TypeObj> {
        FP64Type::get(ctx).into()
    }
}

#[attr_interface_impl]
impl FloatAttr for FPDoubleAttr {
    fn get_inner(&self) -> &dyn apfloat::DynFloat {
        &self.0
    }

    fn build_from(&self, df: Box<dyn apfloat::DynFloat>) -> Box<dyn FloatAttr> {
        let df = df
            .downcast::<apfloat::Double>()
            .expect("Expected a Double precision float");
        Box::new(FPDoubleAttr(*df))
    }

    fn get_semantics(&self) -> apfloat::Semantics {
        Self::get_semantics_static()
    }

    fn get_semantics_static() -> apfloat::Semantics
    where
        Self: Sized,
    {
        <apfloat::Double as apfloat::GetSemantics>::get_semantics()
    }
}

/// An attribute that is a dictionary of other attributes.
/// Similar to MLIR's [DictionaryAttr](https://mlir.llvm.org/docs/Dialects/Builtin/#dictionaryattr),
#[pliron_attr(name = "builtin.dict", verifier = "succ")]
#[derive(PartialEq, Clone, Eq, Debug)]
pub struct DictAttr(AttributeDict);

impl Printable for DictAttr {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{}", self.0.disp(ctx))
    }
}

impl Parsable for DictAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        _state_stream: &mut StateStream<'a>,
        _argg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        AttributeDict::parser(())
            .map(DictAttr)
            .parse_stream(_state_stream)
            .into_result()
    }
}

impl DictAttr {
    /// Create a new [DictAttr].
    pub fn new(value: Vec<(Identifier, AttrObj)>) -> Self {
        let mut dict = AttributeDict::default();
        for (name, val) in value {
            dict.0.insert(name, val);
        }
        DictAttr(dict)
    }

    /// Add an entry to the dictionary.
    pub fn insert(&mut self, key: &Identifier, val: AttrObj) {
        self.0.0.insert(key.clone(), val);
    }

    /// Remove an entry from the dictionary.
    pub fn remove(&mut self, key: &Identifier) {
        self.0.0.remove(key);
    }

    /// Lookup a name in the dictionary.
    pub fn lookup<'a>(&'a self, key: &Identifier) -> Option<&'a AttrObj> {
        self.0.0.get(key)
    }

    /// Lookup a name in the dictionary, get a mutable reference.
    pub fn lookup_mut<'a>(&'a mut self, key: &Identifier) -> Option<&'a mut AttrObj> {
        self.0.0.get_mut(key)
    }
}

/// A vector of other attributes.
#[pliron_attr(name = "builtin.vec", format = "`[` vec($0, CharSpace(`,`)) `]`")]
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct VecAttr(pub Vec<AttrObj>);

impl VecAttr {
    pub fn new(value: Vec<AttrObj>) -> Self {
        VecAttr(value)
    }
}

impl Verify for VecAttr {
    fn verify(&self, ctx: &Context) -> Result<()> {
        self.0.iter().try_for_each(|elm| elm.verify(ctx))
    }
}

/// Represent attributes that only have meaning from their existence.
/// See [UnitAttr](https://mlir.llvm.org/docs/Dialects/Builtin/#unitattr) in MLIR.
#[pliron_attr(name = "builtin.unit", format, verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Copy, Debug, Default, Hash)]
pub struct UnitAttr;

impl UnitAttr {
    pub fn new() -> Self {
        UnitAttr
    }
}

/// An attribute that does nothing but hold a Type.
/// Same as MLIR's [TypeAttr](https://mlir.llvm.org/docs/Dialects/Builtin/#typeattr).
#[pliron_attr(name = "builtin.type", format = "$0", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct TypeAttr(Ptr<TypeObj>);

impl TypeAttr {
    pub fn new(ty: Ptr<TypeObj>) -> Self {
        TypeAttr(ty)
    }
}

impl Typed for TypeAttr {
    fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.0
    }
}

#[attr_interface_impl]
impl TypedAttrInterface for TypeAttr {
    fn get_type(&self, _ctx: &Context) -> Ptr<TypeObj> {
        self.0
    }
}

#[pliron_attr(
    name = "builtin.operand_segment_sizes",
    format = "`[` vec($0, CharSpace(`,`)) `]`",
    verifier = "succ"
)]
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct OperandSegmentSizesAttr(pub Vec<u32>);

#[cfg(test)]
mod tests {
    use alloc::{format, string::ToString, vec};

    use awint::bw;
    use expect_test::expect;

    use crate::{
        attribute::{AttrObj, attr_cast},
        builtin::{
            attr_interfaces::TypedAttrInterface,
            attributes::{
                FPDoubleAttr, FPSingleAttr, IntegerAttr, OperandSegmentSizesAttr, StringAttr,
            },
            types::{IntegerType, Signedness},
        },
        context::Context,
        identifier::Identifier,
        irfmt::parsers::attr_parser,
        location,
        parsable::{self, state_stream_from_iterator},
        printable::Printable,
        utils::apint::APInt,
    };

    use super::*;

    #[test]
    fn test_integer_attributes() {
        let mut ctx = Context::new();

        let i64_ty = IntegerType::get(&mut ctx, 64, Signedness::Signed);

        let int64_0_ptr: AttrObj = IntegerAttr::new(i64_ty, APInt::from_i64(0, bw(64))).into();
        let int64_1_ptr: AttrObj = IntegerAttr::new(i64_ty, APInt::from_i64(15, bw(64))).into();
        assert!(int64_0_ptr.is::<IntegerAttr>() && int64_0_ptr.ne(&int64_1_ptr));
        let int64_0_ptr2: AttrObj = IntegerAttr::new(i64_ty, APInt::from_i64(0, bw(64))).into();
        assert!(int64_0_ptr == int64_0_ptr2);
        assert_eq!(
            int64_0_ptr.disp(&ctx).to_string(),
            "builtin.integer <0: si64>"
        );
        assert_eq!(
            int64_1_ptr.disp(&ctx).to_string(),
            "builtin.integer <15: si64>"
        );
        assert!(
            APInt::from(int64_0_ptr.downcast_ref::<IntegerAttr>().unwrap().clone()).is_zero()
                && APInt::to_i64(&APInt::from(
                    int64_1_ptr.downcast_ref::<IntegerAttr>().unwrap().clone()
                )) == 15
        );

        let attr_input = "builtin.integer <0: builtin.unit>";
        let state_stream = state_stream_from_iterator(
            attr_input.chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let parse_err = attr_parser()
            .parse(state_stream)
            .err()
            .expect("Integer attribute with non-integer type shouldn't be parsed successfully");
        let expected_err_msg = expect![[r#"
            Parse error at line: 1, column: 21
            Unexpected `b`
            Expected whitespaces, si, ui, i or whitespace
        "#]];
        expected_err_msg.assert_eq(&parse_err.to_string());
    }

    #[test]
    fn test_string_attributes() {
        let mut ctx = Context::new();

        let str_0_ptr: AttrObj = StringAttr::new("hello".to_string()).into();
        let str_1_ptr: AttrObj = StringAttr::new("world".to_string()).into();
        assert!(str_0_ptr.is::<StringAttr>() && str_0_ptr.ne(&str_1_ptr));
        let str_0_ptr2: AttrObj = StringAttr::new("hello".to_string()).into();
        assert!(str_0_ptr == str_0_ptr2);
        assert_eq!(str_0_ptr.disp(&ctx).to_string(), "builtin.string \"hello\"");
        assert_eq!(str_1_ptr.disp(&ctx).to_string(), "builtin.string \"world\"");
        assert_eq!(
            String::from(str_0_ptr.downcast_ref::<StringAttr>().unwrap().clone()),
            "hello",
        );
        assert_eq!(
            String::from(str_1_ptr.downcast_ref::<StringAttr>().unwrap().clone()),
            "world"
        );

        let attr_input = "builtin.string \"hello\"";
        let state_stream = state_stream_from_iterator(
            attr_input.chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let attr = attr_parser().parse(state_stream).unwrap().0;
        assert_eq!(attr.disp(&ctx).to_string(), attr_input);

        let attr_input = "builtin.string \"hello \\\"world\\\"\"";
        let state_stream = state_stream_from_iterator(
            attr_input.chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let attr_parsed = attr_parser().parse(state_stream).unwrap().0;
        assert_eq!(attr_parsed.disp(&ctx).to_string(), attr_input,);

        // Unsupported escaped character.
        let state_stream = state_stream_from_iterator(
            "builtin.string \"hello \\k \"".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let res = attr_parser().parse(state_stream);
        let err_msg = format!("{}", res.err().unwrap());
        let expected_err_msg = expect![[r#"
            Parse error at line: 1, column: 23
            Unexpected escaped character \k
        "#]];
        expected_err_msg.assert_eq(&err_msg);
    }

    #[test]
    fn test_dictionary_attributes() {
        let hello_attr: AttrObj = StringAttr::new("hello".to_string()).into();
        let world_attr: AttrObj = StringAttr::new("world".to_string()).into();

        let hello_id: Identifier = "hello".try_into().unwrap();
        let world_id: Identifier = "world".try_into().unwrap();

        let mut dict1: AttrObj = DictAttr::new(vec![
            (hello_id.clone(), hello_attr.clone()),
            (world_id.clone(), world_attr.clone()),
        ])
        .into();
        let mut dict2 = DictAttr::new(vec![(
            hello_id.clone(),
            StringAttr::new("hello".to_string()).into(),
        )])
        .into();
        let dict1_rev = DictAttr::new(vec![
            (world_id.clone(), world_attr.clone()),
            (hello_id.clone(), hello_attr.clone()),
        ])
        .into();
        assert!(dict1.ne(&dict2));
        assert!(dict1 == dict1_rev);

        let dict1_attr = dict1.as_mut().downcast_mut::<DictAttr>().unwrap();
        let dict2_attr = dict2.as_mut().downcast_mut::<DictAttr>().unwrap();
        assert!(dict1_attr.lookup(&hello_id).unwrap() == &hello_attr);
        assert!(dict1_attr.lookup(&world_id).unwrap() == &world_attr);
        assert!(
            dict1_attr
                .lookup(&"hello_world".try_into().unwrap())
                .is_none()
        );
        dict2_attr.insert(&world_id, world_attr);
        assert!(dict1_attr == dict2_attr);

        dict1_attr.remove(&hello_id);
        dict2_attr.remove(&hello_id);
        assert!(dict1.eq(&dict2));
    }

    #[test]
    fn test_vec_attributes() {
        let hello_attr: AttrObj = StringAttr::new("hello".to_string()).into();
        let world_attr: AttrObj = StringAttr::new("world".to_string()).into();

        let vec_attr: AttrObj = VecAttr::new(vec![hello_attr.clone(), world_attr.clone()]).into();
        let vec = vec_attr.downcast_ref::<VecAttr>().unwrap();
        assert!(vec.0.len() == 2 && vec.0[0] == hello_attr && vec.0[1] == world_attr);
    }

    #[test]
    fn test_type_attributes() {
        let mut ctx = Context::new();

        let ty = IntegerType::get(&mut ctx, 64, Signedness::Signed).into();
        let ty_attr: AttrObj = TypeAttr::new(ty).into();

        let ty_interface = attr_cast::<dyn TypedAttrInterface>(&*ty_attr).unwrap();
        assert!(ty_interface.get_type(&ctx) == ty);

        let ty_attr = ty_attr.disp(&ctx).to_string();
        let state_stream = state_stream_from_iterator(
            ty_attr.chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let ty_attr_parsed = attr_parser().parse(state_stream).unwrap().0;
        assert_eq!(ty_attr_parsed.disp(&ctx).to_string(), ty_attr);
    }

    #[test]
    fn test_operand_segment_sizes_attr() {
        let mut ctx = Context::new();

        let sizes = vec![1, 2, 3];
        let attr: AttrObj = OperandSegmentSizesAttr(sizes.clone()).into();
        let attr_parsed = attr.downcast_ref::<OperandSegmentSizesAttr>().unwrap();
        assert_eq!(attr_parsed.0, sizes);

        let attr_disp = attr.disp(&ctx).to_string();
        let state_stream = state_stream_from_iterator(
            attr_disp.chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );
        let attr_parsed = attr_parser().parse(state_stream).unwrap().0;
        assert_eq!(attr_parsed.disp(&ctx).to_string(), attr_disp);
        let attr_parsed = attr_parsed
            .downcast_ref::<OperandSegmentSizesAttr>()
            .unwrap();
        assert_eq!(attr_parsed.0, sizes);
    }

    #[test]
    fn test_floating_point_attributes() {
        // Single precision float
        let single_attr: AttrObj = FPSingleAttr::from(std::f32::consts::PI).into();
        let single_attr2: AttrObj = FPSingleAttr::from(2.71).into();

        assert!(single_attr.is::<FPSingleAttr>());
        assert_ne!(&single_attr, &single_attr2);

        let single_attr = *single_attr.downcast::<FPSingleAttr>().unwrap();
        assert_eq!(f32::from(single_attr.clone()), std::f32::consts::PI);

        let single_attr2 = *single_attr2.downcast::<FPSingleAttr>().unwrap();
        // Perform addition
        let sum: f32 = f32::from(single_attr.clone()) + f32::from(single_attr2);
        assert!((sum - 5.8515927).abs() < 1e-6);

        // Double precision float
        let double_attr: AttrObj = FPDoubleAttr::from(std::f64::consts::TAU).into();
        let double_attr2: AttrObj = FPDoubleAttr::from(1.414).into();

        assert!(double_attr.is::<FPDoubleAttr>());
        assert_ne!(&double_attr, &double_attr2);

        let double_attr = *double_attr.downcast::<FPDoubleAttr>().unwrap();
        assert_eq!(f64::from(double_attr.clone()), std::f64::consts::TAU);

        let double_attr2 = *double_attr2.downcast::<FPDoubleAttr>().unwrap();
        // Perform multiplication
        let product: f64 = f64::from(double_attr) * f64::from(double_attr2);
        assert!((product - 8.884424024).abs() < 1e-6);
    }
}
