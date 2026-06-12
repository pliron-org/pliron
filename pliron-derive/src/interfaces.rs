//! Attribute macros for defining and implementing Interfaces.

use quote::{ToTokens, quote};
use syn::{
    DeriveInput, ItemImpl, ItemTrait, Path, Result, Token, TypeParamBound, parse::Parse,
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

/// This macro does two things:
/// 1. Mark that the rust type be castable to the interface via type_to_trait.
/// 2. Register the trait verifier in the rust type's list of interface verifiers.
pub(crate) fn interface_impl(
    input: proc_macro::TokenStream,
    interface_verifiers_slice: Path,
    all_verifiers_fn_type: Path,
) -> Result<proc_macro2::TokenStream> {
    let r#impl = syn::parse2::<ItemImpl>(input.into())?;

    let Some((_, intr_name, _)) = r#impl.trait_.clone() else {
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

    let rust_ty = (*r#impl.self_ty).clone();
    let trait_cast = quote! {
        ::pliron::type_to_trait!(#rust_ty, #intr_name);
    };
    let verifiers_entry = quote! {
        const _: () = {
            #[cfg_attr(not(target_family = "wasm"), ::pliron::linkme::distributed_slice(#interface_verifiers_slice), linkme(crate = ::pliron::linkme))]
            static INTERFACE_VERIFIER: ::pliron::deps::sync::LazyLock<(::core::any::TypeId, (#all_verifiers_fn_type))> =
                ::pliron::deps::sync::LazyLock::new(|| {
                    ::pliron::log::debug!(
                        target: "interface_registration",
                        "{}({:?}) for type {}({:?})",
                        ::core::any::type_name::<dyn #intr_name>(),
                        ::core::any::TypeId::of::<dyn #intr_name>(),
                        ::core::any::type_name::<#rust_ty>(),
                        ::core::any::TypeId::of::<#rust_ty>(),
                    );
                    (::core::any::TypeId::of::<#rust_ty>(),
                        <#rust_ty as #intr_name>::__all_verifiers)
                });

            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! {
                ::pliron::utils::inventory::LazyLockWrapper(&INTERFACE_VERIFIER)
            }
        };
    };

    let mut output = r#impl.to_token_stream();
    output.extend(trait_cast);
    output.extend(verifiers_entry);

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

/// For each interface specified in the list, expand it to
/// ```no_compile
/// #[op_interface_impl]
/// impl Interface for OpStruct { }
/// ```
pub(crate) fn derive_op_interface_impl(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> Result<proc_macro2::TokenStream> {
    let intrs = syn::parse2::<PathList>(attr.into())?;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let struct_name = input.ident.clone();

    let impls = intrs.paths.into_iter().map(|path| {
        quote! {
            #[::pliron::derive::op_interface_impl]
            impl #path for #struct_name {}
        }
    });

    let mut output = input.to_token_stream();
    output.extend(impls);

    Ok(output)
}
