use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::ViewDef;

/// Generates the read-only query surface for a view: a `<Name>View` trait with
/// `get_<name>s(QueryOptions)`. A view is just a relation, so this is the table
/// `get_all` body pointed at the view — filters/sort/limit work unchanged.
pub fn generate(view: &ViewDef) -> TokenStream {
    let name = &view.name;
    let trait_name = format_ident!("{}View", name);
    // Method reads from the view name (`name = "mentor_cards"` -> `get_mentor_cards`),
    // so name views in the plural; the struct name only drives the trait name.
    let get_all = format_ident!("get_{}", view.config.view);

    let base_sql = format!("SELECT {} FROM {}", view.column_list(), view.qualified_name());
    let err_msg = format!("Failed to query view {}", view.config.view);

    quote! {
        pub trait #trait_name {
            fn #get_all(
                &self,
                opts: ::orm::query::QueryOptions,
            ) -> impl std::future::Future<Output = anyhow::Result<Vec<#name>>> + Send;
        }

        impl #trait_name for deadpool_postgres::Pool {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                use ::orm::FromRow;

                let client = self.get().await?;
                let (where_clause, _) = opts.build_where_clause(1);
                let suffix = opts.to_sql_suffix();
                let sql = format!("{}{}{}", #base_sql, where_clause, suffix);

                let rows = client.query(&sql, &opts.filter_params()).await
                    .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

                rows.iter()
                    .map(|row| #name::from_row(row).map_err(|e| anyhow::anyhow!("Row parse error: {}", e)))
                    .collect()
            }
        }
    }
}
