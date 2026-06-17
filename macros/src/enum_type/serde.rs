use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;
    let expected = def.expected_values();

    let serialize_arms = generate_serialize_arms(def);
    let deserialize_arms = generate_deserialize_arms(def);

    quote! {
        impl serde::Serialize for #name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                let s = match self {
                    #(#serialize_arms),*
                };
                serializer.serialize_str(s)
            }
        }

        impl<'de> serde::Deserialize<'de> for #name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                match s.as_str() {
                    #(#deserialize_arms,)*
                    _ => Err(serde::de::Error::custom(
                        format!("Unknown value: {}. Expected one of: {}", s, #expected)
                    ))
                }
            }
        }
    }
}

fn generate_serialize_arms(def: &EnumDef) -> Vec<TokenStream> {
    let name = &def.name;
    def.variants
        .iter()
        .map(|v| {
            let ident = &v.ident;
            let value = &v.value;
            quote! { #name::#ident => #value }
        })
        .collect()
}

fn generate_deserialize_arms(def: &EnumDef) -> Vec<TokenStream> {
    let name = &def.name;
    def.variants
        .iter()
        .map(|v| {
            let ident = &v.ident;
            let value = &v.value;
            quote! { #value => Ok(#name::#ident) }
        })
        .collect()
}
