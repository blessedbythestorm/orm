use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;
    let qualified_name = def.qualified_name();
    let values = def.variants.iter().map(|variant| &variant.value);

    quote! {
        impl ::orm::schema::SqlType for #name {
            const SQL_TYPE: &'static str = #qualified_name;
            const NULLABLE: bool = false;
        }

        inventory::submit! {
            ::orm::schema::registry::EnumItem {
                name: #qualified_name,
                values: &[ #(#values),* ],
            }
        }
    }
}
