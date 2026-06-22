use proc_macro2::TokenStream;
use quote::quote;

use super::parse::ViewDef;

pub fn generate(view: &ViewDef) -> TokenStream {
    let name = &view.name;
    let field_extracts = view.fields.iter().map(|f| {
        let ident = &f.name;
        let name_str = &f.name_str;
        quote! { #ident: row.try_get(#name_str)? }
    });

    quote! {
        impl ::orm::FromRow for #name {
            fn from_row(row: &tokio_postgres::Row) -> Result<Self, tokio_postgres::Error> {
                Ok(#name { #(#field_extracts),* })
            }
        }
    }
}
