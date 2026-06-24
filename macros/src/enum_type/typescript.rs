use proc_macro2::TokenStream;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let variants: Vec<String> = def.variants.iter().map(|v| v.value.clone()).collect();
    crate::export::enum_export(&def.name.to_string(), &def.export_to, &[], &variants)
}
