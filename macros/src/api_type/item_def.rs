use proc_macro2::TokenStream;
use quote::quote;
use syn::Item;

use super::parse::ApiDef;

pub fn generate(def: &ApiDef, input: &Item) -> TokenStream {
    let export_to = &def.export_to;

    match input {
        Item::Struct(s) => generate_struct(s, export_to),
        Item::Enum(e) => generate_enum(e, export_to),
        _ => panic!("api_type only supports structs and enums"),
    }
}

fn generate_struct(input: &syn::ItemStruct, export_to: &str) -> TokenStream {
    let vis = &input.vis;
    let name = &input.ident;
    let generics = &input.generics;
    let fields = &input.fields;

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("api_type")).collect();

    quote! {
        #[derive(Debug, serde::Deserialize, serde::Serialize, validator::Validate, ts_rs::TS)]
        #[ts(export, export_to = #export_to, optional_fields)]
        #(#user_attrs)*
        #vis struct #name #generics #fields
    }
}

fn generate_enum(input: &syn::ItemEnum, export_to: &str) -> TokenStream {
    let vis = &input.vis;
    let name = &input.ident;
    let generics = &input.generics;
    let variants = &input.variants;

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("api_type")).collect();

    quote! {
        #[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, ts_rs::TS)]
        #[ts(export, export_to = #export_to)]
        #(#user_attrs)*
        #vis enum #name #generics {
            #variants
        }
    }
}
