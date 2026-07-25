use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemEnum;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef, input: &ItemEnum) -> TokenStream {
    let vis = &input.vis;
    let name = &def.name;

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("enum_type")).collect();

    let variants = &input.variants;

    let all: Vec<TokenStream> = def
        .variants
        .iter()
        .map(|v| {
            let ident = &v.ident;

            quote! { #name::#ident }
        })
        .collect();

    let as_str_arms: Vec<TokenStream> = def
        .variants
        .iter()
        .map(|v| {
            let ident = &v.ident;
            let value = &v.value;

            quote! { #name::#ident => #value }
        })
        .collect();

    quote! {
        #[derive(Debug, Clone, PartialEq, Eq)]
        #(#user_attrs)*
        #vis enum #name {
            #variants
        }

        impl #name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [#name] = &[#(#all),*];

            /// The wire name — the one string this variant is known by in JSON,
            /// Postgres and TypeScript alike.
            pub fn as_str(&self) -> &'static str {
                match self {
                    #(#as_str_arms),*
                }
            }
        }
    }
}
