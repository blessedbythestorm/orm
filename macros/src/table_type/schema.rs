use proc_macro2::TokenStream;
use quote::quote;

use super::parse::{
    ConstraintKindSpec, ConstraintSpec, FieldDef, ForeignKeySpec, IndexSpec, TableDef,
};

pub fn generate(table: &TableDef) -> TokenStream {
    let schema = &table.config.schema;
    let name = &table.config.table;
    let columns = table.fields.iter().map(column_item);
    let constraints = table.constraints.iter().map(constraint_item);
    let indexes = table.indexes.iter().map(index_item);

    quote! {
        inventory::submit! {
            ::orm::schema::registry::TableItem {
                schema: #schema,
                name: #name,
                columns: &[ #(#columns),* ],
                constraints: &[ #(#constraints),* ],
                indexes: &[ #(#indexes),* ],
            }
        }
    }
}

fn constraint_item(spec: &ConstraintSpec) -> TokenStream {
    let name = &spec.name;
    let kind = match &spec.kind {
        ConstraintKindSpec::Unique { columns } => {
            let columns = columns.iter().map(String::as_str);
            quote! {
                ::orm::schema::registry::ConstraintKindItem::Unique { columns: &[ #(#columns),* ] }
            }
        }
        ConstraintKindSpec::Check { expression } => quote! {
            ::orm::schema::registry::ConstraintKindItem::Check { expression: #expression }
        },
    };

    quote! {
        ::orm::schema::registry::ConstraintItem { name: #name, kind: #kind }
    }
}

fn index_item(spec: &IndexSpec) -> TokenStream {
    let name = &spec.name;
    let columns = spec.columns.iter().map(String::as_str);
    let unique = spec.unique;
    let predicate = match &spec.predicate {
        Some(value) => quote! { Some(#value) },
        None => quote! { None },
    };

    quote! {
        ::orm::schema::registry::IndexItem {
            name: #name,
            columns: &[ #(#columns),* ],
            unique: #unique,
            predicate: #predicate,
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
