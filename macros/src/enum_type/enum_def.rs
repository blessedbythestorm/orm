use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemEnum;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef, input: &ItemEnum) -> TokenStream {
    let vis = &input.vis;
    let name = &def.name;

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("enum_type")).collect();

    let variants = &input.variants;

    quote! {
        #[derive(Debug, Clone, PartialEq, Eq)]
        #(#user_attrs)*
        #vis enum #name {
            #variants
        }
    }
}
