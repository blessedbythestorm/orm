use proc_macro2::TokenStream;
use quote::quote;

use super::parse::TableDef;

pub fn generate(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let field_extracts = generate_field_extracts(table);

    quote! {
        impl ::orm::FromRow for #name {
            fn from_row(row: &tokio_postgres::Row) -> Result<Self, tokio_postgres::Error> {
                Ok(#name { #(#field_extracts),* })
            }
        }
    }
}

fn generate_field_extracts(table: &TableDef) -> Vec<TokenStream> {
    table
        .fields
        .iter()
        .map(|f| {
            let name = &f.name;
            let name_str = &f.name_str;
            quote! { #name: row.try_get(#name_str)? }
        })
        .collect()
}
