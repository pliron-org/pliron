// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    DeriveInput, Ident, LitStr, Path, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

use crate::{
    DeriveIRObject,
    derive_attr::DefAttribute,
    derive_format::derive_format_inner,
    derive_op::{DefOp, derive_attr_get_set_inner, operands_inner, results_inner},
    derive_type::{DefType, DeriveTypeGet},
    interfaces::derive_op_interface_impl_inner,
    verify_succ::verify_succ_impl_inner,
};

/// Attribute specification for pliron entities
#[derive(Clone)]
struct AttributeSpec {
    name: Ident,
    ty: Option<Type>,
}

impl Parse for AttributeSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;
        let ty = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(AttributeSpec { name, ty })
    }
}

/// Operand specification for pliron operations.
/// Name is mandatory (`_` can be used to skip getter generation), type is optional.
#[derive(Clone)]
struct OperandSpec {
    name: Option<Ident>,
    ty: Option<Type>,
}

impl Parse for OperandSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            None
        } else {
            Some(input.parse()?)
        };

        let ty = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            Some(input.parse()?)
        } else {
            None
        };

        Ok(OperandSpec { name, ty })
    }
}

/// Result specification for pliron operations.
/// Name is mandatory (`_` can be used to skip getter generation), type is optional.
#[derive(Clone)]
struct ResultSpec {
    name: Option<Ident>,
    ty: Option<Type>,
}

impl Parse for ResultSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            None
        } else {
            Some(input.parse()?)
        };
        let ty = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(ResultSpec { ty, name })
    }
}

/// Format specification for pliron entities
#[derive(Clone)]
enum FormatSpec {
    /// Use default format generation
    Default,
    /// Use custom format string
    Custom(LitStr),
}

/// Configuration for pliron entity definitions
#[derive(Default)]
struct EntityConfig {
    name: Option<LitStr>,
    format: Option<FormatSpec>,
    interfaces: Option<Vec<Path>>,
    attributes: Option<Vec<AttributeSpec>>,
    operands: Option<Vec<OperandSpec>>,
    results: Option<Vec<ResultSpec>>,
    verifier: Option<LitStr>,
    generate_get: Option<bool>,
}

impl Parse for EntityConfig {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut config = EntityConfig::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;

            match key.to_string().as_str() {
                "format" => {
                    // Check if there's an equals sign - if not, it's default format
                    if input.peek(Token![=]) {
                        input.parse::<Token![=]>()?;
                        let format_str: LitStr = input.parse()?;
                        config.format = Some(FormatSpec::Custom(format_str));
                    } else {
                        config.format = Some(FormatSpec::Default);
                    }
                }
                _ => {
                    input.parse::<Token![=]>()?;
                    match key.to_string().as_str() {
                        "name" => {
                            config.name = Some(input.parse()?);
                        }
                        "interfaces" => {
                            let content;
                            syn::bracketed!(content in input);
                            let interfaces: Punctuated<Path, Token![,]> =
                                content.parse_terminated(Path::parse, Token![,])?;
                            config.interfaces = Some(interfaces.into_iter().collect());
                        }
                        "attributes" => {
                            let content;
                            syn::parenthesized!(content in input);
                            let attributes: Punctuated<AttributeSpec, Token![,]> =
                                content.parse_terminated(AttributeSpec::parse, Token![,])?;
                            config.attributes = Some(attributes.into_iter().collect());
                        }
                        "operands" => {
                            let content;
                            syn::parenthesized!(content in input);
                            let operands: Punctuated<OperandSpec, Token![,]> =
                                content.parse_terminated(OperandSpec::parse, Token![,])?;
                            config.operands = Some(operands.into_iter().collect());
                        }
                        "results" => {
                            let content;
                            syn::parenthesized!(content in input);
                            let results: Punctuated<ResultSpec, Token![,]> =
                                content.parse_terminated(ResultSpec::parse, Token![,])?;
                            config.results = Some(results.into_iter().collect());
                        }
                        "verifier" => {
                            let verifier: LitStr = input.parse()?;
                            if verifier.value() != "succ" {
                                return Err(syn::Error::new(
                                    verifier.span(),
                                    format!(
                                        "Unknown verifier value: '{}'. Only 'succ' is supported",
                                        verifier.value()
                                    ),
                                ));
                            }
                            config.verifier = Some(verifier);
                        }
                        "generate_get" => {
                            let value: syn::LitBool = input.parse()?;
                            config.generate_get = Some(value.value);
                        }
                        _ => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("Unknown configuration key: {}", key),
                            ));
                        }
                    }
                }
            }

            // Require comma separator between properties, allow trailing comma
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        // Validate that name is provided
        if config.name.is_none() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "name is a required field",
            ));
        }

        Ok(config)
    }
}

/// Helper function to add verifier implementation for structs and enums
fn add_verifier_impl(
    mut expanded: TokenStream,
    verifier: &Option<LitStr>,
    input: &DeriveInput,
) -> syn::Result<TokenStream> {
    if let Some(verifier) = verifier
        && verifier.value() == "succ"
    {
        expanded.extend(verify_succ_impl_inner(quote! {}, input.to_token_stream()));
        Ok(expanded)
    } else {
        Ok(expanded)
    }
}

/// Generate the expanded tokens for a pliron type definition
pub(crate) fn pliron_type(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args = args.into();
    let mut input = syn::parse2::<DeriveInput>(input.into())?;
    let config = syn::parse2::<EntityConfig>(args)?;

    // Validate that attributes is not specified for types
    if config.attributes.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "attributes is not supported for types",
        ));
    }

    // Validate that interfaces is not specified for types
    if config.interfaces.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "interfaces is not supported for types",
        ));
    }

    if config.operands.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "operands is not supported for types",
        ));
    }

    if config.results.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "results is not supported for types",
        ));
    }

    let mut expanded = quote! {};

    // Add derive_type_get if requested
    if let Some(true) = config.generate_get {
        expanded.extend(DeriveTypeGet::derive(&input)?.to_token_stream());
    }

    // Add format_type attribute
    match &config.format {
        Some(FormatSpec::Custom(format_str)) => {
            let fmt =
                derive_format_inner(quote! { #format_str }, &mut input, DeriveIRObject::Type)?;
            expanded.extend(fmt);
        }
        Some(FormatSpec::Default) => {
            let fmt = derive_format_inner(quote! {}, &mut input, DeriveIRObject::Type)?;
            expanded.extend(fmt);
        }
        _ => {}
    }

    // Add def_type attribute
    if let Some(name) = &config.name {
        expanded.extend(DefType::derive(name, &input)?.into_token_stream());
    }

    // Add verifier implementation
    expanded = add_verifier_impl(expanded, &config.verifier, &input)?;

    Ok(expanded)
}

/// Generate the expanded tokens for a pliron attribute definition
pub(crate) fn pliron_attr(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args = args.into();
    let mut input = syn::parse2::<DeriveInput>(input.into())?;
    let config = syn::parse2::<EntityConfig>(args)?;

    // Validate that generate_get is not specified for attributes
    if config.generate_get.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "generate_get is not supported for attributes",
        ));
    }

    // Validate that attributes is not specified for attributes
    if config.attributes.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "attributes is not supported for attributes",
        ));
    }

    // Validate that interfaces is not specified for attributes
    if config.interfaces.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "interfaces is not supported for attributes",
        ));
    }

    if config.operands.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "operands is not supported for attributes",
        ));
    }

    if config.results.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "results is not supported for attributes",
        ));
    }

    let mut expanded = quote! {};

    // Add format_attribute attribute
    match &config.format {
        Some(FormatSpec::Custom(format_str)) => {
            let fmt = derive_format_inner(
                quote! { #format_str },
                &mut input,
                DeriveIRObject::Attribute,
            )?;
            expanded.extend(fmt);
        }
        Some(FormatSpec::Default) => {
            let fmt = derive_format_inner(quote! {}, &mut input, DeriveIRObject::Attribute)?;
            expanded.extend(fmt)
        }
        _ => {}
    }

    // Add def_attribute attribute
    if let Some(name) = &config.name {
        expanded.extend(DefAttribute::derive(name, &input)?.into_token_stream());
    }

    // Add verifier implementation
    expanded = add_verifier_impl(expanded, &config.verifier, &input)?;

    Ok(expanded)
}

/// Generate the expanded tokens for a pliron operation definition
pub(crate) fn pliron_op(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args = args.into();
    let mut input = syn::parse2::<DeriveInput>(input.into())?;
    let config = syn::parse2::<EntityConfig>(args)?;

    // Validate that generate_get is not specified for operations
    if config.generate_get.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "generate_get is not supported for operations",
        ));
    }

    let mut interfaces = config.interfaces.unwrap_or_default();
    let mut expanded = quote! {};

    // Add attributes if specified (only for operations)
    if let Some(attributes) = &config.attributes
        && !attributes.is_empty()
    {
        let attr_list = attributes.iter().map(|attr| {
            let name = &attr.name;
            if let Some(ty) = &attr.ty {
                quote! { #name : #ty }
            } else {
                quote! { #name }
            }
        });
        expanded.extend(derive_attr_get_set_inner(
            quote! { #(#attr_list),* },
            &mut input,
        ));
    }

    // Add operands if specified.
    if let Some(operands) = &config.operands
        && !operands.is_empty()
    {
        let operand_list = operands.iter().map(|operand| {
            let ty = &operand.ty;
            match (&operand.name, ty) {
                (Some(name), Some(ty)) => quote! { #name : #ty },
                (Some(name), None) => quote! { #name },
                (None, Some(ty)) => quote! { _ : #ty },
                (None, None) => quote! { _ },
            }
        });
        let (operands, opd_interfaces) = operands_inner(quote! { #(#operand_list),* }, &mut input)?;
        interfaces.extend(opd_interfaces);
        expanded.extend(operands);
    }

    // Add results if specified.
    if let Some(results) = &config.results
        && !results.is_empty()
    {
        let result_list = results.iter().map(|result| {
            let ty = &result.ty;
            match (&result.name, ty) {
                (Some(name), Some(ty)) => quote! { #name : #ty },
                (Some(name), None) => quote! { #name },
                (None, Some(ty)) => quote! { _ : #ty },
                (None, None) => quote! { _ },
            }
        });
        let (results, res_interfaces) = results_inner(quote! { #(#result_list),* }, &mut input)?;
        interfaces.extend(res_interfaces);
        expanded.extend(results);
    }

    // Add interface implementations if specified
    if !interfaces.is_empty() {
        expanded.extend(derive_op_interface_impl_inner(&interfaces, &input)?);
    }

    // Add format_op attribute
    match &config.format {
        Some(FormatSpec::Custom(format_str)) => {
            let fmt = derive_format_inner(quote! { #format_str }, &mut input, DeriveIRObject::Op)?;
            expanded.extend(fmt);
        }
        Some(FormatSpec::Default) => {
            let fmt = derive_format_inner(quote! {}, &mut input, DeriveIRObject::Op)?;
            expanded.extend(fmt);
        }
        _ => {}
    }

    // Add def_op attribute
    if let Some(name) = &config.name {
        expanded.extend(DefOp::derive(name, &input)?.into_token_stream());
    }

    // Add verifier implementation
    expanded = add_verifier_impl(expanded, &config.verifier, &input)?;

    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;
    use quote::quote;

    #[test]
    fn pliron_type_basic() {
        let args = quote! { name = "test.unit_type", verifier = "succ" };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            pub struct UnitType;
        };
        let result = pliron_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            pub struct UnitType;
            impl ::pliron::r#type::Type for UnitType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::ident!("unit_type").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< UnitType as ::pliron::r#type::Type > ::register);
            impl ::pliron::common_traits::Verify for UnitType {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_type_with_format() {
        let args = quote! {
            name = "test.flags_type",
            format = "`type` `{` $flags `}`",
            verifier = "succ"
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct FlagsType {
                flags: u32,
            }
        };
        let result = pliron_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl ::pliron::printable::Printable for FlagsType {
                fn fmt(
                    &self,
                    ctx: &::pliron::context::Context,
                    state: &::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    ::pliron::printable::Printable::fmt(&"type", ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&"{", ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&self.flags, ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&"}", ctx, state, fmt)?;
                    Ok(())
                }
            }
            impl ::pliron::parsable::Parsable for FlagsType {
                type Arg = ();
                type Parsed = ::pliron::r#type::TypedHandle<Self>;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("type"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("{"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let flags = <u32>::parse(state_stream, ())?.0;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("}"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let final_ret_value = FlagsType { flags };
                    Ok(::pliron::r#type::Type::instantiate(final_ret_value, state_stream.state.ctx))
                        .into_parse_result()
                }
            }
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct FlagsType {
                flags: u32,
            }
            impl ::pliron::r#type::Type for FlagsType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::ident!("flags_type").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< FlagsType as ::pliron::r#type::Type > ::register);
            impl ::pliron::common_traits::Verify for FlagsType {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_type_with_get() {
        let args = quote! {
            name = "test.vector_type",
            generate_get = true,
            verifier = "succ"
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct VectorType {
                elem_ty: u32,
                num_elems: u32,
            }
        };
        let result = pliron_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl VectorType {
                /// Get or create a new instance.
                pub fn get(
                    ctx: &::pliron::context::Context,
                    elem_ty: u32,
                    num_elems: u32,
                ) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::instantiate(VectorType { elem_ty, num_elems }, ctx)
                }
            }
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct VectorType {
                elem_ty: u32,
                num_elems: u32,
            }
            impl ::pliron::r#type::Type for VectorType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::ident!("vector_type").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< VectorType as ::pliron::r#type::Type > ::register);
            impl ::pliron::common_traits::Verify for VectorType {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_type_interfaces_not_supported() {
        let args = quote! {
            name = "test.interface_type",
            interfaces = [Interface1, Interface2],
            verifier = "succ"
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct InterfaceType;
        };
        let result = pliron_type(args, input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("interfaces is not supported for types")
        );
    }

    #[test]
    fn pliron_attr_basic() {
        let args = quote! { name = "test.string_attr", verifier = "succ" };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct StringAttr {
                value: String,
            }
        };
        let result = pliron_attr(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct StringAttr {
                value: String,
            }
            impl ::pliron::attribute::Attribute for StringAttr {
                fn hash_attr(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }
                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::ident!("string_attr").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::attribute::ATTR_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(
                < StringAttr as ::pliron::attribute::Attribute > ::register
            );
            impl ::pliron::common_traits::Verify for StringAttr {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_attr_with_format() {
        let args = quote! {
            name = "test.string_attr",
            format = "`attr` `(` $value `)`",
            verifier = "succ"
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct StringAttr {
                value: String,
            }
        };
        let result = pliron_attr(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl ::pliron::printable::Printable for StringAttr {
                fn fmt(
                    &self,
                    ctx: &::pliron::context::Context,
                    state: &::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    ::pliron::printable::Printable::fmt(&"attr", ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&"(", ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&self.value, ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&")", ctx, state, fmt)?;
                    Ok(())
                }
            }
            impl ::pliron::parsable::Parsable for StringAttr {
                type Arg = ();
                type Parsed = Self;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("attr"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("("))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let value = <String>::parse(state_stream, ())?.0;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(")"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let final_ret_value = StringAttr { value };
                    Ok(final_ret_value).into_parse_result()
                }
            }
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct StringAttr {
                value: String,
            }
            impl ::pliron::attribute::Attribute for StringAttr {
                fn hash_attr(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }
                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::ident!("string_attr").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::attribute::ATTR_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(
                < StringAttr as ::pliron::attribute::Attribute > ::register
            );
            impl ::pliron::common_traits::Verify for StringAttr {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_attr_with_enum_and_format_default() {
        let args = quote! {
            name = "test.enum_attr",
            format,
            verifier = "succ"
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            enum PredicateAttr {
                EQ,
                NE,
            }
        };
        let result = pliron_attr(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl ::pliron::printable::Printable for PredicateAttr {
                fn fmt(
                    &self,
                    ctx: &::pliron::context::Context,
                    state: &::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    match self {
                        Self::EQ => {
                            write!(fmt, "{}", stringify!(EQ))?;
                        }
                        Self::NE => {
                            write!(fmt, "{}", stringify!(NE))?;
                        }
                    }
                    Ok(())
                }
            }
            impl ::pliron::parsable::Parsable for PredicateAttr {
                type Arg = ();
                type Parsed = Self;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();
                    let variant_name_parsed = ::pliron::alloc::string::ToString::to_string(
                        &::pliron::identifier::Identifier::parse(state_stream, ())?.0,
                    );
                    let final_ret_value = match variant_name_parsed.as_str() {
                        "EQ" => PredicateAttr::EQ,
                        "NE" => PredicateAttr::NE,
                        _ => {
                            return input_err!(
                                cur_loc.clone(), "Invalid variant name: {}", variant_name_parsed
                            )?;
                        }
                    };
                    Ok(final_ret_value).into_parse_result()
                }
            }
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            enum PredicateAttr {
                EQ,
                NE,
            }
            impl ::pliron::attribute::Attribute for PredicateAttr {
                fn hash_attr(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }
                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::ident!("enum_attr").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::attribute::ATTR_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(
                < PredicateAttr as ::pliron::attribute::Attribute > ::register
            );
            impl ::pliron::common_traits::Verify for PredicateAttr {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_basic() {
        let args = quote! { name = "test.my_op", verifier = "succ" };
        let input = quote! {
            struct MyOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct MyOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for MyOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    MyOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("my_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< MyOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for MyOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_format_and_interfaces() {
        let args = quote! {
            name = "test.if_op",
            format = "`(`$0`)` region($0)",
            interfaces = [OneOpdInterface, ZeroResultInterface, OneRegionInterface],
            verifier = "succ"
        };
        let input = quote! {
            struct IfOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl OneOpdInterface for IfOp {}
            impl ZeroResultInterface for IfOp {}
            impl OneRegionInterface for IfOp {}
            ::pliron::type_to_trait!(IfOp, OneOpdInterface, ZeroResultInterface, OneRegionInterface);
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<IfOp>(),
                    &[
                        <IfOp as OneOpdInterface>::__all_verifiers,
                        <IfOp as ZeroResultInterface>::__all_verifiers,
                        <IfOp as OneRegionInterface>::__all_verifiers,
                    ],
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            impl ::pliron::printable::Printable for IfOp {
                fn fmt(
                    &self,
                    ctx: &::pliron::context::Context,
                    state: &::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    use ::pliron::op::Op;
                    use ::pliron::irfmt::printers::iter_with_sep;
                    use ::pliron::common_traits::Named;
                    let op = self.get_operation().deref(ctx);
                    if op.get_num_results() > 0 {
                        let sep = ::pliron::printable::ListSeparator::CharSpace(',');
                        let results = iter_with_sep(op.results(), sep);
                        write!(fmt, "{} = ", results.print(ctx, state))?;
                    }
                    write!(fmt, "{} ", self.get_opid())?;
                    ::pliron::printable::Printable::fmt(&"(", ctx, state, fmt)?;
                    let opd = self.get_operation().deref(ctx).get_operand(0usize);
                    ::pliron::printable::Printable::fmt(&opd, ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&")", ctx, state, fmt)?;
                    let reg = self.get_operation().deref(ctx).get_region(0usize);
                    ::pliron::printable::Printable::fmt(&reg, ctx, state, fmt)?;
                    Ok(())
                }
            }
            impl ::pliron::parsable::Parsable for IfOp {
                type Arg = ::pliron::alloc::vec::Vec<
                    (::pliron::identifier::Identifier, ::pliron::location::Location),
                >;
                type Parsed = ::pliron::op::OpObj;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();
                    use ::pliron::op::Op;
                    use ::pliron::operation::Operation;
                    use ::pliron::irfmt::parsers::{
                        process_parsed_ssa_defs, ssa_opd_parser, block_opd_parser,
                    };
                    let regions_temp_parent_op = Operation::new(
                        state_stream.state.ctx,
                        Self::get_concrete_op_info(),
                        ::pliron::alloc::vec![],
                        ::pliron::alloc::vec![],
                        ::pliron::alloc::vec![],
                        0,
                    );
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("("))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let opd_0 = ::pliron::irfmt::parsers::ssa_opd_parse(state_stream, ())?.0;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(")"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let reg_0 = ::pliron::region::Region::parser(regions_temp_parent_op)
                        .parse_stream(state_stream)
                        .into_result()?
                        .0;
                    let results = ::pliron::alloc::vec![];
                    if arg.len() != results.len() {
                        return input_err!(
                            cur_loc, "expected {} results as per spec, got {} during parsing",
                            results.len(), arg.len()
                        )?;
                    }
                    let op = ::pliron::operation::Operation::new(
                        state_stream.state.ctx,
                        Self::get_concrete_op_info(),
                        results,
                        ::pliron::alloc::vec![opd_0],
                        ::pliron::alloc::vec![],
                        0,
                    );
                    for region in ::pliron::alloc::vec![reg_0] {
                        ::pliron::region::Region::move_to_op(region, op, state_stream.state.ctx);
                    }
                    if !arg.is_empty() {
                        process_parsed_ssa_defs(state_stream, &arg, op)?;
                    }
                    Operation::erase(regions_temp_parent_op, state_stream.state.ctx);
                    let final_ret_value = Operation::get_op_dyn(op, state_stream.state.ctx);
                    Ok(final_ret_value).into_parse_result()
                }
            }
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct IfOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for IfOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    IfOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("if_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< IfOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for IfOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_type_no_verifier() {
        let args = quote! { name = "test.simple_type" };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct SimpleType;
        };
        let result = pliron_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct SimpleType;
            impl ::pliron::r#type::Type for SimpleType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::ident!("simple_type").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< SimpleType as ::pliron::r#type::Type > ::register);
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_attr_interfaces_not_supported() {
        let args = quote! {
            name = "test.interface_attr",
            interfaces = [AttrInterface1, AttrInterface2]
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct InterfaceAttr;
        };
        let result = pliron_attr(args, input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("interfaces is not supported for attributes")
        );
    }

    #[test]
    fn entity_config_parse_error() {
        let args = quote! { unknown_key = "value" };
        let input = quote! { struct TestType; };
        let result = pliron_type(args, input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown configuration key")
        );
    }

    #[test]
    fn pliron_type_with_default_format() {
        let args = quote! {
            name = "test.default_type",
            format,
            verifier = "succ"
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct DefaultType;
        };
        let result = pliron_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl ::pliron::printable::Printable for DefaultType {
                fn fmt(
                    &self,
                    ctx: &::pliron::context::Context,
                    state: &::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    Ok(())
                }
            }
            impl ::pliron::parsable::Parsable for DefaultType {
                type Arg = ();
                type Parsed = ::pliron::r#type::TypedHandle<Self>;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();
                    let final_ret_value = DefaultType {};
                    Ok(::pliron::r#type::Type::instantiate(final_ret_value, state_stream.state.ctx))
                        .into_parse_result()
                }
            }
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct DefaultType;
            impl ::pliron::r#type::Type for DefaultType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::ident!("default_type").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< DefaultType as ::pliron::r#type::Type > ::register);
            impl ::pliron::common_traits::Verify for DefaultType {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_generic_interfaces() {
        let args = quote! {
            name = "test.generic_op",
            interfaces = [NRegionsInterface<1>, ZeroResultInterface],
            verifier = "succ"
        };
        let input = quote! {
            struct GenericOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl NRegionsInterface<1> for GenericOp {}
            impl ZeroResultInterface for GenericOp {}
            ::pliron::type_to_trait!(GenericOp, NRegionsInterface < 1 >, ZeroResultInterface);
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<GenericOp>(),
                    &[
                        <GenericOp as NRegionsInterface<1>>::__all_verifiers,
                        <GenericOp as ZeroResultInterface>::__all_verifiers,
                    ],
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct GenericOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for GenericOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    GenericOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("generic_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< GenericOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for GenericOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_attributes() {
        let args = quote! {
            name = "test.call_op",
            attributes = (llvm_call_callee: IdentifierAttr, llvm_call_fastmath_flags: FastmathFlagsAttr),
            verifier = "succ"
        };
        let input = quote! {
            struct CallOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[doc(hidden)]
            pub mod call_op_attr_names {
                use ::pliron::std_deps::sync::LazyLock;
                use ::pliron::identifier::Identifier;
                ::pliron::dict_key!(ATTR_KEY_LLVM_CALL_CALLEE, "llvm_call_callee");
                ::pliron::dict_key!(ATTR_KEY_LLVM_CALL_FASTMATH_FLAGS, "llvm_call_fastmath_flags");
            }
            impl CallOp {
                ///Get a [Ref](core::cell::Ref) to the value of the attribute named `llvm_call_callee`.
                /// The `Ref` is a borrow of the containing `Operation` object.
                pub fn get_attr_llvm_call_callee<'a>(
                    &self,
                    ctx: &'a ::pliron::context::Context,
                ) -> Option<::core::cell::Ref<'a, IdentifierAttr>> {
                    ::core::cell::Ref::filter_map(
                            self.op.deref(ctx),
                            |op| {
                                op
                                    .attributes
                                    .get::<
                                        IdentifierAttr,
                                    >(&call_op_attr_names::ATTR_KEY_LLVM_CALL_CALLEE)
                            },
                        )
                        .ok()
                }
                ///Set the value of the attribute named `llvm_call_callee`.
                pub fn set_attr_llvm_call_callee(
                    &self,
                    ctx: &::pliron::context::Context,
                    value: IdentifierAttr,
                ) {
                    self.op
                        .deref_mut(ctx)
                        .attributes
                        .set(call_op_attr_names::ATTR_KEY_LLVM_CALL_CALLEE.clone(), value);
                }
                ///Get a [Ref](core::cell::Ref) to the value of the attribute named `llvm_call_fastmath_flags`.
                /// The `Ref` is a borrow of the containing `Operation` object.
                pub fn get_attr_llvm_call_fastmath_flags<'a>(
                    &self,
                    ctx: &'a ::pliron::context::Context,
                ) -> Option<::core::cell::Ref<'a, FastmathFlagsAttr>> {
                    ::core::cell::Ref::filter_map(
                            self.op.deref(ctx),
                            |op| {
                                op
                                    .attributes
                                    .get::<
                                        FastmathFlagsAttr,
                                    >(&call_op_attr_names::ATTR_KEY_LLVM_CALL_FASTMATH_FLAGS)
                            },
                        )
                        .ok()
                }
                ///Set the value of the attribute named `llvm_call_fastmath_flags`.
                pub fn set_attr_llvm_call_fastmath_flags(
                    &self,
                    ctx: &::pliron::context::Context,
                    value: FastmathFlagsAttr,
                ) {
                    self.op
                        .deref_mut(ctx)
                        .attributes
                        .set(call_op_attr_names::ATTR_KEY_LLVM_CALL_FASTMATH_FLAGS.clone(), value);
                }
            }
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            ///
            ///### Attribute(s):
            ///Note: Only attributes defined directly as part of this operation are listed here.
            ///There may be others, not listed here, defined by interface implementations.
            ///
            ///| Name | Static Name Identifier | Type |
            ///| ---- | ---------------------- | ---- |
            ///| `llvm_call_callee` | [ATTR_KEY_LLVM_CALL_CALLEE](call_op_attr_names::ATTR_KEY_LLVM_CALL_CALLEE) | [IdentifierAttr] |
            ///| `llvm_call_fastmath_flags` | [ATTR_KEY_LLVM_CALL_FASTMATH_FLAGS](call_op_attr_names::ATTR_KEY_LLVM_CALL_FASTMATH_FLAGS) | [FastmathFlagsAttr] |
            struct CallOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for CallOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    CallOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("call_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< CallOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for CallOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_optional_type_attributes() {
        let args = quote! {
            name = "test.constant_op",
            attributes = (constant_value, typed_attr: TypeAttr),
            verifier = "succ"
        };
        let input = quote! {
            struct ConstantOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[doc(hidden)]
            pub mod constant_op_attr_names {
                use ::pliron::std_deps::sync::LazyLock;
                use ::pliron::identifier::Identifier;
                ::pliron::dict_key!(ATTR_KEY_CONSTANT_VALUE, "constant_value");
                ::pliron::dict_key!(ATTR_KEY_TYPED_ATTR, "typed_attr");
            }
            impl ConstantOp {
                ///Get a [Ref](core::cell::Ref) to the value of the attribute named `constant_value`.
                /// The `Ref` is a borrow of the containing `Operation` object.
                pub fn get_attr_constant_value<'a>(
                    &self,
                    ctx: &'a ::pliron::context::Context,
                ) -> Option<::core::cell::Ref<'a, ::pliron::attribute::AttrObj>> {
                    ::core::cell::Ref::filter_map(
                            self.op.deref(ctx),
                            |op| {
                                op.attributes.0.get(&constant_op_attr_names::ATTR_KEY_CONSTANT_VALUE)
                            },
                        )
                        .ok()
                }
                ///Set the value of the attribute named `constant_value`.
                pub fn set_attr_constant_value(
                    &self,
                    ctx: &::pliron::context::Context,
                    value: ::pliron::attribute::AttrObj,
                ) {
                    self.op
                        .deref_mut(ctx)
                        .attributes
                        .0
                        .insert(constant_op_attr_names::ATTR_KEY_CONSTANT_VALUE.clone(), value);
                }
                ///Get a [Ref](core::cell::Ref) to the value of the attribute named `typed_attr`.
                /// The `Ref` is a borrow of the containing `Operation` object.
                pub fn get_attr_typed_attr<'a>(
                    &self,
                    ctx: &'a ::pliron::context::Context,
                ) -> Option<::core::cell::Ref<'a, TypeAttr>> {
                    ::core::cell::Ref::filter_map(
                            self.op.deref(ctx),
                            |op| {
                                op
                                    .attributes
                                    .get::<TypeAttr>(&constant_op_attr_names::ATTR_KEY_TYPED_ATTR)
                            },
                        )
                        .ok()
                }
                ///Set the value of the attribute named `typed_attr`.
                pub fn set_attr_typed_attr(
                    &self,
                    ctx: &::pliron::context::Context,
                    value: TypeAttr,
                ) {
                    self.op
                        .deref_mut(ctx)
                        .attributes
                        .set(constant_op_attr_names::ATTR_KEY_TYPED_ATTR.clone(), value);
                }
            }
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            ///
            ///### Attribute(s):
            ///Note: Only attributes defined directly as part of this operation are listed here.
            ///There may be others, not listed here, defined by interface implementations.
            ///
            ///| Name | Static Name Identifier | Type |
            ///| ---- | ---------------------- | ---- |
            ///| `constant_value` | [ATTR_KEY_CONSTANT_VALUE](constant_op_attr_names::ATTR_KEY_CONSTANT_VALUE) | Any |
            ///| `typed_attr` | [ATTR_KEY_TYPED_ATTR](constant_op_attr_names::ATTR_KEY_TYPED_ATTR) | [TypeAttr] |
            struct ConstantOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for ConstantOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    ConstantOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("constant_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< ConstantOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for ConstantOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_type_all_options() {
        let args = quote! {
            name = "test.full_type",
            format = "`full` `<` $field `>`",
            verifier = "succ",
            generate_get = true
        };
        let input = quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct FullType {
                field: u32,
            }
        };
        let result = pliron_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl FullType {
                /// Get or create a new instance.
                pub fn get(
                    ctx: &::pliron::context::Context,
                    field: u32,
                ) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::instantiate(FullType { field }, ctx)
                }
            }
            impl ::pliron::printable::Printable for FullType {
                fn fmt(
                    &self,
                    ctx: &::pliron::context::Context,
                    state: &::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    ::pliron::printable::Printable::fmt(&"full", ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&"<", ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&self.field, ctx, state, fmt)?;
                    ::pliron::printable::Printable::fmt(&">", ctx, state, fmt)?;
                    Ok(())
                }
            }
            impl ::pliron::parsable::Parsable for FullType {
                type Arg = ();
                type Parsed = ::pliron::r#type::TypedHandle<Self>;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("full"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string("<"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let field = <u32>::parse(state_stream, ())?.0;
                    ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(">"))
                        .parse_stream(state_stream)
                        .into_result()?;
                    let final_ret_value = FullType { field };
                    Ok(::pliron::r#type::Type::instantiate(final_ret_value, state_stream.state.ctx))
                        .into_parse_result()
                }
            }
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct FullType {
                field: u32,
            }
            impl ::pliron::r#type::Type for FullType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::ident!("full_type").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< FullType as ::pliron::r#type::Type > ::register);
            impl ::pliron::common_traits::Verify for FullType {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_operands() {
        let args = quote! {
            name = "test.bin_op",
            operands = (lhs: IntegerType, _, rhs, _: PointerType),
            verifier = "succ"
        };
        let input = quote! {
            struct BinOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl BinOp {
                pub fn get_operand_lhs(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_operand(0usize)
                }
                pub fn get_operand_rhs(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_operand(2usize)
                }
            }
            impl ::pliron::builtin::op_interfaces::OperandNOfType<0usize, IntegerType> for BinOp {}
            impl ::pliron::builtin::op_interfaces::OperandNOfType<3usize, PointerType> for BinOp {}
            ::pliron::type_to_trait!(
                BinOp, ::pliron::builtin::op_interfaces::OperandNOfType < 0usize, IntegerType >,
                ::pliron::builtin::op_interfaces::OperandNOfType < 3usize, PointerType >
            );
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<BinOp>(),
                    &[
                        <BinOp as ::pliron::builtin::op_interfaces::OperandNOfType<
                            0usize,
                            IntegerType,
                        >>::__all_verifiers,
                        <BinOp as ::pliron::builtin::op_interfaces::OperandNOfType<
                            3usize,
                            PointerType,
                        >>::__all_verifiers,
                    ],
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct BinOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for BinOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    BinOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("bin_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< BinOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for BinOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_results() {
        let args = quote! {
            name = "test.ret_op",
            results = (out: IntegerType, _, _: UnitType),
            verifier = "succ"
        };
        let input = quote! {
            struct RetOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl RetOp {
                pub fn get_result_out(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_result(0usize)
                }
            }
            impl ::pliron::builtin::op_interfaces::ResultNOfType<0usize, IntegerType> for RetOp {}
            impl ::pliron::builtin::op_interfaces::ResultNOfType<2usize, UnitType> for RetOp {}
            ::pliron::type_to_trait!(
                RetOp, ::pliron::builtin::op_interfaces::ResultNOfType < 0usize, IntegerType >,
                ::pliron::builtin::op_interfaces::ResultNOfType < 2usize, UnitType >
            );
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<RetOp>(),
                    &[
                        <RetOp as ::pliron::builtin::op_interfaces::ResultNOfType<
                            0usize,
                            IntegerType,
                        >>::__all_verifiers,
                        <RetOp as ::pliron::builtin::op_interfaces::ResultNOfType<
                            2usize,
                            UnitType,
                        >>::__all_verifiers,
                    ],
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct RetOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for RetOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    RetOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("ret_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< RetOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for RetOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_named_results_field() {
        let args = quote! {
            name = "test.named_results_op",
            results = (named_result: IntegerType),
            verifier = "succ"
        };
        let input = quote! {
            struct NamedResultsOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl NamedResultsOp {
                pub fn get_result_named_result(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_result(0usize)
                }
            }
            impl ::pliron::builtin::op_interfaces::ResultNOfType<0usize, IntegerType>
            for NamedResultsOp {}
            ::pliron::type_to_trait!(
                NamedResultsOp, ::pliron::builtin::op_interfaces::ResultNOfType < 0usize, IntegerType
                >
            );
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<NamedResultsOp>(),
                    &[
                        <NamedResultsOp as ::pliron::builtin::op_interfaces::ResultNOfType<
                            0usize,
                            IntegerType,
                        >>::__all_verifiers,
                    ],
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct NamedResultsOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for NamedResultsOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    NamedResultsOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("named_results_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< NamedResultsOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for NamedResultsOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn pliron_op_with_operands_and_results() {
        let args = quote! {
            name = "test.full_op",
            interfaces = [OneOpdInterface],
            operands = (input: IntegerType),
            results = (output: IntegerType),
            verifier = "succ"
        };
        let input = quote! {
            struct FullOp;
        };
        let result = pliron_op(args, input).unwrap();
        let f = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl FullOp {
                pub fn get_operand_input(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_operand(0usize)
                }
            }
            impl FullOp {
                pub fn get_result_output(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_result(0usize)
                }
            }
            impl OneOpdInterface for FullOp {}
            impl ::pliron::builtin::op_interfaces::OperandNOfType<0usize, IntegerType> for FullOp {}
            impl ::pliron::builtin::op_interfaces::ResultNOfType<0usize, IntegerType> for FullOp {}
            ::pliron::type_to_trait!(
                FullOp, OneOpdInterface, ::pliron::builtin::op_interfaces::OperandNOfType < 0usize,
                IntegerType >, ::pliron::builtin::op_interfaces::ResultNOfType < 0usize, IntegerType
                >
            );
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<FullOp>(),
                    &[
                        <FullOp as OneOpdInterface>::__all_verifiers,
                        <FullOp as ::pliron::builtin::op_interfaces::OperandNOfType<
                            0usize,
                            IntegerType,
                        >>::__all_verifiers,
                        <FullOp as ::pliron::builtin::op_interfaces::ResultNOfType<
                            0usize,
                            IntegerType,
                        >>::__all_verifiers,
                    ],
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct FullOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for FullOp {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }
                fn wrap_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }
                fn from_operation(
                    op: ::pliron::context::Ptr<::pliron::operation::Operation>,
                ) -> Self {
                    FullOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::ident!("full_op").into(),
                        dialect: ::pliron::ident!("test").into(),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::op::OP_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< FullOp as ::pliron::op::Op > ::register);
            impl ::pliron::common_traits::Verify for FullOp {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "#]]
        .assert_eq(&got);
    }
}
