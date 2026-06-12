//! [Dialect]s are a mechanism to group related [Op](crate::op::Op)s, [Type](crate::type::Type)s
//! and [Attribute](crate::attribute::Attribute)s.
use core::{fmt::Display, ops::Deref};

use alloc::string::String;
use rustc_hash::FxHashMap;

use crate::{
    attribute::{AttrId, AttrParserFn},
    combine::Parser,
    context::Context,
    identifier::Identifier,
    impl_printable_for_display, input_err,
    location::Located,
    op::{OpId, OpParserFn},
    parsable::{IntoParseResult, Parsable, ParseResult, StateStream},
    printable::{self, Printable},
    r#type::{TypeId, TypeParserFn},
};

/// Dialect name: Safe wrapper around a String.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct DialectName(Identifier);

impl DialectName {
    /// Create a new DialectName
    pub fn new(name: &str) -> DialectName {
        DialectName(name.try_into().expect("Invalid Identifier for DialectName"))
    }
}

impl From<&str> for DialectName {
    fn from(value: &str) -> Self {
        DialectName::new(value)
    }
}

impl_printable_for_display!(DialectName);

impl Display for DialectName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Parsable for DialectName {
    type Arg = ();
    type Parsed = DialectName;

    fn parse<'a>(
        state_stream: &mut StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed>
    where
        Self: Sized,
    {
        let loc = state_stream.loc();
        let id = Identifier::parser(());
        let mut parser = id.then(move |dialect_name| {
            let loc = loc.clone();
            combine::parser(move |state_stream: &mut StateStream<'a>| {
                let dialect_name = DialectName::new(&dialect_name);
                if state_stream.state.ctx.dialects.contains_key(&dialect_name) {
                    Ok(dialect_name).into_parse_result()
                } else {
                    input_err!(loc.clone(), "Unregistered dialect {}", *dialect_name)?
                }
            })
        });
        parser.parse_stream(state_stream).into()
    }
}

impl Deref for DialectName {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A collection of Types and Ops.
/// Dialects are identified by their names.
pub struct Dialect {
    /// Name of this dialect.
    pub name: DialectName,
    /// Ops that are part of this dialect.
    pub(crate) ops: FxHashMap<OpId, OpParserFn>,
    /// Types that are part of this dialect.
    pub(crate) types: FxHashMap<TypeId, TypeParserFn>,
    /// Attributes that are part of this dialect.
    pub(crate) attributes: FxHashMap<AttrId, AttrParserFn>,
}

impl Printable for Dialect {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "{}", self.name.disp(ctx))
    }
}

impl Dialect {
    /// Create a new unregistered dialect.
    pub fn new(name: DialectName) -> Dialect {
        Dialect {
            name,
            ops: FxHashMap::default(),
            types: FxHashMap::default(),
            attributes: FxHashMap::default(),
        }
    }

    /// Register this dialect if not already registered.
    pub fn register<'a>(ctx: &'a mut Context, name: &DialectName) -> &'a mut Dialect {
        // TODO: This should ideally be a single-lookup, so let's wait for Polonius
        if !ctx.dialects.contains_key(name) {
            ctx.dialects
                .insert(name.clone(), Dialect::new(name.clone()));
        }
        ctx.dialects.get_mut(name).unwrap()
    }

    /// Add an [Op](crate::op::Op) to this dialect.
    pub(crate) fn add_op(&mut self, op: OpId, op_parser: OpParserFn) {
        assert!(op.dialect == self.name);
        self.ops.insert(op, op_parser);
    }

    /// Add a [Type](crate::type::Type) to this dialect.
    pub(crate) fn add_type(&mut self, ty: TypeId, ty_parser: TypeParserFn) {
        assert!(ty.dialect == self.name);
        self.types.insert(ty, ty_parser);
    }

    /// Add an [Attribute](crate::attribute::Attribute) to this dialect.
    pub(crate) fn add_attr(&mut self, attr: AttrId, attr_parser: AttrParserFn) {
        assert!(attr.dialect == self.name);
        self.attributes.insert(attr, attr_parser);
    }

    /// This Dialect's name.
    pub fn name(&self) -> &DialectName {
        &self.name
    }
}

#[cfg(test)]
mod test {
    use alloc::{format, string::ToString};
    use expect_test::expect;

    use crate::{
        context::Context,
        location,
        parsable::{self, Parsable, state_stream_from_iterator},
        printable::Printable,
    };

    use super::*;

    #[test]
    fn parse_dialect_name() {
        let mut ctx = Context::new();

        let state_stream = state_stream_from_iterator(
            "non_existant".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let res = DialectName::parser(()).parse(state_stream);
        let err_msg = format!("{}", res.err().unwrap());

        let expected_err_msg = expect![[r#"
            Parse error at line: 1, column: 1
            Unregistered dialect non_existant
        "#]];
        expected_err_msg.assert_eq(&err_msg);

        let state_stream = state_stream_from_iterator(
            "builtin".chars(),
            parsable::State::new(&mut ctx, location::Source::InMemory),
        );

        let parsed = DialectName::parser(()).parse(state_stream).unwrap().0;
        assert_eq!(parsed.disp(&ctx).to_string(), "builtin");
    }
}
