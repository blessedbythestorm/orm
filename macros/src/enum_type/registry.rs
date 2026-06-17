use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;
    let name_str = name.to_string();

    quote! {
        inventory::submit! {
            ::orm::registry::TypeExport {
                name: #name_str,
                export_all: || <#name as ts_rs::TS>::export_all(),
            }
        }
    }
}
