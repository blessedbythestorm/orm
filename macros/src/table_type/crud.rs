use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::{ConstraintKindSpec, TableDef};

pub fn generate(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let relation = table.full_table_name();
    let columns = table.fields.iter()
        .map(|field| field.name_str.as_str());

    let trait_def = generate_trait(table);
    let trait_impl = generate_impl(table);

    quote! {
        impl ::orm::query::QueryModel for #name {
            const RELATION: &'static str = #relation;
            const COLUMNS: &'static [&'static str] = &[#(#columns),*];
        }

        impl ::orm::query::TableModel for #name {}

        #trait_def
        #trait_impl
    }
}

fn generate_trait(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let trait_name = format_ident!("{}Crud", name);
    let insert_name = format_ident!("{}Insert", name);
    let update_name = format_ident!("{}Update", name);

    let get_all = format_ident!("get_{}s", table.name_snake);
    let count_all = format_ident!("count_{}s", table.name_snake);
    let get_one = format_ident!("get_{}", table.name_snake);
    let create = format_ident!("create_{}", table.name_snake);
    let update = format_ident!("update_{}", table.name_snake);
    let delete = format_ident!("delete_{}", table.name_snake);
    let delete_all = format_ident!("delete_{}s", table.name_snake);
    let insert_fields = format_ident!("insert_{}_fields", table.name_snake);
    let update_where = format_ident!("update_{}s_where", table.name_snake);
    let upserts = unique_keys(table)
        .into_iter()
        .map(|columns| {
            let method = upsert_method(table, &columns);
            let selective_method = format_ident!("{}_with", method);

            quote! {
                fn #method(&self, data: &#insert_name) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
                fn #selective_method(&self, data: &#insert_name, update: &#update_name) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            }
        });

    quote! {
        pub trait #trait_name {
            fn #get_all(&self, opts: ::orm::query::QueryOptions) -> impl std::future::Future<Output = anyhow::Result<Vec<#name>>> + Send;
            fn #count_all(&self, opts: ::orm::query::QueryOptions) -> impl std::future::Future<Output = anyhow::Result<i64>> + Send;
            fn #get_one(&self, id: &uuid::Uuid) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #create(&self, data: &#insert_name) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #delete(&self, id: &uuid::Uuid) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
            fn #delete_all(&self, opts: ::orm::query::QueryOptions) -> impl std::future::Future<Output = anyhow::Result<u64>> + Send;
            fn #insert_fields(&self, values: ::orm::query::InsertValues) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #update_where(&self, opts: ::orm::query::QueryOptions, values: ::orm::query::UpdateValues) -> impl std::future::Future<Output = anyhow::Result<Vec<#name>>> + Send;
            #(#upserts)*
        }
    }
}

fn generate_impl(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let trait_name = format_ident!("{}Crud", name);
    let insert_name = format_ident!("{}Insert", name);
    let update_name = format_ident!("{}Update", name);
    let get_all = format_ident!("get_{}s", table.name_snake);
    let count_all = format_ident!("count_{}s", table.name_snake);
    let get_one = format_ident!("get_{}", table.name_snake);
    let create = format_ident!("create_{}", table.name_snake);
    let update = format_ident!("update_{}", table.name_snake);
    let delete = format_ident!("delete_{}", table.name_snake);
    let delete_all = format_ident!("delete_{}s", table.name_snake);
    let insert_fields = format_ident!("insert_{}_fields", table.name_snake);
    let update_where = format_ident!("update_{}s_where", table.name_snake);
    let primary_key = table.primary_key_name();
    let not_found = format!("{} not found", name);
    let no_updates = format!("No fields to update for {}", table.name_snake);
    let insert_values = collect_insert_values(table);
    let update_values = collect_update_values(table, &format_ident!("data"), &[]);
    let upserts = generate_upserts(table);

    quote! {
        impl<C: ::orm::query::QueryBuilderExt + ?Sized> #trait_name for C {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                self.select::<#name>().options(opts).fetch_all().await
            }

            async fn #count_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<i64> {
                self.select::<#name>().options(opts).count().await
            }

            async fn #get_one(&self, id: &uuid::Uuid) -> anyhow::Result<#name> {
                self.select::<#name>().where_(#primary_key, ::orm::query::FilterOp::Eq, *id).fetch_one().await
            }

            async fn #create(&self, data: &#insert_name) -> anyhow::Result<#name> {
                ::orm::validate::Validate::validate(data)?;
                #insert_values
                self.insert::<#name>().values(values).returning_one().await
            }

            async fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> anyhow::Result<#name> {
                ::orm::validate::Validate::validate(data)?;
                #update_values
                if values.is_empty() {
                    anyhow::bail!(#no_updates);
                }
                self.update::<#name>().values(values).where_(#primary_key, ::orm::query::FilterOp::Eq, *id).returning_one().await
            }

            async fn #delete(&self, id: &uuid::Uuid) -> anyhow::Result<()> {
                let affected = self.delete::<#name>().where_(#primary_key, ::orm::query::FilterOp::Eq, *id).execute().await?;
                if affected == 0 {
                    anyhow::bail!(#not_found);
                }
                Ok(())
            }

            async fn #delete_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<u64> {
                self.delete::<#name>().options(opts).execute().await
            }

            async fn #insert_fields(&self, values: ::orm::query::InsertValues) -> anyhow::Result<#name> {
                self.insert::<#name>().values(values).returning_one().await
            }

            async fn #update_where(&self, opts: ::orm::query::QueryOptions, values: ::orm::query::UpdateValues) -> anyhow::Result<Vec<#name>> {
                self.update::<#name>().options(opts).values(values).returning().await
            }

            #(#upserts)*
        }
    }
}

fn collect_insert_values(table: &TableDef) -> TokenStream {
    let fields = table.insert_fields()
        .map(|field| {
            let name = &field.name;
            let column = &field.name_str;
            if field.is_auto_generated {
                quote! {
                    if let Some(value) = &data.#name {
                        values = values.value(#column, value.clone());
                    }
                }
            } else {
                quote! { values = values.value(#column, data.#name.clone()); }
            }
        });

    quote! {
        let mut values = ::orm::query::InsertValues::new();
        #(#fields)*
    }
}

fn collect_update_values(table: &TableDef, data: &syn::Ident, excluded: &[String]) -> TokenStream {
    let fields = table.update_fields()
        .filter(|field| !excluded.contains(&field.name_str))
        .map(|field| {
            let name = &field.name;
            let column = &field.name_str;
            quote! {
                if let Some(value) = &#data.#name {
                    values = values.assign(#column, value.clone());
                }
            }
        });

    quote! {
        let mut values = ::orm::query::UpdateValues::new();
        #(#fields)*
    }
}

fn unique_keys(table: &TableDef) -> Vec<Vec<String>> {
    let mut keys: Vec<Vec<String>> = table.fields.iter()
        .filter(|field| field.is_primary || field.is_unique)
        .map(|field| vec![field.name_str.clone()])
        .collect();

    for constraint in &table.constraints {
        if let ConstraintKindSpec::Unique { columns } = &constraint.kind {
            if !keys.contains(columns) {
                keys.push(columns.clone());
            }
        }
    }

    keys
}

fn upsert_method(table: &TableDef, columns: &[String]) -> syn::Ident {
    format_ident!("upsert_{}_by_{}", table.name_snake, columns.join("_and_"))
}

fn generate_upserts(table: &TableDef) -> Vec<TokenStream> {
    let name = &table.name;
    let insert_name = format_ident!("{}Insert", name);
    let update_name = format_ident!("{}Update", name);
    unique_keys(table)
        .into_iter()
        .map(|columns| {
            let method = upsert_method(table, &columns);
            let selective_method = format_ident!("{}_with", method);
            let insert_values = collect_insert_values(table);
            let update_values = collect_update_values(table, &format_ident!("update"), &columns);
            let mut update_columns: Vec<_> = table.update_fields()
                .filter(|field| !columns.contains(&field.name_str))
                .map(|field| field.name_str.as_str())
                .collect();

            let no_op = &columns[0];
            if update_columns.is_empty() {
                update_columns.push(no_op);
            }

            quote! {
                async fn #method(&self, data: &#insert_name) -> anyhow::Result<#name> {
                    ::orm::validate::Validate::validate(data)?;
                    #insert_values
                    self.insert::<#name>().values(values)
                        .on_conflict(&[#(#columns),*])
                        .do_update_excluded(&[#(#update_columns),*])
                        .returning_one().await
                }

                async fn #selective_method(&self, data: &#insert_name, update: &#update_name) -> anyhow::Result<#name> {
                    ::orm::validate::Validate::validate(data)?;
                    ::orm::validate::Validate::validate(update)?;
                    #insert_values
                    let insert = self.insert::<#name>().values(values).on_conflict(&[#(#columns),*]);
                    #update_values
                    let insert = if values.is_empty() {
                        insert.do_update_excluded(&[#no_op])
                    } else {
                        insert.do_update(values)
                    };
                    insert.returning_one().await
                }
            }
        })
        .collect()
}
