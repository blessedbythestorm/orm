use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;
    let pg_name = &def.pg_name;
    let expected = def.expected_values();

    let to_sql_arms = generate_to_sql_arms(def);
    let from_sql_arms = generate_from_sql_arms(def);

    quote! {
        impl postgres_types::ToSql for #name {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                out: &mut bytes::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Send + Sync>> {
                let s = match self {
                    #(#to_sql_arms),*
                };
                <&str as postgres_types::ToSql>::to_sql(&s, ty, out)
            }

            fn accepts(ty: &postgres_types::Type) -> bool {
                ty.name() == #pg_name
            }

            postgres_types::to_sql_checked!();
        }

        impl<'a> postgres_types::FromSql<'a> for #name {
            fn from_sql(
                ty: &postgres_types::Type,
                raw: &'a [u8],
            ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                let s = <&str as postgres_types::FromSql>::from_sql(ty, raw)?;
                match s {
                    #(#from_sql_arms,)*
                    _ => Err(format!(
                        "Unknown {} value: {}. Expected one of: {}",
                        #pg_name, s, #expected
                    ).into())
                }
            }

            fn accepts(ty: &postgres_types::Type) -> bool {
                ty.name() == #pg_name
            }
        }
    }
}

fn generate_to_sql_arms(def: &EnumDef) -> Vec<TokenStream> {
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

fn generate_from_sql_arms(def: &EnumDef) -> Vec<TokenStream> {
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
