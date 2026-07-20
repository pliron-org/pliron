// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{DeriveInput, LitStr, Result};

const PROC_MACRO_NAME: &str = "def_attribute";

pub(crate) fn def_attribute(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let name = syn::parse2::<LitStr>(args.into())?;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DefAttribute::derive(name, input)?;
    Ok(p.into_token_stream())
}

/// The derived macro body for the `#[def_attribute]` proc macro.
struct DefAttribute {
    input: DeriveInput,
    impl_attr: ImplAttribute,
}

impl DefAttribute {
    fn derive(name: LitStr, input: DeriveInput) -> Result<Self> {
        let name_str = name.value();
        let Some((dialect_name, attr_name)) = name_str.split_once('.') else {
            return Err(syn::Error::new_spanned(
                name,
                "attribute name must be in the form of `dialect.attr_name`",
            ));
        };

        match input.data {
            syn::Data::Struct(_) | syn::Data::Enum(_) => {}
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "Attribute can only be derived for structs or enums",
                ));
            }
        }

        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "Attribute cannot be derived for generic structs or enums",
            ));
        }

        let attrs = input
            .attrs
            .into_iter()
            .filter(|attr| !attr.path().is_ident(PROC_MACRO_NAME))
            .collect();

        let input = DeriveInput { attrs, ..input };

        let impl_attr = ImplAttribute {
            ident: input.ident.clone(),
            dialect_name: dialect_name.to_string(),
            attr_name: attr_name.to_string(),
        };

        Ok(Self { input, impl_attr })
    }
}

impl ToTokens for DefAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let def_struct = &self.input;
        let impl_attribute_trait = &self.impl_attr;
        tokens.extend(quote! {
            #def_struct

            #impl_attribute_trait
        });
    }
}

struct ImplAttribute {
    ident: syn::Ident,
    attr_name: String,
    dialect_name: String,
}

impl ToTokens for ImplAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.ident;
        let attr_name = &self.attr_name;
        let dialect = &self.dialect_name;
        tokens.extend(quote! {
            impl ::pliron::attribute::Attribute for #name {
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other
                        .downcast_ref::<Self>()
                        .map_or(false, |other| other == self)
                }

                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }

                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::attribute::AttrName::try_new(#attr_name).expect("Invalid Identifier for AttrName"),
                        dialect: ::pliron::dialect::DialectName::try_new(#dialect).expect("Invalid Identifier for DialectName"),
                    }
                }

                fn verify_interfaces(&self, ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) =
                        ::pliron::attribute::ATTR_INTERFACE_VERIFIERS_MAP.get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }

            ::pliron::context_registration!(<#name as ::pliron::attribute::Attribute>::register);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn simple() {
        let args = quote! { "testing.unit" };
        let input = quote! {
            #[derive(PartialEq, Eq, Debug, Clone)]
            pub struct UnitAttr;
        };
        let attr = def_attribute(args, input).unwrap();
        let f = syn::parse2::<syn::File>(attr).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(PartialEq, Eq, Debug, Clone)]
            pub struct UnitAttr;
            impl ::pliron::attribute::Attribute for UnitAttr {
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }
                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::attribute::AttrName::try_new("unit")
                            .expect("Invalid Identifier for AttrName"),
                        dialect: ::pliron::dialect::DialectName::try_new("testing")
                            .expect("Invalid Identifier for DialectName"),
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
                < UnitAttr as ::pliron::attribute::Attribute > ::register
            );
        "##]]
        .assert_eq(&got);
    }
}
