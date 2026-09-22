// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Attribute macros for defining and implementing Interfaces.

use quote::{ToTokens, quote};
use syn::{
    DeriveInput, ItemImpl, ItemTrait, Path, Result, Token, Type, TypeParamBound, parse::Parse,
    parse_quote, punctuated::Punctuated,
};

/// This macro does two things:
/// 1. Prepends `supertrait` as a super trait.
/// 2. Adds an entry in `interface_deps_slice` for each
///    super interface, so that verifiers of those can be run prior to this.
pub(crate) fn interface_define(
    input: proc_macro::TokenStream,
    supertrait: Path,
    verifier_type: Path,
    append_dyn_clone_trait: bool,
    target_marker_trait: Path,
) -> Result<proc_macro2::TokenStream> {
    let mut r#trait = syn::parse2::<ItemTrait>(input.into())?;

    if let Some(lifetime) = r#trait.generics.lifetimes().next() {
        return Err(syn::Error::new_spanned(
            lifetime,
            "An interface cannot have a lifetime parameter",
        ));
    }

    let intr_name = r#trait.ident.clone();
    let generics = r#trait.generics.clone();
    // https://github.com/kardeiz/objekt-clonable/blob/master/dyn-clonable-impl/src/lib.rs
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let dep_interfaces: Vec<_> = r#trait
        .supertraits
        .iter()
        .filter_map(|dep| {
            let TypeParamBound::Trait(trait_bound) = dep else {
                return None;
            };
            Some(trait_bound.path.clone())
        })
        .collect();
    let supertraits = r#trait.supertraits;

    // Create a method for getting super verifiers + self verifier
    let all_verifiers = quote! {
        #[doc(hidden)]
        fn __all_verifiers() -> ::pliron::alloc::vec::Vec<#verifier_type> where Self: Sized {
            let mut all_verifiers: ::pliron::alloc::vec::Vec<#verifier_type> = ::pliron::alloc::vec::Vec::new();
            #(
                all_verifiers.append(&mut <Self as #dep_interfaces>::__all_verifiers());
            )*
            all_verifiers.push(<Self as #intr_name #ty_generics >::verify as #verifier_type);
            all_verifiers
        }
    };

    for item in &mut r#trait.items {
        if let syn::TraitItem::Fn(meth) = item
            && meth.sig.ident == "verify"
            && meth.default.is_some()
        {
            // Found the verifier method, add a #[inline(never)] to prevent inlining.
            // This helps reduce multiple executions of the same verifier when
            // called from different sub-interfaces.
            meth.attrs.push(parse_quote! { #[inline(never)] });
            // Add a #[doc(hidden)] to hide it from documentation,
            // since users should call `verify_op`, `verify_attr`, or `verify_type` instead.
            meth.attrs.push(parse_quote! { #[doc(hidden)] });
        }
    }

    r#trait
        .items
        .push(syn::parse2::<syn::TraitItem>(all_verifiers)?);

    // Append main super trait (Op/Attribute/Type).
    r#trait.supertraits = parse_quote! { #supertrait + #supertraits };

    let mut output = r#trait.into_token_stream();
    if append_dyn_clone_trait {
        output.extend(quote! {
            ::pliron::dyn_clone::clone_trait_object!(#impl_generics #intr_name #ty_generics #where_clause);
        });
    }

    output.extend(quote! {
        impl #impl_generics #target_marker_trait for dyn #intr_name #ty_generics #where_clause {}
    });

    Ok(output)
}

/// Implement common traits for `Box<dyn Interface>`.
///
/// These all delegate to the `Attribute` that the interface object holds.
pub(crate) fn attr_interface_obj_traits(
    input: proc_macro::TokenStream,
) -> Result<proc_macro2::TokenStream> {
    let r#trait = syn::parse2::<ItemTrait>(input.into())?;
    let intr_name = r#trait.ident.clone();
    let generics = r#trait.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Only 'static lifetimes work, and that's checked for by `interface_define`.
    let mut obj_where_clause = where_clause
        .cloned()
        .unwrap_or_else(|| parse_quote! { where });
    for ty_param in generics.type_params() {
        let ty_param = &ty_param.ident;
        obj_where_clause
            .predicates
            .push(parse_quote! { #ty_param: 'static });
    }

    // Equality and hashing are on the interface object itself.
    // `Box<dyn Interface>` gets them from the standard library's impls for `Box`.
    Ok(quote! {
        impl #impl_generics ::core::cmp::PartialEq for dyn #intr_name #ty_generics #obj_where_clause {
            fn eq(&self, other: &Self) -> bool {
                ::pliron::attribute::Attribute::eq_attr(self, other)
            }
        }

        impl #impl_generics ::core::cmp::Eq for dyn #intr_name #ty_generics #obj_where_clause {}

        impl #impl_generics ::core::hash::Hash for dyn #intr_name #ty_generics #obj_where_clause {
            fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) {
                ::core::hash::Hasher::write_u64(
                    state,
                    ::pliron::attribute::Attribute::hash_attr(self).into(),
                );
            }
        }

        // Printable: Call the same formatting function that `AttrObj` does.
        impl #impl_generics ::pliron::printable::Printable
            for ::pliron::alloc::boxed::Box<dyn #intr_name #ty_generics> #obj_where_clause
        {
            fn fmt(
                &self,
                ctx: &::pliron::context::Context,
                state: &::pliron::printable::State,
                f: &mut ::core::fmt::Formatter<'_>,
            ) -> ::core::fmt::Result {
                ::pliron::attribute::fmt_attr_obj(&**self, ctx, state, f)
            }
        }

        // Parsable: The box parses any attribute, and rejects one that isn't of this interface.
        impl #impl_generics ::pliron::parsable::Parsable
            for ::pliron::alloc::boxed::Box<dyn #intr_name #ty_generics> #obj_where_clause
        {
            type Arg = ();
            type Parsed = Self;

            fn parse<'a>(
                state_stream: &mut ::pliron::parsable::StateStream<'a>,
                _arg: Self::Arg,
            ) -> ::pliron::parsable::ParseResult<'a, Self::Parsed> {
                ::pliron::attribute::parse_attr_interface_obj(state_stream)
            }
        }
    })
}

/// Whether an interface impl must also be registered for casting boxed objects
/// (i.e., with `boxed_type_to_trait!`, in addition to `type_to_trait!`).
enum RegisterBoxedCast {
    Register,
    Skip,
}

/// Records statically that the rust type implements the interface
enum ImplsMarkerTrait {
    /// Implement this marker trait for the rust type.
    Implement(Path),
    /// This kind of interface has no marker trait.
    Skip,
}

/// Registration options for an interface implementation.
pub(crate) struct InterfaceImplOpts {
    /// Distributed slice that stores the interface verifiers.
    interface_verifiers_slice: Path,
    /// Type of the `__all_verifiers` function.
    all_verifiers_fn_type: Path,
    /// Whether to register casts for boxed values.
    register_boxed_cast: RegisterBoxedCast,
    /// Controls marker-trait implementations for registered types.
    impls_marker_trait: ImplsMarkerTrait,
}

impl InterfaceImplOpts {
    pub(crate) fn op() -> Self {
        Self {
            interface_verifiers_slice: parse_quote! { ::pliron::op::OP_INTERFACE_VERIFIERS },
            all_verifiers_fn_type: parse_quote! { ::pliron::op::OpInterfaceAllVerifiers },
            register_boxed_cast: RegisterBoxedCast::Skip,
            impls_marker_trait: ImplsMarkerTrait::Skip,
        }
    }

    pub(crate) fn attr() -> Self {
        Self {
            interface_verifiers_slice: parse_quote! {
                ::pliron::attribute::ATTR_INTERFACE_VERIFIERS
            },
            all_verifiers_fn_type: parse_quote! {
                ::pliron::attribute::AttrInterfaceAllVerifiers
            },
            register_boxed_cast: RegisterBoxedCast::Register,
            impls_marker_trait: ImplsMarkerTrait::Skip,
        }
    }

    pub(crate) fn r#type() -> Self {
        Self {
            interface_verifiers_slice: parse_quote! {
                ::pliron::r#type::TYPE_INTERFACE_VERIFIERS
            },
            all_verifiers_fn_type: parse_quote! { ::pliron::r#type::TypeInterfaceAllVerifiers },
            register_boxed_cast: RegisterBoxedCast::Skip,
            impls_marker_trait: ImplsMarkerTrait::Implement(
                parse_quote! { ::pliron::r#type::TypeImplsInterface },
            ),
        }
    }
}

/// Generate registrations for a list of type-interface pairs.
fn interface_registrations(
    pairs: &[(Type, Path)],
    opts: &InterfaceImplOpts,
) -> proc_macro2::TokenStream {
    let InterfaceImplOpts {
        interface_verifiers_slice,
        all_verifiers_fn_type,
        register_boxed_cast,
        impls_marker_trait,
    } = opts;
    let rust_tys = pairs.iter().map(|(rust_ty, _)| rust_ty);
    let intr_names = pairs.iter().map(|(_, intr_name)| intr_name);
    let mut output = quote! {
        ::pliron::type_to_trait!(#((#rust_tys, #intr_names)),*);
    };

    if matches!(register_boxed_cast, RegisterBoxedCast::Register) {
        for (rust_ty, intr_name) in pairs {
            output.extend(quote! {
                ::pliron::boxed_type_to_trait!(#rust_ty, #intr_name);
            });
        }
    }

    let verifiers = pairs.iter().map(|(rust_ty, intr_name)| {
        quote! {
            (::core::any::TypeId::of::<#rust_ty>(), <#rust_ty as #intr_name>::__all_verifiers)
        }
    });
    output.extend(quote! {
        const _: () = {
            #[cfg_attr(not(target_family = "wasm"), ::pliron::linkme::distributed_slice(#interface_verifiers_slice), linkme(crate = ::pliron::linkme))]
            static INTERFACE_VERIFIER: &[(::core::any::TypeId, (#all_verifiers_fn_type))] =
                    &[#(#verifiers),*];
            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! {
                ::pliron::InventoryWrapper(&INTERFACE_VERIFIER)
            }
        };
    });

    if let ImplsMarkerTrait::Implement(impls_marker_trait) = impls_marker_trait {
        for (rust_ty, intr_name) in pairs {
            output.extend(quote! {
                impl #impls_marker_trait<dyn #intr_name> for #rust_ty {}
            });
        }
    }

    output
}

/// Register an interface implementation.
pub(crate) fn interface_impl(
    input: proc_macro2::TokenStream,
    opts: InterfaceImplOpts,
) -> Result<proc_macro2::TokenStream> {
    let r#impl = syn::parse2::<ItemImpl>(input)?;

    let Some((intr_name, _)) = r#impl.trait_.clone() else {
        return Err(syn::Error::new_spanned(
            r#impl,
            "#[*_interface_impl] can be specified only on a trait impl",
        ));
    };

    if !r#impl.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            r#impl,
            "#[*_interface_impl] cannot be specified on a trait impl with generic parameters",
        ));
    }

    let pairs = [((*r#impl.self_ty).clone(), intr_name)];
    let mut output = r#impl.to_token_stream();
    output.extend(interface_registrations(&pairs, &opts));

    Ok(output)
}

struct PathList {
    paths: Vec<Path>,
}

impl Parse for PathList {
    fn parse(input: syn::parse::ParseStream) -> Result<Self> {
        let paths = Punctuated::<Path, Token![,]>::parse_terminated(input)?;
        Ok(PathList {
            paths: paths.into_iter().collect(),
        })
    }
}

/// Implement each interface listed in `#[derive_op_interface_impl(...)]` for an op.
pub(crate) fn derive_op_interface_impl(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> Result<proc_macro2::TokenStream> {
    let interfaces = syn::parse2::<PathList>(attr.into())?.paths;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let struct_name = input.ident.clone();
    let struct_ty: Type = parse_quote! { #struct_name };

    let mut output = input.to_token_stream();
    output.extend(quote! {
        #(impl #interfaces for #struct_name {})*
    });

    let pairs: Vec<_> = interfaces
        .into_iter()
        .map(|intr_name| (struct_ty.clone(), intr_name))
        .collect();
    output.extend(interface_registrations(&pairs, &InterfaceImplOpts::op()));

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn op_interface() {
        let pairs = [
            (parse_quote! { MyOp }, parse_quote! { OneOpdInterface }),
            (parse_quote! { MyOp }, parse_quote! { NRegionsInterface<1> }),
        ];
        let result = interface_registrations(&pairs, &InterfaceImplOpts::op());
        let f = syn::parse2::<syn::File>(result).unwrap();
        expect![[r#"
            ::pliron::type_to_trait!((MyOp, OneOpdInterface), (MyOp, NRegionsInterface < 1 >));
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::op::OP_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: &[(
                    ::core::any::TypeId,
                    (::pliron::op::OpInterfaceAllVerifiers),
                )] = &[
                    (::core::any::TypeId::of::<MyOp>(), <MyOp as OneOpdInterface>::__all_verifiers),
                    (
                        ::core::any::TypeId::of::<MyOp>(),
                        <MyOp as NRegionsInterface<1>>::__all_verifiers,
                    ),
                ];
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
        "#]]
        .assert_eq(&prettyplease::unparse(&f));
    }

    #[test]
    fn attr_interface() {
        let pairs = [(parse_quote! { MyAttr }, parse_quote! { MyAttrInterface })];
        let result = interface_registrations(&pairs, &InterfaceImplOpts::attr());
        let f = syn::parse2::<syn::File>(result).unwrap();
        expect![[r#"
            ::pliron::type_to_trait!((MyAttr, MyAttrInterface));
            ::pliron::boxed_type_to_trait!(MyAttr, MyAttrInterface);
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(
                        ::pliron::attribute::ATTR_INTERFACE_VERIFIERS
                    ),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: &[(
                    ::core::any::TypeId,
                    (::pliron::attribute::AttrInterfaceAllVerifiers),
                )] = &[
                    (
                        ::core::any::TypeId::of::<MyAttr>(),
                        <MyAttr as MyAttrInterface>::__all_verifiers,
                    ),
                ];
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
        "#]]
        .assert_eq(&prettyplease::unparse(&f));
    }

    #[test]
    fn type_interface() {
        let pairs = [(parse_quote! { MyType }, parse_quote! { MyTypeInterface })];
        let result = interface_registrations(&pairs, &InterfaceImplOpts::r#type());
        let f = syn::parse2::<syn::File>(result).unwrap();
        expect![[r#"
            ::pliron::type_to_trait!((MyType, MyTypeInterface));
            const _: () = {
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(::pliron::r#type::TYPE_INTERFACE_VERIFIERS),
                    linkme(crate = ::pliron::linkme)
                )]
                static INTERFACE_VERIFIER: &[(
                    ::core::any::TypeId,
                    (::pliron::r#type::TypeInterfaceAllVerifiers),
                )] = &[
                    (
                        ::core::any::TypeId::of::<MyType>(),
                        <MyType as MyTypeInterface>::__all_verifiers,
                    ),
                ];
                #[cfg(target_family = "wasm")]
                ::pliron::inventory::submit! {
                    ::pliron::InventoryWrapper(& INTERFACE_VERIFIER)
                }
            };
            impl ::pliron::r#type::TypeImplsInterface<dyn MyTypeInterface> for MyType {}
        "#]]
        .assert_eq(&prettyplease::unparse(&f));
    }
}
