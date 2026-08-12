use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::parse::{ConstraintKindSpec, TableDef};

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
    let count_all = format_ident!("count_{}s", table.name_snake);
    let get_one = format_ident!("get_{}", table.name_snake);
    let create = format_ident!("create_{}", table.name_snake);
    let update = format_ident!("update_{}", table.name_snake);
    let delete = format_ident!("delete_{}", table.name_snake);
    let delete_all = format_ident!("delete_{}s", table.name_snake);
    let upserts = unique_keys(table).into_iter().map(|columns| {
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

    let pool_client = quote! { let client = self.get().await?; };
    let object_client = quote! { let client = self; };
    let transaction_client = quote! { let client = self; };
    let pool_get_all_body = generate_get_all(table, &pool_client);
    let pool_count_all_body = generate_count_all(table, &pool_client);
    let pool_get_one_body = generate_get_one(table, &pool_client);
    let pool_create_body = generate_create(table, &pool_client);
    let pool_update_body = generate_update(table, &pool_client);
    let pool_delete_body = generate_delete(table, &pool_client);
    let pool_delete_all_body = generate_delete_all(table, &pool_client);
    let object_get_all_body = generate_get_all(table, &object_client);
    let object_count_all_body = generate_count_all(table, &object_client);
    let object_get_one_body = generate_get_one(table, &object_client);
    let object_create_body = generate_create(table, &object_client);
    let object_update_body = generate_update(table, &object_client);
    let object_delete_body = generate_delete(table, &object_client);
    let object_delete_all_body = generate_delete_all(table, &object_client);
    let transaction_get_all_body = generate_get_all(table, &transaction_client);
    let transaction_count_all_body = generate_count_all(table, &transaction_client);
    let transaction_get_one_body = generate_get_one(table, &transaction_client);
    let transaction_create_body = generate_create(table, &transaction_client);
    let transaction_update_body = generate_update(table, &transaction_client);
    let transaction_delete_body = generate_delete(table, &transaction_client);
    let transaction_delete_all_body = generate_delete_all(table, &transaction_client);
    let pool_upserts = generate_upsert_impls(table, &pool_client);
    let object_upserts = generate_upsert_impls(table, &object_client);
    let transaction_upserts = generate_upsert_impls(table, &transaction_client);

    quote! {
        impl #trait_name for deadpool_postgres::Pool {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                #pool_get_all_body
            }

            async fn #count_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<i64> {
                #pool_count_all_body
            }

            async fn #get_one(&self, id: &uuid::Uuid) -> anyhow::Result<#name> {
                #pool_get_one_body
            }

            async fn #create(&self, data: &#insert_name) -> anyhow::Result<#name> {
                #pool_create_body
            }

            async fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> anyhow::Result<#name> {
                #pool_update_body
            }

            async fn #delete(&self, id: &uuid::Uuid) -> anyhow::Result<()> {
                #pool_delete_body
            }

            async fn #delete_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<u64> {
                #pool_delete_all_body
            }

            #(#pool_upserts)*
        }

        impl #trait_name for deadpool_postgres::Object {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                #object_get_all_body
            }

            async fn #count_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<i64> {
                #object_count_all_body
            }

            async fn #get_one(&self, id: &uuid::Uuid) -> anyhow::Result<#name> {
                #object_get_one_body
            }

            async fn #create(&self, data: &#insert_name) -> anyhow::Result<#name> {
                #object_create_body
            }

            async fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> anyhow::Result<#name> {
                #object_update_body
            }

            async fn #delete(&self, id: &uuid::Uuid) -> anyhow::Result<()> {
                #object_delete_body
            }

            async fn #delete_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<u64> {
                #object_delete_all_body
            }

            #(#object_upserts)*
        }

        impl<'transaction> #trait_name for tokio_postgres::Transaction<'transaction> {
            async fn #get_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<Vec<#name>> {
                #transaction_get_all_body
            }

            async fn #count_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<i64> {
                #transaction_count_all_body
            }

            async fn #get_one(&self, id: &uuid::Uuid) -> anyhow::Result<#name> {
                #transaction_get_one_body
            }

            async fn #create(&self, data: &#insert_name) -> anyhow::Result<#name> {
                #transaction_create_body
            }

            async fn #update(&self, id: &uuid::Uuid, data: &#update_name) -> anyhow::Result<#name> {
                #transaction_update_body
            }

            async fn #delete(&self, id: &uuid::Uuid) -> anyhow::Result<()> {
                #transaction_delete_body
            }

            async fn #delete_all(&self, opts: ::orm::query::QueryOptions) -> anyhow::Result<u64> {
                #transaction_delete_all_body
            }

            #(#transaction_upserts)*
        }
    }
}

fn unique_keys(table: &TableDef) -> Vec<Vec<String>> {
    let mut keys: Vec<Vec<String>> = table
        .fields
        .iter()
        .filter(|field| field.is_primary || field.is_unique)
        .map(|field| vec![field.name_str.clone()])
        .collect();

    keys.extend(table.constraints.iter().filter_map(|constraint| {
        match &constraint.kind {
            ConstraintKindSpec::Unique { columns } => Some(columns.clone()),
            ConstraintKindSpec::Check { .. } => None,
        }
    }));

    keys
}

fn upsert_method(table: &TableDef, columns: &[String]) -> syn::Ident {
    format_ident!(
        "upsert_{}_by_{}",
        table.name_snake,
        columns.join("_and_")
    )
}

fn generate_upsert_impls(table: &TableDef, client_setup: &TokenStream) -> Vec<TokenStream> {
    let name = &table.name;
    let insert_name = format_ident!("{}Insert", name);
    let update_name = format_ident!("{}Update", name);

    unique_keys(table)
        .into_iter()
        .map(|columns| {
            let method = upsert_method(table, &columns);
            let body = generate_upsert(table, &columns, client_setup);
            let selective_method = format_ident!("{}_with", method);
            let selective_body = generate_selective_upsert(table, &columns, client_setup);

            quote! {
                async fn #method(&self, data: &#insert_name) -> anyhow::Result<#name> {
                    #body
                }

                async fn #selective_method(&self, data: &#insert_name, update: &#update_name) -> anyhow::Result<#name> {
                    #selective_body
                }
            }
        })
        .collect()
}

fn generate_selective_upsert(
    table: &TableDef,
    conflict_columns: &[String],
    client_setup: &TokenStream,
) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let conflict_target = conflict_columns.join(", ");
    let no_op_column = &conflict_columns[0];
    let err_msg = format!(
        "Failed to upsert {} by {}",
        table.name_snake,
        conflict_columns.join(", ")
    );

    let insert_collectors = table.insert_fields().map(|field| {
        let name = &field.name;
        let column = &field.name_str;

        if field.is_auto_generated {
            quote! {
                if let Some(ref value) = data.#name {
                    insert_columns.push(#column);
                    params.push(value as &(dyn tokio_postgres::types::ToSql + Sync));
                }
            }
        }
        else {
            quote! {
                insert_columns.push(#column);
                params.push(&data.#name as &(dyn tokio_postgres::types::ToSql + Sync));
            }
        }
    });

    let update_fields: Vec<_> = table
        .update_fields()
        .filter(|field| !conflict_columns.contains(&field.name_str))
        .collect();

    let assignment_builders = update_fields.iter().map(|field| {
        let name = &field.name;
        let column = &field.name_str;

        quote! {
            if let Some(ref value) = update.#name {
                params.push(value as &(dyn tokio_postgres::types::ToSql + Sync));
                assignments.push(format!("{} = ${}", #column, params.len()));
            }
        }
    });

    quote! {
        use ::orm::FromRow;

        #client_setup
        let mut insert_columns: Vec<&str> = Vec::new();
        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();

        #(#insert_collectors)*

        let placeholders = (1..=insert_columns.len())
            .map(|index| format!("${}", index))
            .collect::<Vec<_>>()
            .join(", ");
        let mut assignments = Vec::new();

        #(#assignment_builders)*

        if assignments.is_empty() {
            assignments.push(format!("{} = EXCLUDED.{}", #no_op_column, #no_op_column));
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {} RETURNING {}",
            #full_table,
            insert_columns.join(", "),
            placeholders,
            #conflict_target,
            assignments.join(", "),
            #columns,
        );

        let row = client.query_one(&sql, &params).await
            .map_err(|error| anyhow::anyhow!(concat!(#err_msg, ": {}"), error))?;

        #name::from_row(&row)
            .map_err(|error| anyhow::anyhow!("Row parse error: {}", error))
    }
}

fn generate_upsert(
    table: &TableDef,
    conflict_columns: &[String],
    client_setup: &TokenStream,
) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let conflict_target = conflict_columns.join(", ");
    let err_msg = format!(
        "Failed to upsert {} by {}",
        table.name_snake,
        conflict_columns.join(", ")
    );

    let collectors = table.insert_fields().map(|field| {
        let name = &field.name;
        let column = &field.name_str;

        if field.is_auto_generated {
            quote! {
                if let Some(ref value) = data.#name {
                    insert_columns.push(#column);
                    params.push(value as &(dyn tokio_postgres::types::ToSql + Sync));
                }
            }
        }
        else {
            quote! {
                insert_columns.push(#column);
                params.push(&data.#name as &(dyn tokio_postgres::types::ToSql + Sync));
            }
        }
    });

    let mut update_columns: Vec<&str> = table
        .update_fields()
        .filter(|field| !conflict_columns.contains(&field.name_str))
        .map(|field| field.name_str.as_str())
        .collect();

    if update_columns.is_empty() {
        update_columns.push(&conflict_columns[0]);
    }

    let assignments = update_columns
        .iter()
        .map(|column| format!("{column} = EXCLUDED.{column}"))
        .collect::<Vec<_>>()
        .join(", ");

    quote! {
        use ::orm::FromRow;

        #client_setup
        let mut insert_columns: Vec<&str> = Vec::new();
        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();

        #(#collectors)*

        let placeholders = (1..=insert_columns.len())
            .map(|index| format!("${}", index))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {} RETURNING {}",
            #full_table,
            insert_columns.join(", "),
            placeholders,
            #conflict_target,
            #assignments,
            #columns,
        );

        let row = client.query_one(&sql, &params).await
            .map_err(|error| anyhow::anyhow!(concat!(#err_msg, ": {}"), error))?;

        #name::from_row(&row)
            .map_err(|error| anyhow::anyhow!("Row parse error: {}", error))
    }
}

fn generate_get_all(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let base_sql = format!("SELECT {} FROM {}", columns, full_table);
    let err_msg = format!("Failed to get {}s", table.name_snake);

    quote! {
        use ::orm::FromRow;

        #client_setup
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

fn generate_count_all(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
    let full_table = table.full_table_name();
    let err_msg = format!("Failed to count {}s", table.name_snake);

    quote! {
        #client_setup
        let (where_clause, _) = opts.build_where_clause(1);
        let sql = format!("SELECT COUNT(*) AS count FROM {}{}", #full_table, where_clause);
        let row = client.query_one(&sql, &opts.filter_params()).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;
        row.try_get::<_, i64>("count")
            .map_err(|e| anyhow::anyhow!("Count parse error: {}", e))
    }
}

fn generate_get_one(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let primary_key = table.primary_key_name();
    let sql = format!("SELECT {} FROM {} WHERE {} = $1", columns, full_table, primary_key);
    let err_msg = format!("Failed to get {}", table.name_snake);

    quote! {
        use ::orm::FromRow;

        #client_setup
        let row = client.query_one(#sql, &[id]).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        #name::from_row(&row).map_err(|e| anyhow::anyhow!("Row parse error: {}", e))
    }
}

fn generate_create(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
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

        #client_setup
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

fn generate_update(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let columns = table.column_list();
    let err_msg = format!("Failed to update {}", table.name_snake);
    let no_fields_err = format!("No fields to update for {}", table.name_snake);
    let primary_key = table.primary_key_name();

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

        #client_setup
        let mut set_clauses = String::new();
        let mut param_idx = 0usize;
        let mut has_updates = false;

        #(#set_clause_builders)*

        if !has_updates {
            anyhow::bail!(#no_fields_err);
        }

        param_idx += 1;
        let sql = format!("UPDATE {} SET {} WHERE {} = ${} RETURNING {}", #full_table, set_clauses, #primary_key, param_idx, #columns);

        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
        #(#param_collectors)*
        params.push(id);

        let row = client.query_one(&sql, &params).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        #name::from_row(&row).map_err(|e| anyhow::anyhow!("Row parse error: {}", e))
    }
}

fn generate_delete(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
    let name = &table.name;
    let full_table = table.full_table_name();
    let primary_key = table.primary_key_name();
    let sql = format!("DELETE FROM {} WHERE {} = $1", full_table, primary_key);
    let err_msg = format!("Failed to delete {}", table.name_snake);
    let not_found_err = format!("{} not found", name);

    quote! {
        #client_setup
        let result = client.execute(#sql, &[id]).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))?;

        if result == 0 {
            anyhow::bail!(#not_found_err);
        }

        Ok(())
    }
}

fn generate_delete_all(table: &TableDef, client_setup: &TokenStream) -> TokenStream {
    let full_table = table.full_table_name();
    let err_msg = format!("Failed to delete {}s", table.name_snake);

    quote! {
        #client_setup
        let (where_clause, _) = opts.build_where_clause(1);
        if where_clause.is_empty() {
            anyhow::bail!("bulk delete requires at least one filter");
        }
        let sql = format!("DELETE FROM {}{}", #full_table, where_clause);
        client.execute(&sql, &opts.filter_params()).await
            .map_err(|e| anyhow::anyhow!(concat!(#err_msg, ": {}"), e))
    }
}
