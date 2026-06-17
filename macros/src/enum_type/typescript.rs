use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;
    let name_str = name.to_string();
    let export_to = &def.export_to;

    let inline_str = def.variants.iter().map(|v| format!("\"{}\"", v.value)).collect::<Vec<_>>().join(" | ");

    quote! {
        impl ts_rs::TS for #name {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;

            fn name() -> String {
                #name_str.to_owned()
            }

            fn inline() -> String {
                #inline_str.to_owned()
            }

            fn inline_flattened() -> String {
                Self::inline()
            }

            fn decl() -> String {
                format!("type {} = {};", Self::name(), Self::inline())
            }

            fn decl_concrete() -> String {
                Self::decl()
            }

            fn output_path() -> Option<std::path::PathBuf> {
                Some(std::path::PathBuf::from(#export_to))
            }
        }
    }
}
