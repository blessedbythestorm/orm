use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::TableDef;

pub fn generate(table: &TableDef) -> TokenStream {
    let name = format_ident!("{}Update", table.name);
    let export_path = table.export_path();
    let fields = generate_fields(table);

    quote! {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, validator::Validate, ts_rs::TS)]
        #[ts(export, export_to = #export_path, optional_fields)]
        pub struct #name {
            #(#fields),*
        }
    }
}

fn generate_fields(table: &TableDef) -> Vec<TokenStream> {
    table
        .update_fields()
        .map(|f| {
            let name = &f.name;
            let ty = f.as_option_type();
            let validate_attrs = &f.validate_attrs;

            quote! {
                #[ts(optional)]
                #(#validate_attrs)*
                pub #name: #ty
            }
        })
        .collect()
}
