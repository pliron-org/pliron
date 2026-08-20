// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Derive macros for for "decontextualization" traits.
//!
//! `StableHash` and `CloneIntoContext` derives assume that every field's type
//! already implements the corresponding trait, and just delegate to it, field by field.
//! For an enum, the variant's discriminant is mixed into the hash for `StableHash`,
//! and is naturally preserved by reconstructing the same variant for `CloneIntoContext`.
//!
//! `CloneAttributeIntoContext` and `CloneTypeIntoContext` are the [Attribute]/[Type] interface
//! versions of `CloneIntoContext`, implemented by delegating to an existing `CloneIntoContext`
//! impl on the same type.
//!
//! `StableHash`, `CloneAttributeIntoContext` and `CloneTypeIntoContext` register their impls
//!  with `type_to_trait!` (the latter two via interface registrations). So they cannot be
//! derived for generic types. `CloneIntoContext` can be derived for generic types.
//!
//! [Attribute]: ../pliron/attribute/trait.Attribute.html
//! [Type]: ../pliron/type/trait.Type.html

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, parse_quote};

use crate::interfaces;

/// A single field, as seen by the generated code.
///
/// It is either the field's own name, or a synthesized `field_0`, `field_1`, ...
/// for tuple fields.
struct FieldInfo {
    binding: Ident,
}

fn field_infos(fields: &Fields) -> Vec<FieldInfo> {
    match fields {
        Fields::Named(fields) => fields
            .named
            .iter()
            .map(|field| FieldInfo {
                binding: field
                    .ident
                    .clone()
                    .expect("named field must have an identifier"),
            })
            .collect(),
        Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, _field)| FieldInfo {
                binding: format_ident!("field_{}", i),
            })
            .collect(),
        Fields::Unit => Vec::new(),
    }
}

/// A pattern that destructures a value of the given `fields` shape, binding
/// each field to its [FieldInfo::binding]. Empty for [Fields::Unit].
fn destructure_pattern(fields: &Fields, infos: &[FieldInfo]) -> TokenStream {
    let bindings = infos.iter().map(|info| &info.binding);
    match fields {
        Fields::Named(_) => quote! { { #(#bindings),* } },
        Fields::Unnamed(_) => quote! { ( #(#bindings),* ) },
        Fields::Unit => quote! {},
    }
}

/// The shape (named / tuple / unit) that constructs a value from per-field
/// expressions produced by `value_for`. Mirrors [destructure_pattern]'s shape.
fn construct_expr(
    fields: &Fields,
    infos: &[FieldInfo],
    value_for: impl Fn(&FieldInfo) -> TokenStream,
) -> TokenStream {
    match fields {
        Fields::Named(_) => {
            let entries = infos.iter().map(|info| {
                let name = &info.binding;
                let value = value_for(info);
                quote! { #name: #value }
            });
            quote! { { #(#entries),* } }
        }
        Fields::Unnamed(_) => {
            let values = infos.iter().map(value_for);
            quote! { ( #(#values),* ) }
        }
        Fields::Unit => quote! {},
    }
}

/// Reject unions.
fn check_no_union(input: &DeriveInput) -> syn::Result<()> {
    match &input.data {
        Data::Struct(_) | Data::Enum(_) => Ok(()),
        Data::Union(_) => Err(syn::Error::new_spanned(
            input,
            "cannot be derived for unions",
        )),
    }
}

/// Reject generic structs/enums.
fn check_no_generics(input: &DeriveInput) -> syn::Result<()> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "cannot be derived for generic structs or enums",
        ));
    }
    Ok(())
}

pub(crate) fn derive_stable_hash(input: TokenStream) -> syn::Result<TokenStream> {
    let input = syn::parse2::<DeriveInput>(input)?;
    check_no_union(&input)?;
    check_no_generics(&input)?;
    let ident = &input.ident;

    let hash_stmt = |info: &FieldInfo| {
        let name = &info.binding;
        quote! { ::pliron::irbuild::decontext::StableHash::stable_hash(#name, _ctx, _state); }
    };

    // Hash the type's fully-qualified name first, so that two distinct types
    // with the same field/variant shape don't hash identically.
    let type_tag_stmt = quote! {
        let mut _state = _state;
        ::core::hash::Hash::hash(
            ::core::concat!(::core::module_path!(), "::", ::core::stringify!(#ident)),
            &mut _state,
        );
    };

    let body = match &input.data {
        Data::Struct(data) => {
            let infos = field_infos(&data.fields);
            let hash_stmts = infos.iter().map(hash_stmt);
            let pattern = destructure_pattern(&data.fields, &infos);
            quote! {
                #type_tag_stmt
                let #ident #pattern = self;
                #(#hash_stmts)*
            }
        }
        Data::Enum(data) => {
            let arms = data.variants.iter().map(|variant| {
                let variant_ident = &variant.ident;
                let infos = field_infos(&variant.fields);
                let pattern = destructure_pattern(&variant.fields, &infos);
                let hash_stmts = infos.iter().map(hash_stmt);
                quote! { #ident::#variant_ident #pattern => { #(#hash_stmts)* } }
            });
            quote! {
                #type_tag_stmt
                ::core::hash::Hash::hash(&::core::mem::discriminant(self), &mut _state);
                match self {
                    #(#arms)*
                }
            }
        }
        Data::Union(_) => unreachable!("checked by check_no_union"),
    };

    Ok(quote! {
        impl ::pliron::irbuild::decontext::StableHash for #ident {
            fn stable_hash(
                &self,
                _ctx: &::pliron::context::Context,
                _state: &mut dyn ::core::hash::Hasher,
            ) {
                #body
            }
        }
        ::pliron::type_to_trait!(#ident, ::pliron::irbuild::decontext::StableHash);
    })
}

pub(crate) fn derive_clone_into_context(input: TokenStream) -> syn::Result<TokenStream> {
    let input = syn::parse2::<DeriveInput>(input)?;
    check_no_union(&input)?;
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let value_for = |info: &FieldInfo| {
        let name = &info.binding;
        quote! {
            ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                #name, _src_ctx, _dst_ctx,
            )
        }
    };

    let cloned = match &input.data {
        Data::Struct(data) => {
            let infos = field_infos(&data.fields);
            let construct = construct_expr(&data.fields, &infos, value_for);
            let pattern = destructure_pattern(&data.fields, &infos);
            quote! {
                {
                    let #ident #pattern = self;
                    #ident #construct
                }
            }
        }
        Data::Enum(data) => {
            let arms = data.variants.iter().map(|variant| {
                let variant_ident = &variant.ident;
                let infos = field_infos(&variant.fields);
                let pattern = destructure_pattern(&variant.fields, &infos);
                let construct = construct_expr(&variant.fields, &infos, value_for);
                quote! { #ident::#variant_ident #pattern => #ident::#variant_ident #construct, }
            });
            quote! {
                match self {
                    #(#arms)*
                }
            }
        }
        Data::Union(_) => unreachable!("checked by check_no_union"),
    };

    Ok(quote! {
        impl #impl_generics ::pliron::irbuild::decontext::CloneIntoContext
            for #ident #ty_generics #where_clause
        {
            fn clone_into_context(
                &self,
                _src_ctx: &::pliron::context::Context,
                _dst_ctx: &mut ::pliron::context::Context,
            ) -> #ident #ty_generics {
                #cloned
            }
        }
    })
}

/// Implement `CloneAttributeIntoContext` for `$ident` by delegating to its own
/// `CloneIntoContext` impl and boxing the result.
pub(crate) fn derive_clone_attribute_into_context(input: TokenStream) -> syn::Result<TokenStream> {
    let input = syn::parse2::<DeriveInput>(input)?;
    check_no_union(&input)?;
    check_no_generics(&input)?;
    let ident = &input.ident;

    let item_impl: syn::ItemImpl = parse_quote! {
        impl ::pliron::irbuild::decontext::CloneAttributeIntoContext for #ident {
            fn clone_into_context(
                &self,
                src_ctx: &::pliron::context::Context,
                dst_ctx: &mut ::pliron::context::Context,
            ) -> ::pliron::attribute::AttrObj {
                ::pliron::alloc::boxed::Box::new(
                    ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                        self, src_ctx, dst_ctx,
                    ),
                )
            }
        }
    };

    let interface_verifiers_slice = parse_quote! { ::pliron::attribute::ATTR_INTERFACE_VERIFIERS };
    let all_verifiers_fn_type = parse_quote! { ::pliron::attribute::AttrInterfaceAllVerifiers };
    interfaces::interface_impl(
        item_impl.into_token_stream(),
        interface_verifiers_slice,
        all_verifiers_fn_type,
    )
}

/// Implement `CloneTypeIntoContext` for `$ident` by delegating to its own `CloneIntoContext`
/// impl, then re-interning the clone into the destination `Context`.
pub(crate) fn derive_clone_type_into_context(input: TokenStream) -> syn::Result<TokenStream> {
    let input = syn::parse2::<DeriveInput>(input)?;
    check_no_union(&input)?;
    check_no_generics(&input)?;
    let ident = &input.ident;

    let item_impl: syn::ItemImpl = parse_quote! {
        impl ::pliron::irbuild::decontext::CloneTypeIntoContext for #ident {
            fn clone_into_context(
                &self,
                src_ctx: &::pliron::context::Context,
                dst_ctx: &mut ::pliron::context::Context,
            ) -> ::pliron::r#type::TypeHandle {
                let cloned = ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                    self, src_ctx, dst_ctx,
                );
                <#ident as ::pliron::r#type::Type>::instantiate(cloned, dst_ctx).into()
            }
        }
    };

    let interface_verifiers_slice = parse_quote! { ::pliron::r#type::TYPE_INTERFACE_VERIFIERS };
    let all_verifiers_fn_type = parse_quote! { ::pliron::r#type::TypeInterfaceAllVerifiers };
    interfaces::interface_impl(
        item_impl.into_token_stream(),
        interface_verifiers_slice,
        all_verifiers_fn_type,
    )
}

#[cfg(test)]
mod tests {
    use expect_test::expect;
    use quote::quote;

    use super::{
        derive_clone_attribute_into_context, derive_clone_into_context,
        derive_clone_type_into_context, derive_stable_hash,
    };

    fn pretty(result: syn::Result<proc_macro2::TokenStream>) -> String {
        let tokens = result.unwrap();
        let file = syn::parse2::<syn::File>(tokens).unwrap();
        prettyplease::unparse(&file)
    }

    #[test]
    fn stable_hash_named_fields() {
        let input = quote! {
            struct Foo {
                a: u64,
                b: TypeHandle,
            }
        };
        let got = pretty(derive_stable_hash(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::StableHash for Foo {
                fn stable_hash(
                    &self,
                    _ctx: &::pliron::context::Context,
                    _state: &mut dyn ::core::hash::Hasher,
                ) {
                    let mut _state = _state;
                    ::core::hash::Hash::hash(
                        ::core::concat!(::core::module_path!(), "::", ::core::stringify!(Foo)),
                        &mut _state,
                    );
                    let Foo { a, b } = self;
                    ::pliron::irbuild::decontext::StableHash::stable_hash(a, _ctx, _state);
                    ::pliron::irbuild::decontext::StableHash::stable_hash(b, _ctx, _state);
                }
            }
            ::pliron::type_to_trait!(Foo, ::pliron::irbuild::decontext::StableHash);
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn stable_hash_tuple_struct() {
        let input = quote! {
            struct Foo(u64, AttrObj);
        };
        let got = pretty(derive_stable_hash(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::StableHash for Foo {
                fn stable_hash(
                    &self,
                    _ctx: &::pliron::context::Context,
                    _state: &mut dyn ::core::hash::Hasher,
                ) {
                    let mut _state = _state;
                    ::core::hash::Hash::hash(
                        ::core::concat!(::core::module_path!(), "::", ::core::stringify!(Foo)),
                        &mut _state,
                    );
                    let Foo(field_0, field_1) = self;
                    ::pliron::irbuild::decontext::StableHash::stable_hash(field_0, _ctx, _state);
                    ::pliron::irbuild::decontext::StableHash::stable_hash(field_1, _ctx, _state);
                }
            }
            ::pliron::type_to_trait!(Foo, ::pliron::irbuild::decontext::StableHash);
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn stable_hash_unit_struct() {
        let input = quote! {
            struct Foo;
        };
        let got = pretty(derive_stable_hash(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::StableHash for Foo {
                fn stable_hash(
                    &self,
                    _ctx: &::pliron::context::Context,
                    _state: &mut dyn ::core::hash::Hasher,
                ) {
                    let mut _state = _state;
                    ::core::hash::Hash::hash(
                        ::core::concat!(::core::module_path!(), "::", ::core::stringify!(Foo)),
                        &mut _state,
                    );
                    let Foo = self;
                }
            }
            ::pliron::type_to_trait!(Foo, ::pliron::irbuild::decontext::StableHash);
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn stable_hash_enum() {
        let input = quote! {
            enum Foo {
                A,
                B(u64),
                C { x: u64, y: TypeHandle },
            }
        };
        let got = pretty(derive_stable_hash(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::StableHash for Foo {
                fn stable_hash(
                    &self,
                    _ctx: &::pliron::context::Context,
                    _state: &mut dyn ::core::hash::Hasher,
                ) {
                    let mut _state = _state;
                    ::core::hash::Hash::hash(
                        ::core::concat!(::core::module_path!(), "::", ::core::stringify!(Foo)),
                        &mut _state,
                    );
                    ::core::hash::Hash::hash(&::core::mem::discriminant(self), &mut _state);
                    match self {
                        Foo::A => {}
                        Foo::B(field_0) => {
                            ::pliron::irbuild::decontext::StableHash::stable_hash(
                                field_0,
                                _ctx,
                                _state,
                            );
                        }
                        Foo::C { x, y } => {
                            ::pliron::irbuild::decontext::StableHash::stable_hash(x, _ctx, _state);
                            ::pliron::irbuild::decontext::StableHash::stable_hash(y, _ctx, _state);
                        }
                    }
                }
            }
            ::pliron::type_to_trait!(Foo, ::pliron::irbuild::decontext::StableHash);
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn stable_hash_rejects_generics() {
        let input = quote! {
            struct Foo<T> { a: T }
        };
        let err = derive_stable_hash(input).unwrap_err();
        assert!(
            err.to_string()
                .contains("cannot be derived for generic structs or enums")
        );
    }

    #[test]
    fn stable_hash_rejects_unions() {
        let input = quote! {
            union Foo { a: u64 }
        };
        let err = derive_stable_hash(input).unwrap_err();
        assert!(err.to_string().contains("cannot be derived for unions"));
    }

    #[test]
    fn clone_into_context_named_fields() {
        let input = quote! {
            struct Foo {
                a: u64,
                b: TypeHandle,
            }
        };
        let got = pretty(derive_clone_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    _src_ctx: &::pliron::context::Context,
                    _dst_ctx: &mut ::pliron::context::Context,
                ) -> Foo {
                    {
                        let Foo { a, b } = self;
                        Foo {
                            a: ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                                a,
                                _src_ctx,
                                _dst_ctx,
                            ),
                            b: ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                                b,
                                _src_ctx,
                                _dst_ctx,
                            ),
                        }
                    }
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_into_context_unit_struct() {
        let input = quote! {
            struct Foo;
        };
        let got = pretty(derive_clone_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    _src_ctx: &::pliron::context::Context,
                    _dst_ctx: &mut ::pliron::context::Context,
                ) -> Foo {
                    {
                        let Foo = self;
                        Foo
                    }
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_into_context_empty_named_struct() {
        let input = quote! {
            struct Foo {}
        };
        let got = pretty(derive_clone_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    _src_ctx: &::pliron::context::Context,
                    _dst_ctx: &mut ::pliron::context::Context,
                ) -> Foo {
                    {
                        let Foo {} = self;
                        Foo {}
                    }
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_into_context_empty_tuple_struct() {
        let input = quote! {
            struct Foo();
        };
        let got = pretty(derive_clone_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    _src_ctx: &::pliron::context::Context,
                    _dst_ctx: &mut ::pliron::context::Context,
                ) -> Foo {
                    {
                        let Foo() = self;
                        Foo()
                    }
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_into_context_enum() {
        let input = quote! {
            enum Foo {
                A,
                B(u64),
            }
        };
        let got = pretty(derive_clone_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    _src_ctx: &::pliron::context::Context,
                    _dst_ctx: &mut ::pliron::context::Context,
                ) -> Foo {
                    match self {
                        Foo::A => Foo::A,
                        Foo::B(field_0) => {
                            Foo::B(
                                ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                                    field_0,
                                    _src_ctx,
                                    _dst_ctx,
                                ),
                            )
                        }
                    }
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_into_context_generic_struct() {
        let input = quote! {
            struct Foo<T: CloneIntoContext> {
                a: T,
            }
        };
        let got = pretty(derive_clone_into_context(input));
        expect![[r#"
            impl<T: CloneIntoContext> ::pliron::irbuild::decontext::CloneIntoContext for Foo<T> {
                fn clone_into_context(
                    &self,
                    _src_ctx: &::pliron::context::Context,
                    _dst_ctx: &mut ::pliron::context::Context,
                ) -> Foo<T> {
                    {
                        let Foo { a } = self;
                        Foo {
                            a: ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                                a,
                                _src_ctx,
                                _dst_ctx,
                            ),
                        }
                    }
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_attribute_into_context_struct() {
        let input = quote! {
            struct Foo {
                a: u64,
            }
        };
        let got = pretty(derive_clone_attribute_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneAttributeIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    src_ctx: &::pliron::context::Context,
                    dst_ctx: &mut ::pliron::context::Context,
                ) -> ::pliron::attribute::AttrObj {
                    ::pliron::alloc::boxed::Box::new(
                        ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                            self,
                            src_ctx,
                            dst_ctx,
                        ),
                    )
                }
            }
            ::pliron::type_to_trait!(Foo, ::pliron::irbuild::decontext::CloneAttributeIntoContext);
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(
                        ::pliron::attribute::ATTR_INTERFACE_VERIFIERS
                    ),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::attribute::AttrInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<Foo>(),
                    <Foo as ::pliron::irbuild::decontext::CloneAttributeIntoContext>::__all_verifiers,
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_attribute_into_context_rejects_generics() {
        let input = quote! {
            struct Foo<T> { a: T }
        };
        let err = derive_clone_attribute_into_context(input).unwrap_err();
        assert!(
            err.to_string()
                .contains("cannot be derived for generic structs or enums")
        );
    }

    #[test]
    fn clone_type_into_context_struct() {
        let input = quote! {
            struct Foo {
                a: u64,
            }
        };
        let got = pretty(derive_clone_type_into_context(input));
        expect![[r#"
            impl ::pliron::irbuild::decontext::CloneTypeIntoContext for Foo {
                fn clone_into_context(
                    &self,
                    src_ctx: &::pliron::context::Context,
                    dst_ctx: &mut ::pliron::context::Context,
                ) -> ::pliron::r#type::TypeHandle {
                    let cloned = ::pliron::irbuild::decontext::CloneIntoContext::clone_into_context(
                        self,
                        src_ctx,
                        dst_ctx,
                    );
                    <Foo as ::pliron::r#type::Type>::instantiate(cloned, dst_ctx).into()
                }
            }
            ::pliron::type_to_trait!(Foo, ::pliron::irbuild::decontext::CloneTypeIntoContext);
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::r#type::TYPE_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: (
                    ::core::any::TypeId,
                    (::pliron::r#type::TypeInterfaceAllVerifiers),
                ) = (
                    ::core::any::TypeId::of::<Foo>(),
                    <Foo as ::pliron::irbuild::decontext::CloneTypeIntoContext>::__all_verifiers,
                );
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn clone_type_into_context_rejects_generics() {
        let input = quote! {
            struct Foo<T> { a: T }
        };
        let err = derive_clone_type_into_context(input).unwrap_err();
        assert!(
            err.to_string()
                .contains("cannot be derived for generic structs or enums")
        );
    }
}
