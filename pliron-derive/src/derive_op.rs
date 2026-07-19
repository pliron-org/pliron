// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{DeriveInput, LitStr, Result, parse::Parser, parse_quote};

const PROC_MACRO_NAME: &str = "def_op";

struct OperandOrResultSpec {
    name: Option<syn::Ident>,
    ty: Option<Box<syn::Type>>,
}

impl ToTokens for OperandOrResultSpec {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match &self.name {
            Some(name) => name.to_tokens(tokens),
            None => tokens.extend(quote! { _ }),
        }
        if let Some(ty) = &self.ty {
            tokens.extend(quote! { : #ty });
        }
    }
}

impl syn::parse::Parse for OperandOrResultSpec {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = if input.peek(syn::Token![_]) {
            input.parse::<syn::Token![_]>()?;
            None
        } else {
            Some(input.parse()?)
        };
        let ty = if input.peek(syn::Token![:]) {
            input.parse::<syn::Token![:]>()?;
            Some(Box::new(input.parse()?))
        } else {
            None
        };
        Ok(Self { name, ty })
    }
}

pub(crate) fn def_op(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> Result<TokenStream> {
    let name = syn::parse2::<LitStr>(args.into())?;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DefOp::derive(name, input)?;
    Ok(p.into_token_stream())
}

/// The derived macro body for the `#[def_op]` proc macro.
struct DefOp {
    input: DeriveInput,
    impl_op: ImplOp,
}

impl DefOp {
    fn derive(name: LitStr, input: DeriveInput) -> Result<Self> {
        let name_str = name.value();
        let Some((dialect_name, op_name)) = name_str.split_once('.') else {
            return Err(syn::Error::new_spanned(
                name,
                "op_name must be in the form `dialect.op_name`",
            ));
        };

        let syn::Data::Struct(ref struct_data) = input.data else {
            return Err(syn::Error::new_spanned(
                &input,
                "Type can only be derived for structs",
            ));
        };
        if !struct_data.fields.is_empty() {
            return Err(syn::Error::new_spanned(
                &struct_data.fields,
                "Op struct cannot have custom fields",
            ));
        }
        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "Op cannot be derived for generic structs",
            ));
        }

        let attrs = input
            .attrs
            .into_iter()
            .filter(|attr| !attr.path().is_ident(PROC_MACRO_NAME))
            .collect();

        let input = DeriveInput { attrs, ..input };

        let impl_op = ImplOp {
            struct_name: input.ident.clone(),
            dialect_name: dialect_name.to_string(),
            op_name: op_name.to_string(),
        };
        Ok(Self { input, impl_op })
    }
}

impl ToTokens for DefOp {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let def_struct = {
            let vis = &self.input.vis;
            let attributes = &self.input.attrs;
            let ident = &self.input.ident;
            let generics = &self.input.generics;
            quote! {
                #[derive(Clone, Copy, PartialEq, Eq, Hash)]
                #(#attributes)*
                #vis struct #ident #generics { op: ::pliron::context::Ptr<::pliron::operation::Operation> }
            }
        };

        let impl_op_trait = &self.impl_op;

        tokens.extend(quote! {
            #def_struct

            #impl_op_trait
        });
    }
}

struct ImplOp {
    struct_name: syn::Ident,
    dialect_name: String,
    op_name: String,
}

impl ToTokens for ImplOp {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.struct_name;
        let dialect = &self.dialect_name;
        let op_name = &self.op_name;
        tokens.extend(quote! {
            impl ::pliron::op::Op for #name {
                fn get_operation(&self) -> ::pliron::context::Ptr<::pliron::operation::Operation> {
                    self.op
                }

                fn wrap_operation(op: ::pliron::context::Ptr<::pliron::operation::Operation>) -> ::pliron::op::OpObj {
                    ::pliron::op::OpObj::new(Self::from_operation(op))
                }

                fn from_operation(op: ::pliron::context::Ptr<::pliron::operation::Operation>) -> Self {
                    #name { op }
                }

                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }

                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::op::OpName::new(#op_name),
                        dialect: ::pliron::dialect::DialectName::new(#dialect),
                    }
                }

                fn verify_interfaces(&self, ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) =
                        ::pliron::op::OP_INTERFACE_VERIFIERS_MAP.get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }

            ::pliron::context_registration!(<#name as ::pliron::op::Op>::register);
        });
    }
}

pub(crate) fn derive_attr_get_set(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> Result<TokenStream> {
    enum Attr {
        Name(syn::Ident),
        // The boxing is to reduce the total size of the enum,
        // as advised by Clippy.
        NameWithType(syn::Ident, Box<syn::Type>),
    }

    impl ToTokens for Attr {
        fn to_tokens(&self, tokens: &mut TokenStream) {
            match self {
                Attr::Name(ident) => ident.to_tokens(tokens),
                Attr::NameWithType(ident, ty) => {
                    ident.to_tokens(tokens);
                    tokens.extend(quote! { : #ty });
                }
            }
        }
    }

    impl syn::parse::Parse for Attr {
        fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
            let attr_name: syn::Ident = input.parse()?;
            if input.peek(syn::Token![:]) {
                input.parse::<syn::Token![:]>()?;
                let ty: syn::Type = input.parse()?;
                Ok(Attr::NameWithType(attr_name, Box::new(ty)))
            } else {
                Ok(Attr::Name(attr_name))
            }
        }
    }

    let attrs = syn::punctuated::Punctuated::<Attr, syn::Token![,]>::parse_terminated
        .parse2(args.into())?;

    let mut input = syn::parse2::<DeriveInput>(input.into())?;
    let op_name = input.ident.clone();
    let module_name = format_ident!("{}_attr_names", op_name.to_string().to_case(Case::Snake));

    let mut output = TokenStream::new();

    let attr_comment_intro = [
        "",
        "### Attribute(s):  ",
        "Note: Only attributes defined directly as part of this operation are listed here.",
        "There may be others, not listed here, defined by interface implementations.",
        "",
        "| Name | Static Name Identifier | Type |",
        "| ---- | ---------------------- | ---- |",
    ]
    .iter()
    .map(|s| {
        let attr_comment: syn::Attribute = parse_quote! {
            #[doc = #s]
        };
        attr_comment
    })
    .collect::<Vec<_>>();

    let mut attr_name_const_decls = Vec::new();
    let mut attr_get_set_fns = Vec::new();
    let mut attr_comment_lines = attr_comment_intro;
    for attr in attrs.iter() {
        let (attr_name, ty_opt) = match attr {
            Attr::Name(attr_name) => (attr_name, None),
            Attr::NameWithType(attr_name, ty) => (attr_name, Some(ty)),
        };

        // Build the const declaration for the attribute name.
        let attr_name_string = attr_name.to_string();
        let attr_name_uppercase = attr_name_string.to_uppercase();
        let attr_name_const = format_ident!("ATTR_KEY_{}", attr_name_uppercase);
        let attr_name_const_decl = quote! {
            ::pliron::dict_key!(#attr_name_const, #attr_name_string);
        };
        attr_name_const_decls.push(attr_name_const_decl);

        // Build the getter and setter methods for the attribute.
        let fn_name_get = format_ident!("get_attr_{}", attr_name);
        let fn_name_set = format_ident!("set_attr_{}", attr_name);
        let fn_comment_get = format!(
            "Get a [Ref](core::cell::Ref) to the value of the attribute named `{attr_name}`."
        );
        let fn_comment_set = format!("Set the value of the attribute named `{attr_name}`.");
        let get_set_fns = if let Some(ty) = ty_opt {
            quote! {
                #[doc = #fn_comment_get]
                /// The `Ref` is a borrow of the containing `Operation` object.
                pub fn #fn_name_get<'a>(&self, ctx: &'a ::pliron::context::Context)
                    -> Option<::core::cell::Ref<'a, #ty>>
                {
                    ::core::cell::Ref::filter_map(self.op.deref(ctx), |op|
                        op.attributes.get::<#ty>(&*#module_name::#attr_name_const)).ok()
                }

                #[doc = #fn_comment_set]
                pub fn #fn_name_set(&self, ctx: &::pliron::context::Context, value: #ty) {
                    self.op.deref_mut(ctx).attributes.set(#module_name::#attr_name_const.clone(), value);
                }
            }
        } else {
            quote! {
                #[doc = #fn_comment_get]
                /// The `Ref` is a borrow of the containing `Operation` object.
                pub fn #fn_name_get<'a>(&self, ctx: &'a ::pliron::context::Context)
                    -> Option<::core::cell::Ref<'a, ::pliron::attribute::AttrObj>>
                {
                    ::core::cell::Ref::filter_map(self.op.deref(ctx), |op|
                        op.attributes.0.get(&*#module_name::#attr_name_const)).ok()
                }

                #[doc = #fn_comment_set]
                pub fn #fn_name_set
                    (&self, ctx: &::pliron::context::Context,
                    value: ::pliron::attribute::AttrObj)
                {
                    self.op.deref_mut(ctx).attributes
                        .0.insert(#module_name::#attr_name_const.clone(), value);
                }
            }
        };
        attr_get_set_fns.push(get_set_fns);

        // Build a line in the markdown table for the attribute.
        let attr_comment = format!(
            "| `{}` | [{}]({}) | {} |",
            attr_name,
            attr_name_const,
            // For some reason, the printer inserts spaces between "::"
            // which screws up the link generation in the markdown.
            quote! { #module_name::#attr_name_const }
                .to_token_stream()
                .to_string()
                .replace(" ", ""),
            if let Some(ty) = ty_opt {
                format!("[{}]", ty.to_token_stream())
            } else {
                "Any".to_string()
            }
        );
        let attr_comment: syn::Attribute = parse_quote! {
            #[doc = #attr_comment]
        };
        attr_comment_lines.push(attr_comment);
    }

    // Generate the module with the attribute name constants.
    // This is so that we don't have to build an Identifier from the attribute name
    // every time we want to use it.
    let attr_name_const_decls = quote! {
        #[doc(hidden)]
        pub mod #module_name {
            use ::pliron::std_deps::sync::LazyLock;
            use ::pliron::identifier::Identifier;

            #(#attr_name_const_decls)
            *
        }
    };
    output.extend(attr_name_const_decls);

    // Generate the impl block for the attribute getter and setter methods.
    let impl_block = quote! {
        impl #op_name {
            #(#attr_get_set_fns)*
        }
    };
    output.extend(impl_block);

    // Append the markdown table for the attributes.
    input.attrs.extend(attr_comment_lines);

    // Since this is an attribute macro, we need to include the original input too.
    output.extend(input.to_token_stream());

    Ok(output)
}

pub(crate) fn operands(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> Result<TokenStream> {
    let operands =
        syn::punctuated::Punctuated::<OperandOrResultSpec, syn::Token![,]>::parse_terminated
            .parse2(args.into())?;

    let mut input = syn::parse2::<DeriveInput>(input.into())?;
    let op_name = input.ident.clone();

    let mut getter_fns = Vec::new();
    let mut typed_interfaces = Vec::new();
    for (index, operand) in operands.iter().enumerate() {
        if let Some(name) = &operand.name {
            let fn_name_get = format_ident!("get_operand_{}", name);
            getter_fns.push(quote! {
                pub fn #fn_name_get(&self, ctx: &::pliron::context::Context) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_operand(#index)
                }
            });
        }

        if let Some(ty) = &operand.ty {
            typed_interfaces.push(quote! {
                ::pliron::builtin::op_interfaces::OperandNOfType<#index, #ty>
            });
        }
    }

    if !typed_interfaces.is_empty() {
        let typed_interface_list = quote! { #(#typed_interfaces),* };
        input.attrs.push(parse_quote! {
            #[::pliron::derive::derive_op_interface_impl(#typed_interface_list)]
        });
    }

    let mut output = TokenStream::new();
    if !getter_fns.is_empty() {
        output.extend(quote! {
            impl #op_name {
                #(#getter_fns)*
            }
        });
    }

    // Since this is an attribute macro, we need to include the original input too.
    output.extend(input.to_token_stream());

    Ok(output)
}

pub(crate) fn results(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> Result<TokenStream> {
    let results =
        syn::punctuated::Punctuated::<OperandOrResultSpec, syn::Token![,]>::parse_terminated
            .parse2(args.into())?;

    let mut input = syn::parse2::<DeriveInput>(input.into())?;
    let op_name = input.ident.clone();

    let mut getter_fns = Vec::new();
    let mut typed_interfaces = Vec::new();
    for (index, result) in results.iter().enumerate() {
        if let Some(name) = &result.name {
            let fn_name_get = format_ident!("get_result_{}", name);
            getter_fns.push(quote! {
                pub fn #fn_name_get(&self, ctx: &::pliron::context::Context) -> ::pliron::value::Value {
                    self.op.deref(ctx).get_result(#index)
                }
            });
        }

        if let Some(ty) = &result.ty {
            typed_interfaces.push(quote! {
                ::pliron::builtin::op_interfaces::ResultNOfType<#index, #ty>
            });
        }
    }

    let typed_interface_list = quote! { #(#typed_interfaces),* };
    if !typed_interface_list.is_empty() {
        input.attrs.push(parse_quote! {
            #[::pliron::derive::derive_op_interface_impl(#typed_interface_list)]
        });
    }

    let mut output = TokenStream::new();
    if !getter_fns.is_empty() {
        output.extend(quote! {
            impl #op_name {
                #(#getter_fns)*
            }
        });
    }
    output.extend(input.to_token_stream());

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn simple() {
        let arg = quote! { "testing.testop" };
        let input = quote! {
            struct TestOp;
        };
        let op = def_op(arg, input).unwrap();
        let f = syn::parse2::<syn::File>(op).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct TestOp {
                op: ::pliron::context::Ptr<::pliron::operation::Operation>,
            }
            impl ::pliron::op::Op for TestOp {
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
                    TestOp { op }
                }
                fn get_opid(&self) -> ::pliron::op::OpId {
                    Self::get_opid_static()
                }
                fn get_opid_static() -> ::pliron::op::OpId {
                    ::pliron::op::OpId {
                        name: ::pliron::op::OpName::new("testop"),
                        dialect: ::pliron::dialect::DialectName::new("testing"),
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
            ::pliron::context_registration!(< TestOp as ::pliron::op::Op > ::register);
        "##]]
        .assert_eq(&got);
    }
}
