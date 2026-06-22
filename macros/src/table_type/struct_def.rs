use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemStruct;

use super::parse::TableDef;

pub fn generate(table: &TableDef, input: &ItemStruct) -> TokenStream {
    let vis = &input.vis;
    let name = &table.name;
    let export_path = table.export_path();

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("table_type")).collect();

    let helper_attrs = ["pg", "crud", "api"];
    let fields: Vec<_> = input
        .fields
        .iter()
        .map(|f| {
            let mut field = f.clone();
            field.attrs.retain(|a| !helper_attrs.iter().any(|helper| a.path().is_ident(helper)));
            field
        })
        .collect();

    let validate_impl = crate::api_type::validate::generate(name, &input.generics, &input.fields);

    quote! {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
        #[ts(export, export_to = #export_path, optional_fields)]
        #(#user_attrs)*
        #vis struct #name {
            #(#fields),*
        }

        #validate_impl
    }
}
