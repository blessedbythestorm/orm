use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemStruct;

use super::parse::JsonDef;

pub fn generate(def: &JsonDef, input: &ItemStruct) -> TokenStream {
    let vis = &input.vis;
    let name = &def.name;
    let export_to = &def.export_to;

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("json_type")).collect();

    let fields = &input.fields;

    quote! {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
        #[ts(export, export_to = #export_to)]
        #(#user_attrs)*
        #vis struct #name #fields
    }
}
