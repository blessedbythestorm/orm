use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::ViewDef;

/// Generates the read-only query surface for a view: a `<Name>View` trait with
/// `get_<name>s(QueryOptions)`. A view is just a relation, so this is the table
/// `get_all` body pointed at the view — filters/sort/limit work unchanged.
pub fn generate(view: &ViewDef) -> TokenStream {
    let name = &view.name;
    let relation = view.qualified_name();
    let columns = view.fields.iter()
        .map(|field| field.name_str.as_str());

    let trait_name = format_ident!("{}View", name);
    // Method reads from the view name (`name = "mentor_cards"` -> `get_mentor_cards`),
    // so name views in the plural; the struct name only drives the trait name.
    let get_all = format_ident!("get_{}", view.config.view);

    quote! {
        impl ::orm::query::QueryModel for #name {
            const RELATION: &'static str = #relation;
            const COLUMNS: &'static [&'static str] = &[#(#columns),*];
        }

        pub trait #trait_name {
            fn #get_all(
                &self,
                opts: ::orm::query::QueryOptions,
            ) -> impl std::future::Future<Output = anyhow::Result<Vec<#name>>> + Send;
        }

        impl<C: ::orm::query::QueryBuilderExt + ?Sized> #trait_name for C {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                self.select::<#name>().options(opts).fetch_all().await
            }
        }
    }
}
