use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Ident, LitStr, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
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
    interfaces: Option<Vec<Type>>,
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
                            let interfaces: Punctuated<Type, Token![,]> =
                                content.parse_terminated(Type::parse, Token![,])?;
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
    expanded: TokenStream,
    verifier: &Option<LitStr>,
    input: TokenStream,
) -> syn::Result<TokenStream> {
    if let Some(verifier) = verifier
        && verifier.value() == "succ"
    {
        let item: syn::Item = syn::parse2(input)?;
        match item {
            syn::Item::Struct(_) | syn::Item::Enum(_) => Ok(quote! {
                #[::pliron::derive::verify_succ]
                #expanded
            }),
            _ => Ok(expanded),
        }
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
    let input = input.into();
    let config = syn::parse2::<EntityConfig>(args)?;
    let input_tokens = input.clone();

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

    let mut expanded = quote! { #input_tokens };

    // Add derive_type_get if requested
    if let Some(true) = config.generate_get {
        expanded = quote! {
            #[::pliron::derive::derive_type_get]
            #expanded
        };
    }

    // Add format_type attribute
    match &config.format {
        Some(FormatSpec::Custom(format_str)) => {
            expanded = quote! {
                #[::pliron::derive::format_type(#format_str)]
                #expanded
            };
        }
        Some(FormatSpec::Default) => {
            expanded = quote! {
                #[::pliron::derive::format_type]
                #expanded
            };
        }
        _ => {}
    }

    // Add def_type attribute
    if let Some(name) = &config.name {
        expanded = quote! {
            #[::pliron::derive::def_type(#name)]
            #expanded
        };
    }

    // Add verifier implementation
    expanded = add_verifier_impl(expanded, &config.verifier, input)?;

    Ok(expanded)
}

/// Generate the expanded tokens for a pliron attribute definition
pub(crate) fn pliron_attr(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args = args.into();
    let input = input.into();
    let config = syn::parse2::<EntityConfig>(args)?;
    let input_tokens = input.clone();

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

    let mut expanded = quote! { #input_tokens };

    // Add format_attribute attribute
    match &config.format {
        Some(FormatSpec::Custom(format_str)) => {
            expanded = quote! {
                #[::pliron::derive::format_attribute(#format_str)]
                #expanded
            };
        }
        Some(FormatSpec::Default) => {
            expanded = quote! {
                #[::pliron::derive::format_attribute]
                #expanded
            };
        }
        _ => {}
    }

    // Add def_attribute attribute
    if let Some(name) = &config.name {
        expanded = quote! {
            #[::pliron::derive::def_attribute(#name)]
            #expanded
        };
    }

    // Add verifier implementation
    expanded = add_verifier_impl(expanded, &config.verifier, input)?;

    Ok(expanded)
}

/// Generate the expanded tokens for a pliron operation definition
pub(crate) fn pliron_op(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args = args.into();
    let input = input.into();
    let config = syn::parse2::<EntityConfig>(args)?;
    let input_tokens = input.clone();

    // Validate that generate_get is not specified for operations
    if config.generate_get.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "generate_get is not supported for operations",
        ));
    }

    let mut expanded = quote! { #input_tokens };

    // Add interface implementations if specified
    if let Some(interfaces) = &config.interfaces
        && !interfaces.is_empty()
    {
        let interface_list = quote! { #(#interfaces),* };
        expanded = quote! {
            #[::pliron::derive::derive_op_interface_impl(#interface_list)]
            #expanded
        };
    }

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
        expanded = quote! {
            #[::pliron::derive::derive_attr_get_set(#(#attr_list),*)]
            #expanded
        };
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
        expanded = quote! {
            #[::pliron::derive::operands(#(#operand_list),*)]
            #expanded
        };
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
        expanded = quote! {
            #[::pliron::derive::results(#(#result_list),*)]
            #expanded
        };
    }

    // Add format_op attribute
    match &config.format {
        Some(FormatSpec::Custom(format_str)) => {
            expanded = quote! {
                #[::pliron::derive::format_op(#format_str)]
                #expanded
            };
        }
        Some(FormatSpec::Default) => {
            expanded = quote! {
                #[::pliron::derive::format_op]
                #expanded
            };
        }
        _ => {}
    }

    // Add def_op attribute
    if let Some(name) = &config.name {
        expanded = quote! {
            #[::pliron::derive::def_op(#name)]
            #expanded
        };
    }

    // Add verifier implementation
    expanded = add_verifier_impl(expanded, &config.verifier, input)?;

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
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_type("test.unit_type")]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            pub struct UnitType;
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_type("test.flags_type")]
            #[::pliron::derive::format_type("`type` `{` $flags `}`")]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct FlagsType {
                flags: u32,
            }
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_type("test.vector_type")]
            #[::pliron::derive::derive_type_get]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct VectorType {
                elem_ty: u32,
                num_elems: u32,
            }
        "##]]
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
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_attribute("test.string_attr")]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct StringAttr {
                value: String,
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_attribute("test.string_attr")]
            #[::pliron::derive::format_attribute("`attr` `(` $value `)`")]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct StringAttr {
                value: String,
            }
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_attribute("test.enum_attr")]
            #[::pliron::derive::format_attribute]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            enum PredicateAttr {
                EQ,
                NE,
            }
        "##]]
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
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.my_op")]
            struct MyOp;
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.if_op")]
            #[::pliron::derive::format_op("`(`$0`)` region($0)")]
            #[::pliron::derive::derive_op_interface_impl(
                OneOpdInterface,
                ZeroResultInterface,
                OneRegionInterface
            )]
            struct IfOp;
        "##]]
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
            #[::pliron::derive::def_type("test.simple_type")]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct SimpleType;
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_type("test.default_type")]
            #[::pliron::derive::format_type]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct DefaultType;
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.generic_op")]
            #[::pliron::derive::derive_op_interface_impl(NRegionsInterface<1>, ZeroResultInterface)]
            struct GenericOp;
        "##]]
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
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.call_op")]
            #[::pliron::derive::derive_attr_get_set(
                llvm_call_callee:IdentifierAttr,
                llvm_call_fastmath_flags:FastmathFlagsAttr
            )]
            struct CallOp;
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
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.constant_op")]
            #[::pliron::derive::derive_attr_get_set(constant_value, typed_attr:TypeAttr)]
            struct ConstantOp;
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_type("test.full_type")]
            #[::pliron::derive::format_type("`full` `<` $field `>`")]
            #[::pliron::derive::derive_type_get]
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            struct FullType {
                field: u32,
            }
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.bin_op")]
            #[::pliron::derive::operands(lhs:IntegerType, _, rhs, _:PointerType)]
            struct BinOp;
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.ret_op")]
            #[::pliron::derive::results(out:IntegerType, _, _:UnitType)]
            struct RetOp;
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.named_results_op")]
            #[::pliron::derive::results(named_result:IntegerType)]
            struct NamedResultsOp;
        "##]]
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

        expect![[r##"
            #[::pliron::derive::verify_succ]
            #[::pliron::derive::def_op("test.full_op")]
            #[::pliron::derive::results(output:IntegerType)]
            #[::pliron::derive::operands(input:IntegerType)]
            #[::pliron::derive::derive_op_interface_impl(OneOpdInterface)]
            struct FullOp;
        "##]]
        .assert_eq(&got);
    }
}
