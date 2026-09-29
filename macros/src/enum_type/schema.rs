use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;
    let qualified_name = def.qualified_name();
    let rust_name = name.to_string();
    let variants = def.variants.iter()
        .map(|variant| {
            let rust_name = variant.ident.to_string();
            let value = &variant.value;
            quote! {
                ::orm::schema::registry::EnumVariantItem {
                    rust_name: #rust_name,
                    value: #value,
                }
            }
        });

    quote! {
        impl ::orm::schema::SqlType for #name {
            const SQL_TYPE: &'static str = #qualified_name;
            const NULLABLE: bool = false;
        }

        inventory::submit! {
            ::orm::schema::registry::EnumItem {
                name: #qualified_name,
                rust_name: #rust_name,
                variants: &[ #(#variants),* ],
            }
        }
    }
}
