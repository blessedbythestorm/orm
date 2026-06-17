use proc_macro2::TokenStream;
use quote::quote;

use super::parse::JsonDef;

pub fn generate(def: &JsonDef) -> TokenStream {
    let name = &def.name;

    quote! {
        impl ::orm::schema::SqlType for #name {
            const SQL_TYPE: &'static str = "jsonb";
            const NULLABLE: bool = false;
        }
    }
}
