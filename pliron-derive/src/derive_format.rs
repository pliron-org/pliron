use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use rustc_hash::{FxHashMap, FxHashSet};
use syn::{Data, DeriveInput, Generics, LitStr, Result, spanned::Spanned};

use crate::irfmt::{
    Directive, Elem, FieldIdent, FmtData, Format, Lit, UnnamedVar, Var, canonical_format_for_enums,
    canonical_format_for_structs, canonical_op_format,
};

/// Data parsed from the macro to derive formatting for Rust types
pub(crate) struct FmtInput {
    pub ident: syn::Ident,
    pub generics: Generics,
    pub data: FmtData,
    pub format: Format,
}

#[derive(PartialEq, Eq)]
pub(crate) enum DeriveIRObject {
    Op,
    Attribute,
    Type,
    AnyOtherRustType,
}

/// enums can have a `#[format]` for its variants.
/// Those shouldn't be in the final output, so we remove them.
fn erase_helper_attributes(mut input: DeriveInput) -> DeriveInput {
    match input.data {
        Data::Struct(_) | Data::Union(_) => input,
        Data::Enum(ref mut data) => {
            for variant in &mut data.variants {
                variant.attrs.retain(|attr| !attr.path().is_ident("format"));
            }
            input
        }
    }
}

/// Derive the [Printable](::pliron::printable::Printable) and
/// [Parsable](::pliron::parsable::Parsable) traits for Rust types.
pub(crate) fn derive(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
    irobj: DeriveIRObject,
) -> Result<TokenStream> {
    // Parse the input to this macro.
    let input = syn::parse2::<DeriveInput>(input.into())?;

    // Prepare the format description.
    let args = Into::<TokenStream>::into(args);
    let format_data = FmtData::try_from(input.clone())?;

    let format = if let FmtData::Enum(_) = format_data {
        // Enums have only one preset format and are not customizable.
        if !args.is_empty() {
            return Err(syn::Error::new_spanned(
                input,
                "Custom format strings for enums can only be specified for individual variants"
                    .to_string(),
            ));
        }
        canonical_format_for_enums()
    } else if !args.is_empty() {
        let format_str = syn::parse2::<LitStr>(args)?;
        Format::parse(&format_str.value()).map_err(|e| syn::Error::new_spanned(format_str, e))?
    } else if irobj == DeriveIRObject::Op {
        canonical_op_format()
    } else {
        canonical_format_for_structs(&format_data, input.span())?
    };

    // Prepare data for the deriver.
    let format_input = FmtInput {
        ident: input.ident.clone(),
        generics: input.generics.clone(),
        data: format_data,
        format,
    };

    let mut derived = derive_from_parsed(format_input, irobj)?;

    // We're not in a derive macro (but an attribute macro),
    // so attach the original input back.
    derived.extend(erase_helper_attributes(input).into_token_stream());
    Ok(derived)
}

/// Derive a token stream for [Printable](::pliron::printable::Printable)
/// and [Parsable](::pliron::parsable::Parsable) traits from [FmtInput].
fn derive_from_parsed(input: FmtInput, irobj: DeriveIRObject) -> Result<TokenStream> {
    let mut format_tokens = TokenStream::new();

    let (printable, parsable) = match irobj {
        DeriveIRObject::Op => (
            DeriveOpPrintable::build(&input)?,
            DeriveOpParsable::build(&input, &mut OpParserState::default())?,
        ),
        DeriveIRObject::AnyOtherRustType => (
            DeriveBasePrintable::build(&input)?,
            DeriveBaseParsable::build(&input, &mut ())?,
        ),
        DeriveIRObject::Attribute => (
            DeriveBasePrintable::build(&input)?,
            DeriveAttributeParsable::build(&input, &mut ())?,
        ),
        DeriveIRObject::Type => (
            DeriveBasePrintable::build(&input)?,
            DeriveTypeParsable::build(&input, &mut ())?,
        ),
    };

    format_tokens.extend(printable);
    format_tokens.extend(parsable);
    Ok(format_tokens)
}

#[derive(Default)]
struct OpPrinterState {
    is_canonical: bool,
}

/// Generate token stream for derived [Printable](::pliron::printable::Printable) trait.
trait PrintableBuilder<State: Default> {
    // Entry function. Builds the outer function outline.
    fn build(input: &FmtInput) -> Result<TokenStream> {
        let name = input.ident.clone();
        let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
        let body = Self::build_body(input, &mut State::default())?;

        let derived = quote! {
            impl #impl_generics ::pliron::printable::Printable for #name #ty_generics #where_clause {
                fn fmt(
                    &self,
                    ctx: & ::pliron::context::Context,
                    state: & ::pliron::printable::State,
                    fmt: &mut ::core::fmt::Formatter<'_>,
                ) -> ::core::fmt::Result {
                    #body
                    Ok(())
                }
            }
        };
        Ok(derived)
    }

    // Build the body of the outer function Printable::fmt.
    fn build_body(input: &FmtInput, state: &mut State) -> Result<TokenStream> {
        if let FmtData::Enum(r#enum) = &input.data {
            let mut output = quote! {};
            for (format, r#struct) in &r#enum.variants {
                let variant_name = r#struct.name.clone();
                let variant_input = FmtInput {
                    ident: variant_name.clone(),
                    generics: Generics::default(),
                    data: FmtData::Struct(r#struct.clone()),
                    format: format.clone(),
                };
                let mut variant_fields = quote! {};
                if !r#struct.fields.is_empty() {
                    let mut is_named = false;
                    let mut fields = quote! {};
                    for field in &r#struct.fields {
                        let field_name = field.ident.clone();
                        match field_name {
                            FieldIdent::Named(name) => {
                                is_named = true;
                                let field = format_ident!("{}", name);
                                fields.extend(quote! {
                                    #field,
                                });
                            }
                            FieldIdent::Unnamed(index) => {
                                let field = format_ident!("field_at_{}", index);
                                fields.extend(quote! {
                                    #field,
                                });
                            }
                        }
                    }
                    if is_named {
                        variant_fields.extend(quote! {
                            { #fields }
                        });
                    } else {
                        variant_fields.extend(quote! {
                            ( #fields )
                        });
                    }
                }
                let variant_tokens = Self::build_format(&variant_input, state)?;
                output.extend(quote! {
                    Self::#variant_name #variant_fields => {
                        write!(fmt, "{}", stringify!(#variant_name))?;
                        #variant_tokens
                    },
                });
            }
            Ok(quote! {
                match self {
                    #output
                }
            })
        } else {
            Self::build_format(input, state)
        }
    }

    fn build_lit(_input: &FmtInput, _state: &mut State, lit: &str) -> TokenStream {
        quote! { ::pliron::printable::Printable::fmt(&#lit, ctx, state, fmt)?; }
    }

    fn build_format(input: &FmtInput, state: &mut State) -> Result<TokenStream> {
        input
            .format
            .elems
            .iter()
            .map(|elem| Self::build_elem(input, state, elem))
            .try_fold(TokenStream::new(), |mut acc, e| {
                acc.extend(e?);
                Ok(acc)
            })
    }

    fn build_elem(input: &FmtInput, state: &mut State, elem: &Elem) -> Result<TokenStream> {
        match elem {
            Elem::Lit(Lit { lit, .. }) => Ok(Self::build_lit(input, state, lit)),
            Elem::Var(Var { name, .. }) => Self::build_var(input, state, name),
            Elem::UnnamedVar(UnnamedVar { index, .. }) => {
                Self::build_unnamed_var(input, state, *index)
            }
            Elem::Directive(d) => Self::build_directive(input, state, d),
        }
    }

    fn build_var(input: &FmtInput, state: &mut State, name: &str) -> Result<TokenStream>;
    fn build_unnamed_var(input: &FmtInput, state: &mut State, index: usize) -> Result<TokenStream>;
    fn build_directive(input: &FmtInput, state: &mut State, d: &Directive) -> Result<TokenStream>;
}

struct DeriveBasePrintable;

impl PrintableBuilder<()> for DeriveBasePrintable {
    fn build_var(input: &FmtInput, _state: &mut (), name: &str) -> Result<TokenStream> {
        let FmtData::Struct(ref r#struct) = input.data else {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                "Only structs fields as variables are supported".to_string(),
            ));
        };
        if !r#struct
            .fields
            .iter()
            .map(|f| &f.ident)
            .any(|i| i == &FieldIdent::Named(name.to_string()))
        {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                format!("field name \"{name}\" is invalid"),
            ));
        }
        let field_name = format_ident!("{}", name);
        // If the field is an enum variant, then we don't need to access it with `&self`.
        let field = if r#struct.is_enum_variant {
            quote! { #field_name }
        } else {
            quote! { &self.#field_name }
        };
        Ok(quote! { ::pliron::printable::Printable::fmt(#field, ctx, state, fmt)?; })
    }

    fn build_unnamed_var(input: &FmtInput, _state: &mut (), index: usize) -> Result<TokenStream> {
        // This is a struct unnamed field access.
        let FmtData::Struct(ref r#struct) = input.data else {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                "Only tuple indices in structs (tuples) are supported".to_string(),
            ));
        };
        if !r#struct
            .fields
            .iter()
            .map(|f| &f.ident)
            .any(|i| i == &FieldIdent::Unnamed(index))
        {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                format!("field index \"{index}\" is invalid"),
            ));
        }
        let index = syn::Index::from(index);
        // If the field is an enum variant, then we don't need to access it with `&self`,
        // but instead need to use the index field which we gave a name to, when expanding
        // the match arm.
        let field = if r#struct.is_enum_variant {
            let field_at_index = format_ident!("field_at_{}", index);
            quote! { #field_at_index }
        } else {
            quote! { &self.#index }
        };
        Ok(quote! { ::pliron::printable::Printable::fmt(#field, ctx, state, fmt)?; })
    }

    fn build_directive(input: &FmtInput, _state: &mut (), d: &Directive) -> Result<TokenStream> {
        fn var_name_from_elem(
            elem: &Elem,
            is_enum_variant: bool,
            err: Result<TokenStream>,
        ) -> Result<TokenStream> {
            match elem {
                Elem::Var(Var { name, .. }) => {
                    let name = format_ident!("{}", name);
                    if is_enum_variant {
                        Ok(quote! { #name })
                    } else {
                        Ok(quote! { &self.#name })
                    }
                }
                Elem::UnnamedVar(idx) => {
                    if is_enum_variant {
                        let name = format_ident!("field_at_{}", idx.index);
                        Ok(quote! { #name })
                    } else {
                        let index = syn::Index::from(idx.index);
                        Ok(quote! { &self.#index })
                    }
                }
                _ => err,
            }
        }
        if d.name == "opt" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `opt` directive takes a single variable argument referring
                            to a struct or tuple field with type Option"
                    .to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let FmtData::Struct(ref r#struct) = input.data else {
                return err;
            };

            let name = var_name_from_elem(&d.args[0], r#struct.is_enum_variant, err)?;

            Ok(quote! {
                if let Some(val) = #name {
                    ::pliron::printable::Printable::fmt(val, ctx, state, fmt)?;
                }
            })
        } else if d.name == "vec" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `vec` directive takes two arguments, the first referring
                          to a struct or tuple field with type Vec, and the second one,
                          a directive specifying the separator"
                    .to_string(),
            ));
            if d.args.len() != 2 {
                return err;
            }
            let FmtData::Struct(ref r#struct) = input.data else {
                return err;
            };

            let Elem::Directive(sep) = &d.args[1] else {
                return err;
            };
            let sep = directive_to_list_separator(sep, true, input.ident.span())?;

            let name = var_name_from_elem(&d.args[0], r#struct.is_enum_variant, err)?;

            Ok(quote! {
                ::pliron::irfmt::printers::list_with_sep(#name, #sep).fmt(ctx, state, fmt)?;
            })
        } else {
            unimplemented!("Unknown directive {}", d.name)
        }
    }
}

struct DeriveOpPrintable;

impl PrintableBuilder<OpPrinterState> for DeriveOpPrintable {
    fn build_var(
        input: &FmtInput,
        _state: &mut OpPrinterState,
        attr_name: &str,
    ) -> Result<TokenStream> {
        let attr_name = attr_name.to_string();
        let op_name = input.ident.clone();
        let missing_attr_err = format!("Missing attribute {} on Op {}", &attr_name, &op_name);
        Ok(quote! {
            {
                let name: ::pliron::identifier::Identifier = #attr_name.try_into().expect("Invalid attribute name");
                let self_op = self.get_operation().deref(ctx);
                let attr = self_op.attributes.0.get(&name).expect(&#missing_attr_err);
                ::pliron::printable::Printable::fmt(attr, ctx, state, fmt)?;
            }
        })
    }

    fn build_unnamed_var(
        _input: &FmtInput,
        _state: &mut OpPrinterState,
        index: usize,
    ) -> Result<TokenStream> {
        Ok(quote! {
            let opd = self.get_operation().deref(ctx).get_operand(#index);
            ::pliron::printable::Printable::fmt(&opd, ctx, state, fmt)?;
        })
    }

    fn build_directive(
        input: &FmtInput,
        state: &mut OpPrinterState,
        d: &Directive,
    ) -> Result<TokenStream> {
        if d.name == "canonical" {
            state.is_canonical = true;
            Ok(
                quote! { ::pliron::op::canonical_syntax_print(::pliron::op::OpObj::new(*self), ctx, state, fmt)?; },
            )
        } else if d.name == "type" {
            let err = Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `type` directive takes a single unnamed variable argument to specify the result index".to_string(),
                ));
            if d.args.len() != 1 {
                return err;
            }
            if let Elem::UnnamedVar(UnnamedVar { index, .. }) = &d.args[0] {
                Ok(quote! {
                    let res = self.get_operation().deref(ctx).get_type(#index);
                    ::pliron::printable::Printable::fmt(&res, ctx, state, fmt)?;
                })
            } else {
                err
            }
        } else if d.name == "region" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `region` directive takes a single unnamed variable argument to specify the region index".to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            if let Elem::UnnamedVar(UnnamedVar { index, .. }) = &d.args[0] {
                Ok(quote! {
                    let reg = self.get_operation().deref(ctx).get_region(#index);
                    ::pliron::printable::Printable::fmt(&reg, ctx, state, fmt)?;
                })
            } else {
                err
            }
        } else if d.name == "attr" || d.name == "opt_attr" {
            let DeriveAttrDirectiveArgs {
                attr_dict_key,
                attr_type,
                label: label_opt,
                delimiters: delimiters_opt,
            } = parse_attr_directive_args(d, input)?;

            let missing_attr_err = format!(
                "Missing attribute {} on Op {}",
                &attr_dict_key,
                &input.ident.clone()
            );
            let mut res = TokenStream::new();

            let label = label_opt.map(|l| l + " : ").unwrap_or_default();
            let (starting_delimiter, ending_delimiter) = delimiters_opt.unwrap_or_default();

            res.extend(quote! {
                let self_op = self.get_operation().deref(ctx);
                let attr = self_op.attributes.get::<#attr_type>(
                    &::pliron::identifier::Identifier::try_from(#attr_dict_key).unwrap()
                );
                if let Some(attr) = attr {
                    write!(fmt, "{}{}", #starting_delimiter, #label)?;
                    ::pliron::printable::Printable::fmt(attr, ctx, state, fmt)?;
                    write!(fmt, "{}", #ending_delimiter)?;
                }
            });
            if d.name == "attr" {
                // For `attr`, we need to error if the attribute is missing.
                res.extend(quote! {
                    else {
                        panic!("{}", &#missing_attr_err);
                    }
                });
            }

            Ok(res)
        } else if d.name == "succ" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `succ` directive takes a single unnamed variable argument to specify the successor index".to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let Elem::UnnamedVar(UnnamedVar { index, .. }) = &d.args[0] else {
                return err;
            };
            Ok(quote! {
                let succ = self.get_operation().deref(ctx).get_successor(#index);
                let succ_name = ::pliron::alloc::string::ToString::to_string("^") + &succ.unique_name(ctx);
                ::pliron::printable::Printable::fmt(&succ_name, ctx, state, fmt)?;
            })
        } else if d.name == "successors" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `successors` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                    .to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let Elem::Directive(sep) = &d.args[0] else {
                return err;
            };
            let sep = directive_to_list_separator(sep, true, input.ident.span())?;
            Ok(quote! {
                let op = self.get_operation().deref(ctx);
                let succs = op.successors().map(|succ| ::pliron::alloc::string::ToString::to_string("^") + &succ.unique_name(ctx));
                let succs = ::pliron::irfmt::printers::iter_with_sep(succs, #sep);
                ::pliron::printable::Printable::fmt(&succs, ctx, state, fmt)?;
            })
        } else if d.name == "regions" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `regions` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                    .to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let Elem::Directive(sep) = &d.args[0] else {
                return err;
            };
            let sep = directive_to_list_separator(sep, true, input.ident.span())?;
            Ok(quote! {
                let op = self.get_operation().deref(ctx);
                let regions = op.regions();
                let regions = ::pliron::irfmt::printers::iter_with_sep(regions, #sep);
                ::pliron::printable::Printable::fmt(&regions, ctx, state, fmt)?;
            })
        } else if d.name == "operands" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `operands` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                    .to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let Elem::Directive(sep) = &d.args[0] else {
                return err;
            };
            let sep = directive_to_list_separator(sep, true, input.ident.span())?;
            Ok(quote! {
                let op = self.get_operation().deref(ctx);
                let operands = op.operands();
                let operands = ::pliron::irfmt::printers::iter_with_sep(operands, #sep);
                ::pliron::printable::Printable::fmt(&operands, ctx, state, fmt)?;
            })
        } else if d.name == "types" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `types` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                    .to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let Elem::Directive(sep) = &d.args[0] else {
                return err;
            };
            let sep = directive_to_list_separator(sep, true, input.ident.span())?;
            Ok(quote! {
                let op = self.get_operation().deref(ctx);
                let types = op.result_types();
                let types = ::pliron::irfmt::printers::iter_with_sep(types, #sep);
                ::pliron::printable::Printable::fmt(&types, ctx, state, fmt)?;
            })
        } else if d.name == "attr_dict" {
            Ok(quote! {
                let self_op = self.get_operation().deref(ctx);
                ::pliron::printable::Printable::fmt(&self_op.attributes, ctx, state, fmt)?;
            })
        } else {
            unimplemented!("Unknown directive {}", d.name)
        }
    }

    fn build_body(input: &FmtInput, state: &mut OpPrinterState) -> Result<TokenStream> {
        let formatted_tokens = Self::build_format(input, state)?;
        let mut output = quote! {};
        if !state.is_canonical {
            output.extend(quote! {
                use ::pliron::op::Op;
                use ::pliron::irfmt::printers::iter_with_sep;
                use ::pliron::common_traits::Named;
                let op = self.get_operation().deref(ctx);
                if op.get_num_results() > 0 {
                    let sep = ::pliron::printable::ListSeparator::CharSpace(',');
                    let results = iter_with_sep(op.results(), sep);
                    write!(fmt, "{} = ", results.disp(ctx))?;
                }
                write!(fmt, "{} ", self.get_opid())?;
            });
        }
        output.extend(formatted_tokens);
        Ok(output)
    }
}

/// Generate token stream for derived [Printable](::pliron::printable::Printable) trait.
trait ParsableBuilder<State: Default> {
    /// Entry point to build a parser.
    fn build(input: &FmtInput, state: &mut State) -> Result<TokenStream> {
        let name = input.ident.clone();
        let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
        let body = Self::build_body(input, state)?;
        let final_ret_value = Self::build_final_ret_value(input, state);

        let arg = Self::build_assoc_type_arg(input, state)?;
        let parsed = Self::build_assoc_type_parsed(input, state)?;
        let derived = quote! {
            impl #impl_generics ::pliron::parsable::Parsable for #name #ty_generics #where_clause {
                type Arg = #arg;
                type Parsed = #parsed;
                fn parse<'__pliron_parse>(
                    state_stream: &mut ::pliron::parsable::StateStream<'__pliron_parse>,
                    arg: Self::Arg,
                ) -> ::pliron::parsable::ParseResult<'__pliron_parse, Self::Parsed> {
                    use ::pliron::parsable::IntoParseResult;
                    use ::pliron::combine::Parser;
                    use ::pliron::input_err;
                    use ::pliron::location::Located;
                    let cur_loc = state_stream.loc();

                    #body
                    #final_ret_value
                }
            }
        };
        Ok(derived)
    }

    /// Default implementation for building the parser's associated type for `Arg`.
    fn build_assoc_type_arg(_input: &FmtInput, _state: &mut State) -> Result<TokenStream> {
        Ok(quote! { () })
    }

    /// Default implementation for building the parser's associated type for `Parsed`.
    fn build_assoc_type_parsed(_input: &FmtInput, _state: &mut State) -> Result<TokenStream> {
        Ok(quote! { Self })
    }

    fn build_lit(_input: &FmtInput, lit: &str, _state: &mut State) -> TokenStream {
        let trimmed_lit = lit.trim();
        quote! {
            ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(#trimmed_lit))
            .parse_stream(state_stream)
            .into_result()?;
        }
    }

    fn build_format(input: &FmtInput, state: &mut State) -> Result<TokenStream> {
        input
            .format
            .elems
            .iter()
            .map(|elem| Self::build_elem(input, state, elem))
            .try_fold(TokenStream::new(), |mut acc, e| {
                acc.extend(e?);
                Ok(acc)
            })
    }

    fn build_elem(input: &FmtInput, state: &mut State, elem: &Elem) -> Result<TokenStream> {
        match elem {
            Elem::Lit(Lit { lit, .. }) => Ok(Self::build_lit(input, lit, state)),
            Elem::Var(Var { name, .. }) => Self::build_var(input, state, name),
            Elem::UnnamedVar(UnnamedVar { index, .. }) => {
                Self::build_unnamed_var(input, state, *index)
            }
            Elem::Directive(d) => Self::build_directive(input, state, d),
        }
    }

    fn build_body(input: &FmtInput, state: &mut State) -> Result<TokenStream> {
        if let FmtData::Enum(r#enum) = &input.data {
            let enum_name = r#enum.name.clone();
            assert!(!r#enum.variants.is_empty(), "Enum has no variants");
            let variant_name_parsed = quote! {
                let variant_name_parsed =
                    ::pliron::alloc::string::ToString::to_string(
                        &::pliron::identifier::Identifier::parser(()).
                            parse_stream(state_stream).into_result()?.0
                    );
            };

            let mut match_arms = quote! {};
            for (format, r#struct) in &r#enum.variants {
                let variant_name = r#struct.name.clone();
                let variant_name_str = variant_name.to_string();
                let parsed_variant = if r#struct.fields.is_empty() {
                    quote! {
                        // Could as well use Self::#variant_name here.
                        #enum_name::#variant_name
                    }
                } else {
                    let variant_input = FmtInput {
                        ident: r#struct.name.clone(),
                        generics: Generics::default(),
                        data: FmtData::Struct(r#struct.clone()),
                        format: format.clone(),
                    };
                    let built_body = Self::build_body(&variant_input, state)?;
                    quote! {
                        #built_body
                        final_ret_value
                    }
                };
                match_arms.extend(quote! {
                    #variant_name_str => {
                        #parsed_variant
                    },
                });
            }
            return Ok(quote! {
                #variant_name_parsed
                let final_ret_value = match variant_name_parsed.as_str() {
                    #match_arms
                    _ => {
                        return input_err!(
                            cur_loc.clone(),
                            "Invalid variant name: {}", variant_name_parsed
                        )?;
                    }
                };
            });
        }

        let mut processed_fmt = Self::build_format(input, state)?;
        let FmtData::Struct(r#struct) = &input.data else {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                "Only structs are supported by default impl of ParsableBuilder".to_string(),
            ));
        };
        let name = &r#struct.name;
        let mut named = true;

        let mut obj_builder = quote! {};
        for field in &r#struct.fields {
            match &field.ident {
                FieldIdent::Named(name) => {
                    let field_name = format_ident!("{}", name);
                    obj_builder.extend(quote! {
                        #field_name,
                    });
                }
                FieldIdent::Unnamed(index) => {
                    let field_name = format_ident!("field_at_{}", index);
                    obj_builder.extend(quote! {
                        #field_name,
                    });
                    named = false;
                }
            }
        }

        let name_prefix = if r#struct.is_enum_variant {
            quote! { Self:: }
        } else {
            quote! {}
        };
        let final_ret_value = if named {
            quote! {
                let final_ret_value = #name_prefix #name {
                    #obj_builder
                };
            }
        } else {
            quote! {
                let final_ret_value = #name_prefix #name (
                    #obj_builder
                );
            }
        };

        processed_fmt.extend(quote! {
            #final_ret_value
        });
        Ok(processed_fmt)
    }

    fn build_var(input: &FmtInput, _state: &mut State, name: &str) -> Result<TokenStream> {
        let FmtData::Struct(r#struct) = &input.data else {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                "Only structs fields as variables are supported".to_string(),
            ));
        };

        let Some(crate::irfmt::Field { ty, .. }) = r#struct
            .fields
            .iter()
            .find(|f| matches!(&f.ident, FieldIdent::Named(field_name) if name == field_name))
        else {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                format!("{name} field not found"),
            ));
        };
        let name = format_ident!("{}", name);
        Ok(quote! {
            let #name = <#ty>::parser(()).parse_stream(state_stream).into_result()?.0;
        })
    }

    fn build_unnamed_var(
        input: &FmtInput,
        _state: &mut State,
        index: usize,
    ) -> Result<TokenStream> {
        let FmtData::Struct(r#struct) = &input.data else {
            return Err(syn::Error::new_spanned(
                input.ident.clone(),
                "Only tuple indices in structs (tuples) are supported".to_string(),
            ));
        };

        let crate::irfmt::Field { ty, .. } = r#struct.fields.get(index).ok_or_else(|| {
            syn::Error::new_spanned(
                input.ident.clone(),
                format!("field index \"{index}\" is invalid"),
            )
        })?;
        let name = format_ident!("field_at_{}", index);
        Ok(quote! {
            let #name = <#ty>::parser(()).parse_stream(state_stream).into_result()?.0;
        })
    }

    fn build_directive(input: &FmtInput, _state: &mut State, d: &Directive) -> Result<TokenStream> {
        if d.name == "opt" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `opt` directive takes a single variable argument referring
                            to a struct or tuple field with type Option"
                    .to_string(),
            ));
            if d.args.len() != 1 {
                return err;
            }
            let FmtData::Struct(ref r#struct) = input.data else {
                return err;
            };

            let (name, ty) = match &d.args[0] {
                Elem::Var(Var { name, .. }) => {
                    let name = format_ident!("{}", name);
                    let Some(crate::irfmt::Field { ty, .. }) = r#struct.fields.iter().find(
                        |f| matches!(&f.ident, FieldIdent::Named(field_name) if name == field_name),
                    ) else {
                        return Err(syn::Error::new_spanned(
                            input.ident.clone(),
                            format!("{name} field not found"),
                        ));
                    };
                    (name, ty)
                }
                Elem::UnnamedVar(UnnamedVar { pos: _, index }) => {
                    let name = format_ident!("field_at_{}", index);
                    let crate::irfmt::Field { ty, .. } =
                        r#struct.fields.get(*index).ok_or_else(|| {
                            syn::Error::new_spanned(
                                input.ident.clone(),
                                format!("field index \"{}\" is invalid", *index),
                            )
                        })?;
                    (name, ty)
                }
                _ => {
                    return err;
                }
            };
            let inner_ty = get_inner_type_option_vec(ty)?;
            Ok(quote! {
                let #name = ::pliron::combine::parser::choice::optional(<#inner_ty>::parser(()))
                    .parse_stream(state_stream).into_result()?.0;
            })
        } else if d.name == "vec" {
            let err = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `vec` directive takes two arguments, the first referring
                          to a struct or tuple field with type Vec, and the second one,
                          a directive specifying the separator"
                    .to_string(),
            ));
            if d.args.len() != 2 {
                return err;
            }
            let FmtData::Struct(ref r#struct) = input.data else {
                return err;
            };

            let (name, ty) = match &d.args[0] {
                Elem::Var(Var { name, .. }) => {
                    let name = format_ident!("{}", name);
                    let Some(crate::irfmt::Field { ty, .. }) = r#struct.fields.iter().find(
                        |f| matches!(&f.ident, FieldIdent::Named(field_name) if name == field_name),
                    ) else {
                        return Err(syn::Error::new_spanned(
                            input.ident.clone(),
                            format!("{name} field not found"),
                        ));
                    };
                    (name, ty)
                }
                Elem::UnnamedVar(UnnamedVar { pos: _, index }) => {
                    let name = format_ident!("field_at_{}", index);
                    let crate::irfmt::Field { ty, .. } =
                        r#struct.fields.get(*index).ok_or_else(|| {
                            syn::Error::new_spanned(
                                input.ident.clone(),
                                format!("field index \"{}\" is invalid", *index),
                            )
                        })?;
                    (name, ty)
                }
                _ => {
                    return err;
                }
            };

            let Elem::Directive(sep) = &d.args[1] else {
                return err;
            };
            let sep = directive_to_list_separator(sep, false, input.ident.span())?;

            let inner_ty = get_inner_type_option_vec(ty)?;
            Ok(quote! {
                let parsed_vec: ::pliron::alloc::vec::Vec<_> = ::pliron::irfmt::parsers::list_parser(#sep, <#inner_ty>::parser(()))
                    .parse_stream(state_stream).into_result()?.0;
                let #name: #ty = parsed_vec.try_into().map_err(|_vec| {
                    ::pliron::input_error!(
                        cur_loc.clone(),
                        "Failed to parse into {}: incorrect length", stringify!(#ty)
                    )
                })?;
            })
        } else {
            unimplemented!("Unknown directive {}", d.name)
        }
    }

    /// After the entire body is built, build the parsed value to be finally returned.
    fn build_final_ret_value(_input: &FmtInput, _state: &mut State) -> TokenStream {
        quote! {
            Ok(final_ret_value).into_parse_result()
        }
    }
}

/// Basic parsable builder for any Rust type.
/// Builds an object of that type.
/// Requires all contents to be parsable themselves.
struct DeriveBaseParsable;

impl ParsableBuilder<()> for DeriveBaseParsable {}

/// Specify various elements individually or all at once.
enum ElementSpec<T> {
    Individual(FxHashMap<T, syn::Ident>),
    All(syn::Ident),
}

impl<T> Default for ElementSpec<T> {
    fn default() -> Self {
        ElementSpec::Individual(FxHashMap::default())
    }
}

#[derive(Default)]
struct OpParserState {
    is_canonical: bool,
    // If there is a region(s) specified in the syntax,
    // we build a temporary Op to hold the region(s) before
    // building the actual Op.
    regions_temp_parent_op: Option<syn::Ident>,
    operands: ElementSpec<usize>,
    successors: ElementSpec<usize>,
    result_types: ElementSpec<usize>,
    // The second element specifies attribtues that are specified as optional.
    attributes: (ElementSpec<String>, FxHashSet<String>),
    regions: ElementSpec<usize>,
}

struct DeriveOpParsable;

impl ParsableBuilder<OpParserState> for DeriveOpParsable {
    fn build_body(input: &FmtInput, state: &mut OpParserState) -> Result<TokenStream> {
        let mut output = quote! {
            use ::pliron::op::Op;
            use ::pliron::operation::Operation;
            use ::pliron::irfmt::parsers::
                {process_parsed_ssa_defs, ssa_opd_parser, block_opd_parser, attr_parser};
        };

        // Is region parsing specified as part of the syntax?
        fn has_regions(input: &FmtInput) -> bool {
            input.format.elems.iter().any(|f| {
                matches!(f, Elem::Directive(directive)
                        if directive.name == "region" || directive.name == "regions")
            })
        }

        if has_regions(input) {
            // For Operations with regions, we create a temporary Op of the same type
            // (with no operands, successors, etc.) to hold the regions before
            // building the actual Op. This temporary Op is later erased.
            let regions_temp_parent_op = format_ident!("regions_temp_parent_op");
            state.regions_temp_parent_op = Some(regions_temp_parent_op.clone());
            output.extend(quote! {
                let #regions_temp_parent_op = Operation::new(
                    state_stream.state.ctx,
                    Self::get_concrete_op_info(),
                    ::pliron::alloc::vec![],
                    ::pliron::alloc::vec![],
                    ::pliron::alloc::vec![],
                    0,
                );
            });
        }

        let built_format = Self::build_format(input, state)?;
        output.extend(built_format);

        if state.is_canonical {
            return Ok(output);
        }

        let operands = match state.operands {
            ElementSpec::Individual(ref operands) => {
                let num_operands = operands.keys().map(|idx| idx + 1).max().unwrap_or_default();
                for i in 0..num_operands {
                    if !operands.contains_key(&i) {
                        return Err(syn::Error::new_spanned(
                            input.ident.clone(),
                            format!("missing operand {i}"),
                        ));
                    }
                }
                let operand_indices = (0..num_operands).map(|i| operands[&i].clone());
                quote! {
                    ::pliron::alloc::vec![#( #operand_indices ),*]
                }
            }
            ElementSpec::All(ref all) => {
                quote! {
                    #all
                }
            }
        };

        let successors = match state.successors {
            ElementSpec::Individual(ref successors) => {
                let num_successors = successors
                    .keys()
                    .map(|idx| idx + 1)
                    .max()
                    .unwrap_or_default();
                for i in 0..num_successors {
                    if !successors.contains_key(&i) {
                        return Err(syn::Error::new_spanned(
                            input.ident.clone(),
                            format!("missing successor {i}"),
                        ));
                    }
                }
                let successor_indices = (0..num_successors).map(|i| successors[&i].clone());
                quote! {
                    ::pliron::alloc::vec![#( #successor_indices ),*]
                }
            }
            ElementSpec::All(ref all) => {
                quote! {
                    #all
                }
            }
        };

        let regions = match state.regions {
            ElementSpec::Individual(ref regions) => {
                let num_regions = regions.keys().map(|idx| idx + 1).max().unwrap_or_default();
                for i in 0..num_regions {
                    if !regions.contains_key(&i) {
                        return Err(syn::Error::new_spanned(
                            input.ident.clone(),
                            format!("missing region {i}"),
                        ));
                    }
                }
                let region_indices = (0..num_regions).map(|i| regions[&i].clone());
                quote! {
                    ::pliron::alloc::vec![#( #region_indices ),*]
                }
            }
            ElementSpec::All(ref all) => {
                quote! {
                    #all
                }
            }
        };

        let results = match state.result_types {
            ElementSpec::Individual(ref result_types) => {
                let num_result_types = result_types
                    .keys()
                    .map(|idx| idx + 1)
                    .max()
                    .unwrap_or_default();
                let result_type_indices = (0..num_result_types).map(|i| result_types[&i].clone());
                quote! {
                    ::pliron::alloc::vec![#( #result_type_indices ),*]
                }
            }
            ElementSpec::All(ref all) => {
                quote! {
                    #all
                }
            }
        };

        let results_var = format_ident!("results");
        // Assing the result types to a new variable to avoid
        // re-evaluating the vec![] expression multiple times.
        output.extend(quote! {
            let #results_var = #results;
        });

        let mut attribute_sets = quote! {};
        match &state.attributes.0 {
            ElementSpec::Individual(attributes) => {
                for (attr_name, attr_ident) in attributes {
                    let set_attr = if state.attributes.1.contains(attr_name) {
                        // This is an optional attribute.
                        quote! {
                            if let Some(#attr_ident) = #attr_ident {
                                op.deref_mut(state_stream.state.ctx).attributes.0.insert(
                                    ::pliron::identifier::Identifier::try_from(#attr_name).unwrap(),
                                    ::pliron::alloc::boxed::Box::new(#attr_ident),
                                );
                            }
                        }
                    } else {
                        quote! {
                            op.deref_mut(state_stream.state.ctx).attributes.0.insert(
                                ::pliron::identifier::Identifier::try_from(#attr_name).unwrap(),
                                #attr_ident,
                            );
                        }
                    };
                    attribute_sets.extend(set_attr);
                }
            }
            ElementSpec::All(attr_sets_name) => {
                attribute_sets = quote! {
                    op.deref_mut(state_stream.state.ctx).attributes = #attr_sets_name;
                }
            }
        }

        let results_check = quote! {
            if arg.len() != #results_var.len() {
                return
                    input_err!(cur_loc,
                        "expected {} results as per spec, got {} during parsing",
                        #results_var.len(),
                        arg.len()
                    )?;
            }
        };

        output.extend(results_check);

        output.extend(quote! {
            let op = ::pliron::operation::Operation::new(
                state_stream.state.ctx,
                Self::get_concrete_op_info(),
                #results_var,
                #operands,
                #successors,
                0,        // regions
            );

            for region in #regions {
                ::pliron::region::Region::move_to_op(region, op, state_stream.state.ctx);
            }

            if !arg.is_empty() {
                process_parsed_ssa_defs(state_stream, &arg, op)?;
            }
        });

        if let Some(regions_temp_parent_op) = &state.regions_temp_parent_op {
            output.extend(quote! {
                Operation::erase(#regions_temp_parent_op, state_stream.state.ctx);
            });
        }

        output.extend(quote! {
            let final_ret_value = Operation::get_op_dyn(op, state_stream.state.ctx);
        });

        output.extend(attribute_sets);

        Ok(output)
    }

    fn build_var(
        _input: &FmtInput,
        state: &mut OpParserState,
        attr_name: &str,
    ) -> Result<TokenStream> {
        let attr_name = attr_name.to_string();
        let attr_name_ident = format_ident!("{}", attr_name);

        match state.attributes.0 {
            ElementSpec::Individual(ref mut attributes) => {
                attributes.insert(attr_name.clone(), attr_name_ident.clone());
            }
            ElementSpec::All(_) => {
                return Err(syn::Error::new_spanned(
                    _input.ident.clone(),
                    "Cannot mix attributes directive with named attributes".to_string(),
                ));
            }
        }

        Ok(quote! {
            let #attr_name_ident = attr_parser()
                .parse_stream(state_stream)
                .into_result()?
                .0;
        })
    }

    fn build_unnamed_var(
        _input: &FmtInput,
        state: &mut OpParserState,
        index: usize,
    ) -> Result<TokenStream> {
        let opd_name = format_ident!("opd_{}", index);
        match state.operands {
            ElementSpec::Individual(ref mut operands) => {
                operands.insert(index, opd_name.clone());
            }
            ElementSpec::All(_) => {
                return Err(syn::Error::new_spanned(
                    _input.ident.clone(),
                    "Cannot mix operands directive with unnamed operands".to_string(),
                ));
            }
        }
        Ok(quote! {
            let #opd_name = ssa_opd_parser().parse_stream(state_stream).into_result()?.0;
        })
    }

    fn build_directive(
        input: &FmtInput,
        state: &mut OpParserState,
        d: &Directive,
    ) -> Result<TokenStream> {
        if d.name == "canonical" {
            state.is_canonical = true;
            Ok(quote! {
                ::pliron::op::canonical_syntax_parser::<Self>(
                    arg,
                )
                .parse_stream(state_stream)
                .into()
            })
        } else if d.name == "type" {
            let err = Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `type` directive takes a single unnamed variable argument to specify the result index".to_string(),
                ));
            if d.args.len() != 1 {
                return err;
            }
            if let Elem::UnnamedVar(UnnamedVar { index, .. }) = &d.args[0] {
                let res_type = format_ident!("res_{}", index);
                match state.result_types {
                    ElementSpec::Individual(ref mut result_types) => {
                        result_types.insert(*index, res_type.clone());
                    }
                    ElementSpec::All(_) => {
                        return Err(syn::Error::new_spanned(
                            input.ident.clone(),
                            "Cannot mix result types directive with numbered result types"
                                .to_string(),
                        ));
                    }
                }
                Ok(quote! {
                    let #res_type = ::pliron::irfmt::parsers::type_parser().parse_stream(state_stream).into_result()?.0;
                })
            } else {
                err
            }
        } else if d.name == "region" {
            let Some(Elem::UnnamedVar(UnnamedVar { index: reg_idx, .. })) = &d.args.first() else {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `region` directive takes a single unnamed variable argument to specify the region index".to_string(),
                ));
            };

            let reg_name = format_ident!("reg_{}", *reg_idx);
            match state.regions {
                ElementSpec::Individual(ref mut regions) => {
                    regions.insert(*reg_idx, reg_name.clone());
                }
                ElementSpec::All(_) => {
                    return Err(syn::Error::new_spanned(
                        input.ident.clone(),
                        "Cannot mix regions directive with numbered regions".to_string(),
                    ));
                }
            }

            let regions_temp_parent_op = state
                .regions_temp_parent_op
                .as_ref()
                .expect("There is a region directive but a temporary parent Op is not created");

            Ok(quote! {
                let #reg_name = ::pliron::region::Region::parser
                    (#regions_temp_parent_op).parse_stream(state_stream).into_result()?.0;
            })
        } else if d.name == "attr" || d.name == "opt_attr" {
            let DeriveAttrDirectiveArgs {
                attr_dict_key,
                attr_type,
                label: label_opt,
                delimiters: delimiters_opt,
            } = parse_attr_directive_args(d, input)?;
            let attr_name_ident = format_ident!("{}", attr_dict_key);

            match &mut state.attributes {
                (ElementSpec::Individual(attributes), optionals) => {
                    attributes.insert(attr_dict_key.clone(), attr_name_ident.clone());
                    if d.name == "opt_attr" {
                        optionals.insert(attr_dict_key.clone());
                    }
                }
                (ElementSpec::All(_), _optionals) => {
                    return Err(syn::Error::new_spanned(
                        input.ident.clone(),
                        "Cannot mix attributes directive with named attributes".to_string(),
                    ));
                }
            }

            let attr_parser = quote! {
                #attr_type::parser(())
            };
            let labelled_parser = if let Some(label) = &label_opt {
                quote! {
                (::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(#label))
                    .skip(::pliron::combine::parser::char::char(':').skip(::pliron::combine::parser::char::spaces()))).with(#attr_parser)
                }
            } else {
                attr_parser
            };
            let delimited_labelled_parser = if let Some((open, close)) = &delimiters_opt {
                quote! {
                    ::pliron::combine::parser::sequence::between(
                        ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(#open)),
                        ::pliron::irfmt::parsers::spaced(::pliron::combine::parser::char::string(#close)),
                        #labelled_parser
                    )
                }
            } else {
                labelled_parser
            };

            if d.name == "opt_attr" {
                Ok(quote! {
                    let #attr_name_ident = ::pliron::combine::parser::choice::optional
                        (#delimited_labelled_parser).parse_stream(state_stream).into_result()?.0;
                })
            } else {
                Ok(quote! {
                    let parsed = #delimited_labelled_parser
                        .parse_stream(state_stream).into_result()?.0;
                    let #attr_name_ident = ::pliron::alloc::boxed::Box::new(parsed);
                })
            }
        } else if d.name == "succ" {
            let Some(Elem::UnnamedVar(UnnamedVar { index, .. })) = &d.args.first() else {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `succ` directive takes a single unnamed variable argument to specify the successor index".to_string(),
                ));
            };

            let succ_name = format_ident!("succ_{}", index);
            match state.successors {
                ElementSpec::Individual(ref mut successors) => {
                    successors.insert(*index, succ_name.clone());
                }
                ElementSpec::All(_) => {
                    return Err(syn::Error::new_spanned(
                        input.ident.clone(),
                        "Cannot mix successors directive with numbered successors".to_string(),
                    ));
                }
            }
            Ok(quote! {
                let #succ_name = block_opd_parser().parse_stream(state_stream).into_result()?.0;
            })
        } else if d.name == "regions" {
            let Some(Elem::Directive(sep)) = &d.args.first() else {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `regions` directive takes a single argument to specify the separator.
                        Refer to the documentation for details"
                        .to_string(),
                ));
            };
            let sep = directive_to_list_separator(sep, false, input.ident.span())?;
            let regions_var_name = format_ident!("{}", "regions");
            if matches!(&state.regions, ElementSpec::Individual(regions) if !regions.is_empty()) {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "Cannot mix regions directive with numbered regions".to_string(),
                ));
            }
            state.regions = ElementSpec::All(regions_var_name.clone());
            let regions_temp_parent_op = state
                .regions_temp_parent_op
                .as_ref()
                .expect("There is a region directive but a temporary parent Op is not created");

            Ok(quote! {
                let #regions_var_name =
                    ::pliron::irfmt::parsers::list_parser
                        (#sep, ::pliron::region::Region::parser(#regions_temp_parent_op))
                    .parse_stream(state_stream)
                    .into_result()?
                    .0;
            })
        } else if d.name == "successors" {
            let Some(Elem::Directive(sep)) = &d.args.first() else {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `successors` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                        .to_string(),
                ));
            };
            let sep = directive_to_list_separator(sep, false, input.ident.span())?;
            let succ_var_name = format_ident!("{}", "successors");
            if matches!(&state.successors, ElementSpec::Individual(successors) if !successors.is_empty())
            {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "Cannot mix successors directive with numbered successors".to_string(),
                ));
            }
            state.successors = ElementSpec::All(succ_var_name.clone());
            Ok(quote! {
                let #succ_var_name = ::pliron::irfmt::parsers::list_parser(#sep, block_opd_parser())
                    .parse_stream(state_stream)
                    .into_result()?
                    .0;
            })
        } else if d.name == "operands" {
            let Some(Elem::Directive(sep)) = &d.args.first() else {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `operands` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                        .to_string(),
                ));
            };
            let sep = directive_to_list_separator(sep, false, input.ident.span())?;
            let operands_var_name = format_ident!("{}", "operands");
            if matches!(&state.operands, ElementSpec::Individual(operands) if !operands.is_empty())
            {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "Cannot mix operands directive with numbered operands".to_string(),
                ));
            }
            state.operands = ElementSpec::All(operands_var_name.clone());
            Ok(quote! {
                let #operands_var_name = ::pliron::irfmt::parsers::list_parser(#sep, ssa_opd_parser())
                    .parse_stream(state_stream)
                    .into_result()?
                    .0;
            })
        } else if d.name == "types" {
            let Some(Elem::Directive(sep)) = &d.args.first() else {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The `types` directive takes a single argument to specify the separator.
                    Refer to the documentation for details"
                        .to_string(),
                ));
            };
            let sep = directive_to_list_separator(sep, false, input.ident.span())?;
            let result_types_var_name = format_ident!("{}", "result_types");
            if matches!(&state.result_types, ElementSpec::Individual(result_types) if !result_types.is_empty())
            {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "Cannot mix result types directive with numbered result types".to_string(),
                ));
            }
            state.result_types = ElementSpec::All(result_types_var_name.clone());
            Ok(quote! {
                let #result_types_var_name = ::pliron::irfmt::parsers::list_parser(#sep, ::pliron::irfmt::parsers::type_parser())
                    .parse_stream(state_stream)
                    .into_result()?
                    .0;
            })
        } else if d.name == "attr_dict" {
            let attr_sets_name = format_ident!("attr_sets");
            if matches!(&state.attributes.0, ElementSpec::Individual(attributes) if !attributes.is_empty())
            {
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "Cannot mix attributes directive with named attributes".to_string(),
                ));
            }
            state.attributes.0 = ElementSpec::All(attr_sets_name.clone());
            Ok(quote! {
                let #attr_sets_name =
                    ::pliron::attribute::AttributeDict::parser(()).parse_stream(state_stream).into_result()?.0;
            })
        } else {
            unimplemented!("Unknown directive {}", d.name)
        }
    }

    fn build_final_ret_value(_input: &FmtInput, state: &mut OpParserState) -> TokenStream {
        if !state.is_canonical {
            quote! {
                Ok(final_ret_value).into_parse_result()
            }
        } else {
            quote! {}
        }
    }

    /// Default implementation for building the parser's associated type for `Arg`.
    fn build_assoc_type_arg(_input: &FmtInput, _state: &mut OpParserState) -> Result<TokenStream> {
        Ok(
            quote! { ::pliron::alloc::vec::Vec<(::pliron::identifier::Identifier, ::pliron::location::Location)> },
        )
    }

    /// Default implementation for building the parser's associated type for `Parsed`.
    fn build_assoc_type_parsed(
        _input: &FmtInput,
        _state: &mut OpParserState,
    ) -> Result<TokenStream> {
        Ok(quote! { ::pliron::op::OpObj })
    }
}

struct DeriveAttributeParsable;

impl ParsableBuilder<()> for DeriveAttributeParsable {}

struct DeriveTypeParsable;

impl ParsableBuilder<()> for DeriveTypeParsable {
    fn build_assoc_type_parsed(_input: &FmtInput, _state: &mut ()) -> Result<TokenStream> {
        Ok(quote! { ::pliron::r#type::TypePtr<Self> })
    }

    fn build_final_ret_value(_input: &FmtInput, _state: &mut ()) -> TokenStream {
        quote! {
            Ok(::pliron::r#type::Type::register_instance(final_ret_value, state_stream.state.ctx)).into_parse_result()
        }
    }
}

/// Arguments for the `attr` and `opt_attr` directives.
struct DeriveAttrDirectiveArgs {
    attr_dict_key: String,
    attr_type: syn::Type,
    label: Option<String>,
    delimiters: Option<(String, String)>,
}

fn parse_attr_directive_args(d: &Directive, input: &FmtInput) -> Result<DeriveAttrDirectiveArgs> {
    if d.args.len() < 2 {
        return Err(syn::Error::new_spanned(
            input.ident.clone(),
            "The `attr` directive takes two mandatory arguments. \
            Check the documentation for details"
                .to_string(),
        ));
    }

    let mut args = d.args.iter();
    let Elem::Var(Var {
        name: attr_dict_key,
        ..
    }) = args.next().unwrap()
    else {
        return Err(syn::Error::new_spanned(
            input.ident.clone(),
            "The first argument to `attr` directive must be a named variable representing its name"
                .to_string(),
        ));
    };

    let attr_type =
        match args.next().unwrap() {
            Elem::Var(Var { name, .. }) => name.clone(),
            Elem::Lit(lit) => {
                lit.lit.clone()
            }
            _ =>
                return Err(syn::Error::new_spanned(
                    input.ident.clone(),
                    "The second argument to `attr` directive must be a named variable or a literal representing its type".to_string(),
                ))
        };

    let mut label = None;
    let mut delimiters = None;

    let mut process_arg = |arg: &Elem| -> Result<()> {
        let err_arg = Err(syn::Error::new_spanned(
            input.ident.clone(),
            "Unexpected argument to `attr` directive".to_string(),
        ));
        let Elem::Directive(directive) = arg else {
            return err_arg;
        };
        if directive.name == "label" {
            let err_args = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `label` directive takes a single named variable argument".to_string(),
            ));
            if directive.args.len() != 1 {
                return err_args;
            }
            let Elem::Var(Var {
                name: label_var, ..
            }) = &directive.args[0]
            else {
                return err_args;
            };
            label = Some(label_var.clone());
        } else if directive.name == "delimiters" {
            let err_args = Err(syn::Error::new_spanned(
                input.ident.clone(),
                "The `delimiters` directive takes two literal arguments".to_string(),
            ));
            if directive.args.len() != 2 {
                return err_args;
            }
            let Elem::Lit(Lit { lit: open_lit, .. }) = &directive.args[0] else {
                return err_args;
            };
            let Elem::Lit(Lit { lit: close_lit, .. }) = &directive.args[1] else {
                return err_args;
            };
            delimiters = Some((open_lit.clone(), close_lit.clone()));
        } else {
            return err_arg;
        }
        Ok(())
    };

    // Process 3rd and 4th arguments if present.
    if let Some(arg) = args.next() {
        process_arg(arg)?;
    }
    if let Some(arg) = args.next() {
        process_arg(arg)?;
    }

    let attr_type = syn::parse_str::<syn::Type>(&attr_type)?;
    Ok(DeriveAttrDirectiveArgs {
        attr_dict_key: attr_dict_key.clone(),
        attr_type,
        label,
        delimiters,
    })
}

use syn::{
    AngleBracketedGenericArguments, GenericArgument, Path, PathArguments, PathSegment, Type,
    TypePath,
};

/// For [Option], [Vec], and array types, get the inner type.
/// This is based on the guidance in
/// [the procedural macro workshop](https://github.com/dtolnay/proc-macro-workshop/blob/master/builder/tests/06-optional-field.rs).
fn get_inner_type_option_vec(ty: &Type) -> Result<Type> {
    let err = Err(syn::Error::new_spanned(
        ty.clone(),
        "Expected an array or an Option or Vec type".to_string(),
    ));

    if let Type::Array(arr) = ty {
        return Ok(*arr.elem.clone());
    }

    if let Type::Path(TypePath {
        qself: None,
        path: Path { segments, .. },
    }) = ty
        && let Some(PathSegment {
            ident,
            arguments: PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }),
        }) = segments.first()
        && let Some(GenericArgument::Type(inner_ty)) = args.first()
    {
        if ident == "Option" || ident == "Vec" {
            return Ok(inner_ty.clone());
        } else {
            return err;
        }
    }

    err
}

/// Parse directive into ListSeparator
fn directive_to_list_separator(
    d: &Directive,
    use_pliron_list_separator: bool,
    span: Span,
) -> Result<TokenStream> {
    let err: Result<TokenStream> = Err(syn::Error::new(
        span,
        "Please refer to the documentation on correctly specifiying a list separator".to_string(),
    ));

    if d.name == "NewLine" {
        if use_pliron_list_separator {
            return Ok(quote! { ::pliron::printable::ListSeparator::NewLine });
        } else {
            return Ok(quote! { '\n' });
        }
    }

    if d.args.len() != 1 {
        return err;
    }

    let Elem::Lit(Lit { lit: sep_char, .. }) = &d.args[0] else {
        return err;
    };

    if sep_char.len() != 1 {
        return err;
    }

    let sep_char = sep_char.chars().next().unwrap();

    if !use_pliron_list_separator {
        return Ok(quote! { #sep_char });
    }

    if d.name == "CharNewline" {
        return Ok(quote! { ::pliron::printable::ListSeparator::CharNewline(#sep_char) });
    } else if d.name == "Char" {
        return Ok(quote! { ::pliron::printable::ListSeparator::Char(#sep_char) });
    } else if d.name == "CharSpace" {
        return Ok(quote! { ::pliron::printable::ListSeparator::CharSpace(#sep_char) });
    }

    err
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::{DeriveIRObject, derive};

    #[test]
    fn format_preserves_generics_in_impl_headers() {
        let input = quote! {
            struct Wrapper<'a, T, const N: usize>
            where
                T: ::pliron::printable::Printable + ::pliron::parsable::Parsable,
            {
                value: T,
                marker: &'a [u64; N],
            }
        };
        let result = derive(
            proc_macro2::TokenStream::new(),
            input,
            DeriveIRObject::AnyOtherRustType,
        )
        .unwrap();
        let file = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&file);

        assert!(got.contains(
            "impl<'a, T, const N: usize> ::pliron::printable::Printable for Wrapper<'a, T, N>"
        ));
        assert!(got.contains(
            "impl<'a, T, const N: usize> ::pliron::parsable::Parsable for Wrapper<'a, T, N>"
        ));
        assert!(got.contains("fn parse<'__pliron_parse>("));
        assert!(got.contains("StateStream<'__pliron_parse>"));
        assert!(got.contains("ParseResult<'__pliron_parse, Self::Parsed>"));
        assert!(got.contains("struct Wrapper<'a, T, const N: usize>"));
    }

    #[test]
    fn format_enum_preserves_generics_in_impl_headers() {
        let input = quote! {
            enum Wrapper<T>
            where
                T: ::pliron::printable::Printable + ::pliron::parsable::Parsable,
            {
                Value(T),
            }
        };
        let result = derive(
            proc_macro2::TokenStream::new(),
            input,
            DeriveIRObject::AnyOtherRustType,
        )
        .unwrap();
        let file = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&file);

        assert!(got.contains("impl<T> ::pliron::printable::Printable for Wrapper<T>"));
        assert!(got.contains("impl<T> ::pliron::parsable::Parsable for Wrapper<T>"));
        assert!(got.contains(
            "where\n    T: ::pliron::printable::Printable + ::pliron::parsable::Parsable,"
        ));
    }
}
