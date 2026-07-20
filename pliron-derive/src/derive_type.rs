// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{DeriveInput, LitStr, Result};

const PROC_MACRO_NAME: &str = "def_type";

pub(crate) fn def_type(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let name = syn::parse2::<LitStr>(args.into())?;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DefType::derive(name, input)?;
    Ok(p.into_token_stream())
}

/// Input for the `#[def_type]` proc macro.
struct DefType {
    input: DeriveInput,
    impl_type: ImplType,
}

impl DefType {
    fn derive(name: LitStr, input: DeriveInput) -> Result<Self> {
        let name_str = name.value();
        let Some((dialect_name, type_name)) = name_str.split_once('.') else {
            return Err(syn::Error::new_spanned(
                name,
                "type name must be in the form `dialect.type_name`",
            ));
        };

        let is_singleton = match input.data {
            syn::Data::Struct(ref data_struct) => data_struct.fields.is_empty(),
            syn::Data::Enum(_) => false,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "Type can only be derived for structs or enums",
                ));
            }
        };

        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "Type cannot be derived for generic structs or enums",
            ));
        }

        let attrs = input
            .attrs
            .into_iter()
            .filter(|attr| !attr.path().is_ident(PROC_MACRO_NAME))
            .collect();

        let input = DeriveInput { attrs, ..input };

        let impl_type = ImplType {
            ident: input.ident.clone(),
            dialect_name: dialect_name.to_string(),
            type_name: type_name.to_string(),
            is_singleton,
        };
        Ok(Self { input, impl_type })
    }
}

impl ToTokens for DefType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let def_struct = &self.input;
        let impl_type = &self.impl_type;
        tokens.extend(quote! {
            #def_struct

            #impl_type
        });
    }
}

struct ImplType {
    ident: syn::Ident,
    dialect_name: String,
    type_name: String,
    is_singleton: bool,
}

impl ToTokens for ImplType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.ident;
        let dialect = &self.dialect_name;
        let type_name = &self.type_name;
        let register = if self.is_singleton {
            quote! {
                |ctx: &mut ::pliron::context::Context| {
                    <#name as ::pliron::r#type::Type>::register(ctx);
                    ::pliron::r#type::Type::register_instance(#name {}, ctx);
                }
            }
        } else {
            quote! {
                <#name as ::pliron::r#type::Type>::register
            }
        };

        tokens.extend(quote! {
            impl ::pliron::r#type::Type for #name {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }

                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other
                        .downcast_ref::<Self>()
                        .map_or(false, |other| other == self)
                }

                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }

                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::r#type::TypeName::try_new(#type_name).expect("Invalid Identifier for TypeName"),
                        dialect: ::pliron::dialect::DialectName::try_new(#dialect).expect("Invalid Identifier for DialectName"),
                    }
                }

                fn verify_interfaces(&self, ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) =
                        ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP.get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }

            ::pliron::context_registration!(#register);
        });
    }
}

pub(crate) fn derive_type_get(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args_stream = args.into();
    if !args_stream.is_empty() {
        return Err(syn::Error::new_spanned(
            args_stream,
            "`#[derive_type_get]` does not take any arguments",
        ));
    }

    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DeriveTypeGet::derive(input)?;
    Ok(p.into_token_stream())
}

/// Input for the `#[derive(DeriveTypeGet)]` proc macro.
struct DeriveTypeGet {
    input: DeriveInput,
    ident: syn::Ident,
    fields: syn::Fields,
}

impl DeriveTypeGet {
    fn derive(input: DeriveInput) -> Result<Self> {
        let fields = match input.data {
            syn::Data::Struct(ref data_struct) => data_struct.fields.clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "DeriveTypeGet can only be derived for structs",
                ));
            }
        };

        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "DeriveTypeGet cannot be derived for generic structs",
            ));
        }

        Ok(Self {
            ident: input.ident.clone(),
            input,
            fields,
        })
    }
}

impl ToTokens for DeriveTypeGet {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let input = &self.input;
        let impl_get = derive_type_get_impl(&self.ident, &self.fields);
        tokens.extend(quote! {
            #input

            #impl_get
        });
    }
}

/// Generate get methods for types based on their fields
fn derive_type_get_impl(ident: &syn::Ident, fields: &syn::Fields) -> TokenStream {
    let field_params = generate_field_params(fields);
    let struct_construction = generate_struct_construction(ident, fields);

    quote! {
        impl #ident {
            /// Get or create a new instance.
            pub fn get(
                ctx: &::pliron::context::Context,
                #field_params
            ) -> ::pliron::r#type::TypedHandle<Self> {
                ::pliron::r#type::Type::register_instance(
                    #struct_construction,
                    ctx,
                )
            }
        }
    }
}

/// Generate field parameters for function signatures
fn generate_field_params(fields: &syn::Fields) -> TokenStream {
    match fields {
        syn::Fields::Named(fields) => {
            let params = fields.named.iter().map(|field| {
                let name = &field.ident;
                let ty = &field.ty;
                quote! { #name: #ty }
            });
            quote! { #(#params),* }
        }
        syn::Fields::Unnamed(fields) => {
            let params = fields.unnamed.iter().enumerate().map(|(i, field)| {
                let name = syn::Ident::new(&format!("field_{}", i), proc_macro2::Span::call_site());
                let ty = &field.ty;
                quote! { #name: #ty }
            });
            quote! { #(#params),* }
        }
        syn::Fields::Unit => quote! {},
    }
}

/// Generate field assignments for struct construction
fn generate_field_assignments(fields: &syn::Fields) -> TokenStream {
    match fields {
        syn::Fields::Named(fields) => {
            let assignments = fields.named.iter().map(|field| {
                let name = &field.ident;
                quote! { #name }
            });
            quote! { #(#assignments),* }
        }
        syn::Fields::Unnamed(fields) => {
            let assignments = fields.unnamed.iter().enumerate().map(|(i, _)| {
                let name = syn::Ident::new(&format!("field_{}", i), proc_macro2::Span::call_site());
                quote! { #name }
            });
            quote! { #(#assignments),* }
        }
        syn::Fields::Unit => quote! {},
    }
}

/// Generate struct construction syntax based on field type
fn generate_struct_construction(ident: &syn::Ident, fields: &syn::Fields) -> TokenStream {
    match fields {
        syn::Fields::Named(_) => {
            let field_assignments = generate_field_assignments(fields);
            quote! { #ident { #field_assignments } }
        }
        syn::Fields::Unnamed(_) => {
            let field_assignments = generate_field_assignments(fields);
            quote! { #ident(#field_assignments) }
        }
        syn::Fields::Unit => {
            quote! { #ident {} }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn simple() {
        let args = quote! { "testing.simple_type" };
        let input = quote! {
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct SimpleType;
        };
        let t = def_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct SimpleType;
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
                        name: ::pliron::r#type::TypeName::try_new("simple_type")
                            .expect("Invalid Identifier for TypeName"),
                        dialect: ::pliron::dialect::DialectName::try_new("testing")
                            .expect("Invalid Identifier for DialectName"),
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
            ::pliron::context_registration!(
                | ctx : & mut ::pliron::context::Context | { < SimpleType as ::pliron::r#type::Type >
                ::register(ctx); ::pliron::r#type::Type::register_instance(SimpleType {}, ctx); }
            );
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn compound() {
        let args = quote! { "testing.compound_type" };
        let input = quote! {
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct CompoundType {
                x1: u32,
                x2: String,
            }
        };
        let t = def_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct CompoundType {
                x1: u32,
                x2: String,
            }
            impl ::pliron::r#type::Type for CompoundType {
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
                        name: ::pliron::r#type::TypeName::try_new("compound_type")
                            .expect("Invalid Identifier for TypeName"),
                        dialect: ::pliron::dialect::DialectName::try_new("testing")
                            .expect("Invalid Identifier for DialectName"),
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
            ::pliron::context_registration!(< CompoundType as ::pliron::r#type::Type > ::register);
        "##]].assert_eq(&got);
    }

    #[test]
    fn enum_type() {
        let args = quote! { "testing.enum_type" };
        let input = quote! {
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub enum EnumType {
                None,
                One(u32),
            }
        };
        let t = def_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub enum EnumType {
                None,
                One(u32),
            }
            impl ::pliron::r#type::Type for EnumType {
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
                        name: ::pliron::r#type::TypeName::try_new("enum_type")
                            .expect("Invalid Identifier for TypeName"),
                        dialect: ::pliron::dialect::DialectName::try_new("testing")
                            .expect("Invalid Identifier for DialectName"),
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
            ::pliron::context_registration!(< EnumType as ::pliron::r#type::Type > ::register);
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn derive_type_get_unit_struct() {
        let args = quote! {};
        let input = quote! {
            pub struct UnitType;
        };
        let t = derive_type_get(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            pub struct UnitType;
            impl UnitType {
                /// Get or create a new instance.
                pub fn get(ctx: &::pliron::context::Context) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::register_instance(UnitType {}, ctx)
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn derive_type_get_named_fields() {
        let args = quote! {};
        let input = quote! {
            pub struct VectorType {
                elem_ty: TypeHandle,
                num_elems: u32,
                kind: VectorTypeKind,
            }
        };
        let t = derive_type_get(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            pub struct VectorType {
                elem_ty: TypeHandle,
                num_elems: u32,
                kind: VectorTypeKind,
            }
            impl VectorType {
                /// Get or create a new instance.
                pub fn get(
                    ctx: &::pliron::context::Context,
                    elem_ty: TypeHandle,
                    num_elems: u32,
                    kind: VectorTypeKind,
                ) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::register_instance(
                        VectorType {
                            elem_ty,
                            num_elems,
                            kind,
                        },
                        ctx,
                    )
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn derive_type_get_unnamed_fields() {
        let args = quote! {};
        let input = quote! {
            pub struct TupleType(u32, String, bool);
        };
        let t = derive_type_get(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            pub struct TupleType(u32, String, bool);
            impl TupleType {
                /// Get or create a new instance.
                pub fn get(
                    ctx: &::pliron::context::Context,
                    field_0: u32,
                    field_1: String,
                    field_2: bool,
                ) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::register_instance(
                        TupleType(field_0, field_1, field_2),
                        ctx,
                    )
                }
            }
        "#]]
        .assert_eq(&got);
    }
}
