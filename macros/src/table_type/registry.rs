use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::TableDef;

pub fn generate(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let name_str = name.to_string();

    let insert_name = format_ident!("{}Insert", name);
    let insert_name_str = format!("{}Insert", name);

    let update_name = format_ident!("{}Update", name);
    let update_name_str = format!("{}Update", name);

    quote! {
        inventory::submit! {
            ::orm::registry::TypeExport {
                name: #name_str,
                export_all: || <#name as ts_rs::TS>::export_all(),
            }
        }
        inventory::submit! {
            ::orm::registry::TypeExport {
                name: #insert_name_str,
                export_all: || <#insert_name as ts_rs::TS>::export_all(),
            }
        }
        inventory::submit! {
            ::orm::registry::TypeExport {
                name: #update_name_str,
                export_all: || <#update_name as ts_rs::TS>::export_all(),
            }
        }
    }
}
