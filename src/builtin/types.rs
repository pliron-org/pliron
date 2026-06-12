use alloc::vec::Vec;

use pliron::derive::{pliron_type, type_interface_impl};

use crate::{
    builtin::type_interfaces::{FloatTypeInterface, FunctionTypeInterface},
    combine::{
        Parser, choice,
        parser::char::{spaces, string},
    },
    context::{Context, Ptr},
    irfmt::parsers::int_parser,
    parsable::{Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    r#type::{Type, TypeObj, TypePtr},
    utils::apfloat::{self, GetSemantics, Semantics},
};

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Signedness {
    Signed,
    Unsigned,
    Signless,
}

#[pliron_type(name = "builtin.integer", generate_get = true, verifier = "succ")]
#[derive(Hash, PartialEq, Eq, Debug, Clone)]
pub struct IntegerType {
    width: u32,
    signedness: Signedness,
}

impl IntegerType {
    /// Get, if it already exists, an integer type.
    pub fn get_existing(
        ctx: &Context,
        width: u32,
        signedness: Signedness,
    ) -> Option<TypePtr<Self>> {
        Type::get_instance(IntegerType { width, signedness }, ctx)
    }

    /// Get width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get signedness.
    pub fn signedness(&self) -> Signedness {
        self.signedness
    }

    /// Is Signed?
    pub fn is_signed(&self) -> bool {
        matches!(self.signedness, Signedness::Signed)
    }

    /// Is Unsigned?
    pub fn is_unsigned(&self) -> bool {
        matches!(self.signedness, Signedness::Unsigned)
    }

    /// Is Signless?
    pub fn is_signless(&self) -> bool {
        matches!(self.signedness, Signedness::Signless)
    }
}

impl Parsable for IntegerType {
    type Arg = ();
    type Parsed = TypePtr<Self>;
    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>
    where
        Self: Sized,
    {
        // Choose b/w si/ui/i ...
        let choicer = choice((
            string("si").map(|_| Signedness::Signed),
            string("ui").map(|_| Signedness::Unsigned),
            string("i").map(|_| Signedness::Signless),
        ));

        // followed by an integer.
        let mut parser = spaces().with(choicer.and(int_parser()));
        parser
            .parse_stream(state_stream)
            .map(|(signedness, width)| IntegerType::get(state_stream.state.ctx, width, signedness))
            .into()
    }
}

impl Printable for IntegerType {
    fn fmt(
        &self,
        _ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match &self.signedness {
            Signedness::Signed => write!(f, "si{}", self.width)?,
            Signedness::Unsigned => write!(f, "ui{}", self.width)?,
            Signedness::Signless => write!(f, "i{}", self.width)?,
        };
        Ok(())
    }
}

/// Map from a list of inputs to a list of results
///
/// See MLIR's [FunctionType](https://mlir.llvm.org/docs/Dialects/Builtin/#functiontype).
///
#[pliron_type(
    name = "builtin.function",
    format = "`<` `(` vec($inputs, CharSpace(`,`)) `)` `->` `(`vec($results, CharSpace(`,`)) `)` `>`",
    generate_get = true,
    verifier = "succ"
)]
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct FunctionType {
    /// Function arguments / inputs.
    inputs: Vec<Ptr<TypeObj>>,
    /// Function results / outputs.
    results: Vec<Ptr<TypeObj>>,
}

impl FunctionType {
    /// Get, if it already exists, a Function type.
    pub fn get_existing(
        ctx: &Context,
        inputs: Vec<Ptr<TypeObj>>,
        results: Vec<Ptr<TypeObj>>,
    ) -> Option<TypePtr<Self>> {
        Type::get_instance(FunctionType { inputs, results }, ctx)
    }
}

#[type_interface_impl]
impl FunctionTypeInterface for FunctionType {
    /// Get a reference to the function input / argument types.
    fn arg_types(&self) -> Vec<Ptr<TypeObj>> {
        self.inputs.clone()
    }

    /// Get a reference to the function result / output types.
    fn res_types(&self) -> Vec<Ptr<TypeObj>> {
        self.results.clone()
    }
}

#[pliron_type(name = "builtin.unit", format, generate_get = true, verifier = "succ")]
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct UnitType;

#[pliron_type(name = "builtin.fp32", format, generate_get = true, verifier = "succ")]
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct FP32Type;
#[type_interface_impl]
impl FloatTypeInterface for FP32Type {
    fn get_semantics(&self) -> Semantics {
        apfloat::Single::get_semantics()
    }
}

#[pliron_type(name = "builtin.fp64", format, generate_get = true, verifier = "succ")]
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct FP64Type;
#[type_interface_impl]
impl FloatTypeInterface for FP64Type {
    fn get_semantics(&self) -> Semantics {
        apfloat::Double::get_semantics()
    }
}

#[pliron_type(name = "builtin.fp16", format, generate_get = true, verifier = "succ")]
#[derive(Hash, PartialEq, Eq, Debug)]
pub struct FP16Type;
#[type_interface_impl]
impl FloatTypeInterface for FP16Type {
    fn get_semantics(&self) -> Semantics {
        apfloat::Half::get_semantics()
    }
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::ToString, vec};
    use expect_test::expect;

    use super::*;

    use crate::{
        builtin::types::{IntegerType, Signedness},
        combine::{Parser, eof},
        context::Context,
        location,
        parsable::{self, Parsable, state_stream_from_iterator},
        r#type::Type,
    };

    #[test]
    fn test_integer_types() {
        let mut ctx = Context::new();

        let int32_1_ptr = IntegerType::get(&mut ctx, 32, Signedness::Signed);
        let int32_2_ptr = IntegerType::get(&mut ctx, 32, Signedness::Signed);
        let int64_ptr = IntegerType::get(&mut ctx, 64, Signedness::Signed);
        let uint32_ptr = IntegerType::get(&mut ctx, 32, Signedness::Unsigned);

        assert!(int32_1_ptr.deref(&ctx).hash_type() == int32_2_ptr.deref(&ctx).hash_type());
        assert!(int32_1_ptr.deref(&ctx).hash_type() != int64_ptr.deref(&ctx).hash_type());
        assert!(int32_1_ptr.deref(&ctx).hash_type() != uint32_ptr.deref(&ctx).hash_type());
        assert!(int32_1_ptr == int32_2_ptr);
        assert!(int32_1_ptr != int64_ptr);
        assert!(int32_1_ptr != uint32_ptr);

        assert!(int32_1_ptr.deref(&ctx).get_self_ptr(&ctx) == int32_1_ptr.into());
        assert!(int32_2_ptr.deref(&ctx).get_self_ptr(&ctx) == int32_1_ptr.into());
        assert!(int32_2_ptr.deref(&ctx).get_self_ptr(&ctx) == int32_2_ptr.into());
        assert!(int64_ptr.deref(&ctx).get_self_ptr(&ctx) == int64_ptr.into());
        assert!(uint32_ptr.deref(&ctx).get_self_ptr(&ctx) == uint32_ptr.into());
        assert!(uint32_ptr.deref(&ctx).get_self_ptr(&ctx) != int32_1_ptr.into());
        assert!(uint32_ptr.deref(&ctx).get_self_ptr(&ctx) != int64_ptr.into());
    }

    #[test]
    fn test_function_types() {
        let mut ctx = Context::new();
        let int32_1_ptr = IntegerType::get(&mut ctx, 32, Signedness::Signed);
        let int64_ptr = IntegerType::get(&mut ctx, 64, Signedness::Signed);

        let ft_ref = FunctionType::get(&mut ctx, vec![int32_1_ptr.into()], vec![int64_ptr.into()])
            .deref(&ctx);
        assert!(
            ft_ref.arg_types()[0] == int32_1_ptr.into()
                && ft_ref.res_types()[0] == int64_ptr.into()
        );
    }

    #[test]
    fn test_integer_parsing() {
        let mut ctx = Context::new();
        let state_stream = state_stream_from_iterator(
            "si64".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let res = IntegerType::parser(())
            .and(eof())
            .parse(state_stream)
            .unwrap()
            .0
            .0;
        assert!(res == IntegerType::get_existing(&ctx, 64, Signedness::Signed).unwrap())
    }

    #[test]
    fn test_integer_parsing_errs() {
        let mut ctx = Context::new();
        let a = "asi64".to_string();
        let state_stream = state_stream_from_iterator(
            a.chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let res = IntegerType::parser(()).parse(state_stream);
        let err_msg = format!("{}", res.err().unwrap());

        let expected_err_msg = expect![[r#"
            Parse error at line: 1, column: 1
            Unexpected `a`
            Expected whitespaces, si, ui or i
        "#]];
        expected_err_msg.assert_eq(&err_msg);
    }

    #[test]
    fn test_fntype_parsing() {
        let mut ctx = Context::new();

        let si32 = IntegerType::get(&mut ctx, 32, Signedness::Signed);

        let state_stream = state_stream_from_iterator(
            "<() -> (builtin.integer si32)>".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let res = FunctionType::parser(())
            .and(eof())
            .parse(state_stream)
            .unwrap()
            .0
            .0;
        assert!(res == FunctionType::get_existing(&ctx, vec![], vec![si32.into()]).unwrap())
    }
}
