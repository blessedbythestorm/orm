use proc_macro2::TokenStream;
use quote::quote;

use super::parse::ViewDef;

/// Registers the projection struct for TypeScript export (same channel tables use).
pub fn generate(view: &ViewDef) -> TokenStream {
    let name = &view.name;
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
