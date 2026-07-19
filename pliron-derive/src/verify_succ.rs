// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! Derive macro for `Verify` trait that always succeeds.

pub(crate) fn verify_succ_impl(
    args: proc_macro2::TokenStream,
    input: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    if !args.is_empty() {
        return Err(syn::Error::new_spanned(
            args,
            "verify_succ does not accept any arguments",
        ));
    }

    let item = syn::parse2::<syn::Item>(input)?;
    match &item {
        syn::Item::Struct(item_struct) => {
            let ident = &item_struct.ident;
            let (impl_generics, ty_generics, where_clause) = item_struct.generics.split_for_impl();
            Ok(quote::quote! {
                #item

                impl #impl_generics ::pliron::common_traits::Verify for #ident #ty_generics #where_clause {
                    fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                        Ok(())
                    }
                }
            })
        }
        syn::Item::Enum(item_enum) => {
            let ident = &item_enum.ident;
            let (impl_generics, ty_generics, where_clause) = item_enum.generics.split_for_impl();
            Ok(quote::quote! {
                #item

                impl #impl_generics ::pliron::common_traits::Verify for #ident #ty_generics #where_clause {
                    fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                        Ok(())
                    }
                }
            })
        }
        _ => Err(syn::Error::new_spanned(
            item,
            "verify_succ can only be applied to a struct or enum",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::verify_succ_impl;
    use expect_test::expect;
    use quote::quote;

    #[test]
    fn verify_succ_on_generic_struct() {
        let input = quote! {
            struct Wrapper<T>
            where
                T: Clone,
            {
                value: T,
            }
        };
        let result = verify_succ_impl(proc_macro2::TokenStream::new(), input).unwrap();
        let file = syn::parse2::<syn::File>(result).unwrap();
        let got = prettyplease::unparse(&file);

        expect![[r##"
            struct Wrapper<T>
            where
                T: Clone,
            {
                value: T,
            }
            impl<T> ::pliron::common_traits::Verify for Wrapper<T>
            where
                T: Clone,
            {
                fn verify(&self, _ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn verify_succ_rejects_non_empty_arguments() {
        let result = verify_succ_impl(
            quote!(unused),
            quote!(
                struct Wrapper;
            ),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("verify_succ does not accept any arguments")
        );
    }
}
