use proc_macro2::TokenStream;
use quote::quote;

use super::parse::EnumDef;

/// Lets the enum be used as a `QueryOptions` filter value — it's already `ToSql`,
/// so binding it is enough. `Option<Enum>` is covered by orm's blanket
/// `impl<T: FilterValue> FilterValue for Option<T>` (a user crate can't impl that
/// itself — `Option` is foreign and non-fundamental).
pub fn generate(def: &EnumDef) -> TokenStream {
    let name = &def.name;

    quote! {
        impl ::orm::query::FilterValue for #name {
            fn into_filter_value(
                self,
                _op: ::orm::query::FilterOp,
            ) -> Option<std::sync::Arc<dyn tokio_postgres::types::ToSql + Send + Sync>> {
                Some(std::sync::Arc::new(self))
            }
        }
    }
}
