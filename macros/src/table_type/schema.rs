use proc_macro2::TokenStream;
use quote::quote;

use super::parse::{FieldDef, ForeignKeySpec, TableDef};

pub fn generate(table: &TableDef) -> TokenStream {
    let schema = &table.config.schema;
    let name = &table.config.table;
    let columns = table.fields.iter().map(column_item);

    quote! {
        inventory::submit! {
            ::orm::schema::registry::TableItem {
                schema: #schema,
                name: #name,
                columns: &[ #(#columns),* ],
            }
        }
    }
}

fn column_item(field: &FieldDef) -> TokenStream {
    let name = &field.name_str;
    let ty = &field.ty;
    let primary_key = field.is_primary;
    let unique = field.is_unique;
    let default = match &field.default {
        Some(value) => quote! { Some(#value) },
        None => quote! { None },
    };
    let foreign_key = match &field.foreign_key {
        Some(spec) => foreign_key_item(spec),
        None => quote! { None },
    };

    quote! {
        ::orm::schema::registry::ColumnItem {
            name: #name,
            sql_type: <#ty as ::orm::schema::SqlType>::SQL_TYPE,
            nullable: <#ty as ::orm::schema::SqlType>::NULLABLE,
            primary_key: #primary_key,
            unique: #unique,
            default: #default,
            foreign_key: #foreign_key,
        }
    }
}

fn foreign_key_item(spec: &ForeignKeySpec) -> TokenStream {
    let schema = &spec.schema;
    let table = &spec.table;
    let column = &spec.column;
    let on_update = spec.on_update.path();
    let on_delete = spec.on_delete.path();

    quote! {
        Some(::orm::schema::registry::ForeignKeyItem {
            schema: #schema,
            table: #table,
            column: #column,
            on_update: #on_update,
            on_delete: #on_delete,
        })
    }
}
