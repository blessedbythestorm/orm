use proc_macro2::TokenStream;
use quote::quote;

use super::parse::ViewDef;

/// Registers the view with the migration engine as a column map + optional
/// filter/order. The engine infers the FROM table and JOINs from foreign keys.
pub fn generate(view: &ViewDef) -> TokenStream {
    let schema = &view.config.schema;
    let name = &view.config.view;

    let columns = view.fields.iter().map(|f| {
        let alias = &f.name_str;
        let col_schema = &f.source.schema;
        let col_table = &f.source.table;
        let col_name = &f.source.column;
        quote! {
            ::orm::schema::registry::ViewColumnItem {
                alias: #alias,
                schema: #col_schema,
                table: #col_table,
                column: #col_name,
            }
        }
    });

    let filter = opt_str(&view.config.filter);
    let order_by = opt_str(&view.config.order_by);

    quote! {
        inventory::submit! {
            ::orm::schema::registry::ViewItem {
                schema: #schema,
                name: #name,
                columns: &[ #(#columns),* ],
                filter: #filter,
                order_by: #order_by,
            }
        }
    }
}

fn opt_str(value: &Option<String>) -> TokenStream {
    match value {
        Some(s) => quote! { Some(#s) },
        None => quote! { None },
    }
}
