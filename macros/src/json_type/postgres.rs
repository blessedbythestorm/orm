use proc_macro2::TokenStream;
use quote::quote;

use super::parse::JsonDef;

pub fn generate(def: &JsonDef) -> TokenStream {
    let name = &def.name;

    quote! {
        impl postgres_types::ToSql for #name {
            fn to_sql(
                &self,
                ty: &postgres_types::Type,
                out: &mut bytes::BytesMut,
            ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Send + Sync>> {
                let json = serde_json::to_value(self)?;
                <serde_json::Value as postgres_types::ToSql>::to_sql(&json, ty, out)
            }

            fn accepts(ty: &postgres_types::Type) -> bool {
                ty.name() == "jsonb" || ty.name() == "json"
            }

            postgres_types::to_sql_checked!();
        }

        impl<'a> postgres_types::FromSql<'a> for #name {
            fn from_sql(
                ty: &postgres_types::Type,
                raw: &'a [u8],
            ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                let json = <serde_json::Value as postgres_types::FromSql>::from_sql(ty, raw)?;
                Ok(serde_json::from_value(json)?)
            }

            fn accepts(ty: &postgres_types::Type) -> bool {
                ty.name() == "jsonb" || ty.name() == "json"
            }
        }
    }
}
