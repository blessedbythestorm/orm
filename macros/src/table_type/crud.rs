use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::TableDef;

pub fn generate(table: &TableDef) -> TokenStream {
    let trait_def = generate_trait(table);
    let trait_impl = generate_impl(table);

    quote! {
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
    let get_one = format_ident!("get_{}", table.name_snake);
    let create = format_ident!("create_{}", table.name_snake);
    let update = format_ident!("update_{}", table.name_snake);
    let delete = format_ident!("delete_{}", table.name_snake);

    quote! {
        pub trait #trait_name {
            fn #get_all(&self, opts: ::orm::query::QueryOptions) -> impl std::future::Future<Output = anyhow::Result<Vec<#name>>> + Send;
            fn #get_one(&self, id: &uuid::Uuid) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #create(&self, data: &#insert_name) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> impl std::future::Future<Output = anyhow::Result<#name>> + Send;
            fn #delete(&self, id: &uuid::Uuid) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
        }
    }
}

fn generate_impl(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let trait_name = format_ident!("{}Crud", name);
    let insert_name = format_ident!("{}Insert", name);
    let update_name = format_ident!("{}Update", name);

    let get_all = format_ident!("get_{}s", table.name_snake);
    let get_one = format_ident!("get_{}", table.name_snake);
    let create = format_ident!("create_{}", table.name_snake);
    let update = format_ident!("update_{}", table.name_snake);
    let delete = format_ident!("delete_{}", table.name_snake);

    let get_all_body = generate_get_all(table);
    let get_one_body = generate_get_one(table);
    let create_body = generate_create(table);
    let update_body = generate_update(table);
    let delete_body = generate_delete(table);

    quote! {
        impl #trait_name for deadpool_postgres::Pool {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                #get_all_body
            }

            async fn #get_one(&self, id: &uuid::Uuid) -> anyhow::Result<#name> {
                #get_one_body
            }

            async fn #create(&self, data: &#insert_name) -> anyhow::Result<#name> {
                #create_body
            }

            async fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> anyhow::Result<#name> {
                #update_body
            }

            async fn #delete(&self, id: &uuid::Uuid) -> anyhow::Result<()> {
                #delete_body
            }
        }
    }
}

fn generate_get_all(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let base_sql = format!("SELECT {} FROM {}", columns, full_table);
    let err_msg = format!("Failed to get {}s", table.name_snake);

    quote! {
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

fn generate_get_one(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let sql = format!("SELECT {} FROM {} WHERE id = $1", columns, full_table);
    let err_msg = format!("Failed to get {}", table.name_snake);

    quote! {
        use ::orm::FromRow;

        let client = self.get().await?;
        let row = client.query_one(#sql, &[id]).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        #name::from_row(&row).map_err(|e| anyhow::anyhow!("Row parse error: {}", e))
    }
}

fn generate_create(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let err_msg = format!("Failed to create {}", table.name_snake);

    let collectors = table.insert_fields().map(|f| {
        let field = &f.name;
        let field_str = &f.name_str;

        if f.is_auto_generated {
            quote! {
                if let Some(ref value) = data.#field {
                    insert_columns.push(#field_str);
                    params.push(value as &(dyn tokio_postgres::types::ToSql + Sync));
                }
            }
        } else {
            quote! {
                insert_columns.push(#field_str);
                params.push(&data.#field as &(dyn tokio_postgres::types::ToSql + Sync));
            }
        }
    });

    quote! {
        use ::orm::FromRow;

        let client = self.get().await?;
        let mut insert_columns: Vec<&str> = Vec::new();
        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();

        #(#collectors)*

        let sql = if insert_columns.is_empty() {
            format!("INSERT INTO {} DEFAULT VALUES RETURNING {}", #full_table, #columns)
        } else {
            let placeholders = (1..=insert_columns.len())
                .map(|i| format!("${}", i))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "INSERT INTO {} ({}) VALUES ({}) RETURNING {}",
                #full_table,
                insert_columns.join(", "),
                placeholders,
                #columns
            )
        };

        let row = client.query_one(&sql, &params).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        #name::from_row(&row).map_err(|e| anyhow::anyhow!("Row parse error: {}", e))
    }
}

fn generate_update(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let err_msg = format!("Failed to update {}", table.name_snake);
    let no_fields_err = format!("No fields to update for {}", table.name_snake);

    let update_fields: Vec<_> = table.update_fields().collect();

    let set_clause_builders = update_fields.iter().map(|f| {
        let field = &f.name;
        let field_str = &f.name_str;
        quote! {
            if data.#field.is_some() {
                if !set_clauses.is_empty() { set_clauses.push_str(", "); }
                param_idx += 1;
                set_clauses.push_str(&format!("{} = ${}", #field_str, param_idx));
                has_updates = true;
            }
        }
    });

    let param_collectors = update_fields.iter().map(|f| {
        let field = &f.name;
        quote! {
            if let Some(ref val) = data.#field {
                params.push(val as &(dyn tokio_postgres::types::ToSql + Sync));
            }
        }
    });

    quote! {
        use ::orm::FromRow;

        let client = self.get().await?;
        let mut set_clauses = String::new();
        let mut param_idx = 0usize;
        let mut has_updates = false;

        #(#set_clause_builders)*

        if !has_updates {
            anyhow::bail!(#no_fields_err);
        }

        param_idx += 1;
        let sql = format!("UPDATE {} SET {} WHERE id = ${} RETURNING {}", #full_table, set_clauses, param_idx, #columns);

        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
        #(#param_collectors)*
        params.push(id);

        let row = client.query_one(&sql, &params).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        #name::from_row(&row).map_err(|e| anyhow::anyhow!("Row parse error: {}", e))
    }
}

fn generate_delete(table: &TableDef) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let sql = format!("DELETE FROM {} WHERE id = $1", full_table);
    let err_msg = format!("Failed to delete {}", table.name_snake);
    let not_found_err = format!("{} not found", name);

    quote! {
        let client = self.get().await?;
        let result = client.execute(#sql, &[id]).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        if result == 0 {
            anyhow::bail!(#not_found_err);
        }

        Ok(())
    }
}
