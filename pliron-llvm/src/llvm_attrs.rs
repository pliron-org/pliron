// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! LLVM [attributes](https://llvm.org/docs/LangRef.html#attributes).
//! Not to be confused with pliron [Attribute][pliron::attribute::Attribute]s.

use alloc::{string::String, vec::Vec};

use pliron::{
    builtin::attr_interfaces::OutlinedAttr,
    combine::{self, Parser, choice, optional, token},
    context::Context,
    derive::{attr_interface_impl, pliron_attr},
    irfmt::{
        parsers::{delimited_list_parser, spaced, type_parser},
        printers::quoted,
    },
    parsable::Parsable,
    printable::{self, Printable},
    r#type::TypeHandle,
};

/// The payload of one LLVM attribute.
#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum LlvmAttrValue {
    /// An enum attribute with no value, such as `nounwind`.
    /// Printed as just `"nounwind"`.
    Unit,
    /// An enum attribute with an integer value, such as `alignstack(16)`.
    /// Printed as `"alignstack" = 16`.
    Int(u64),
    /// An enum attribute with a type value, such as `byval(%struct.S)`.
    /// Printed as `"byval" = builtin.integer i32`.
    Type(TypeHandle),
    /// A string attribute, such as `"target-cpu"="x86-64"`.
    /// Printed as `"target-cpu" = "x86-64"`.
    Str(String),
}

/// LLVM attributes at one attribute index.
///
/// An attribute index can be a parameter from 1 to N, or one of
/// `LLVM_ATTRIBUTE_FUNCTION_INDEX` or `LLVM_ATTRIBUTE_RETURN_INDEX`.
///
/// Printed as `["nounwind", "alignstack" = 16, "target-cpu" = "x86-64"]`.
#[pliron_attr(name = "llvm.attributes", verifier = "succ")]
#[derive(PartialEq, Eq, Clone, Debug, Hash, Default)]
pub struct LlvmAttributesAttr(Vec<(String, LlvmAttrValue)>);

/// Always printed outlined.
#[attr_interface_impl]
impl OutlinedAttr for LlvmAttributesAttr {}

impl LlvmAttributesAttr {
    /// No attributes.
    pub fn new() -> Self {
        Self::default()
    }

    /// The value of the attribute named `name`, if it is present.
    pub fn get(&self, name: &str) -> Option<&LlvmAttrValue> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// True if the attribute named `name` is present.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Set the attribute named `name` to `value`, replacing any existing value.
    pub fn set(&mut self, name: impl Into<String>, value: LlvmAttrValue) {
        let name = name.into();
        match self.0.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = value,
            None => self.0.push((name, value)),
        }
    }

    /// Remove the attribute named `name`, if it is present.
    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(n, _)| n != name);
    }

    /// True if there are no attributes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over `(name, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &LlvmAttrValue)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value))
    }
}

impl Printable for LlvmAttributesAttr {
    fn fmt(
        &self,
        ctx: &Context,
        state: &printable::State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "[")?;
        for (idx, (name, value)) in self.0.iter().enumerate() {
            if idx != 0 {
                write!(f, ", ")?;
            }
            quoted(name).fmt(ctx, state, f)?;
            match value {
                LlvmAttrValue::Unit => (),
                LlvmAttrValue::Int(i) => write!(f, " = {i}")?,
                LlvmAttrValue::Type(ty) => write!(f, " = {}", ty.print(ctx, state))?,
                LlvmAttrValue::Str(s) => {
                    write!(f, " = ")?;
                    quoted(s).fmt(ctx, state, f)?;
                }
            }
        }
        write!(f, "]")
    }
}

impl Parsable for LlvmAttributesAttr {
    type Arg = ();
    type Parsed = Self;

    fn parse<'a>(
        state_stream: &mut pliron::parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> pliron::parsable::ParseResult<'a, Self::Parsed> {
        // No need to lookahead.
        // A string value opens with `"`, an integer with a digit and a type with neither.
        let value = choice!(
            String::parser(()).map(LlvmAttrValue::Str),
            u64::parser(()).map(LlvmAttrValue::Int),
            type_parser().map(LlvmAttrValue::Type)
        );
        let entry = String::parser(()).and(
            optional(combine::attempt(spaced(token('=')).with(value)))
                .map(|value| value.unwrap_or(LlvmAttrValue::Unit)),
        );

        delimited_list_parser('[', ']', ',', entry)
            .map(LlvmAttributesAttr)
            .parse_stream(state_stream)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use expect_test::expect;
    use pliron::{
        builtin::types::{IntegerType, Signedness},
        parsable::parse_from_str,
        result::ExpectOk,
    };

    use super::*;

    /// A list with one entry of every payload shape.
    fn sample(ctx: &mut Context) -> LlvmAttributesAttr {
        let i32_ty = IntegerType::get(ctx, 32, Signedness::Signless);
        let mut attrs = LlvmAttributesAttr::new();
        attrs.set("nounwind", LlvmAttrValue::Unit);
        attrs.set("alignstack", LlvmAttrValue::Int(16));
        attrs.set("byval", LlvmAttrValue::Type(i32_ty.into()));
        attrs.set("target-cpu", LlvmAttrValue::Str("x86-64".to_string()));
        attrs.set("no-jump-tables", LlvmAttrValue::Str(String::new()));
        attrs
    }

    /// Print `attrs`, parse it back and check that nothing changed.
    fn assert_roundtrips(ctx: &mut Context, attrs: LlvmAttributesAttr) {
        let printed = attrs.disp(ctx).to_string();
        let parsed = parse_from_str(LlvmAttributesAttr::parser(()), ctx, &printed).expect_ok(ctx);
        assert_eq!(parsed, attrs, "round-trip mismatch for `{printed}`");
    }

    #[test]
    fn test_llvm_attributes_attr_fmt() {
        let ctx = &mut Context::default();
        let attrs = sample(ctx);

        expect![[
            r#"["nounwind", "alignstack" = 16, "byval" = builtin.integer i32, "target-cpu" = "x86-64", "no-jump-tables" = ""]"#
        ]]
        .assert_eq(&attrs.disp(ctx).to_string());
    }

    #[test]
    fn test_llvm_attributes_attr_roundtrip() {
        let ctx = &mut Context::default();
        assert_roundtrips(ctx, LlvmAttributesAttr::new());
        let attrs = sample(ctx);
        assert_roundtrips(ctx, attrs);
    }

    #[test]
    fn test_llvm_attributes_attr_set_replaces_and_remove() {
        let mut attrs = LlvmAttributesAttr::new();
        attrs.set("alignstack", LlvmAttrValue::Int(8));
        attrs.set("alignstack", LlvmAttrValue::Int(16));
        assert_eq!(attrs.iter().count(), 1);
        assert_eq!(attrs.get("alignstack"), Some(&LlvmAttrValue::Int(16)));

        attrs.remove("alignstack");
        assert!(attrs.is_empty());
        assert!(!attrs.has("alignstack"));
    }
}
