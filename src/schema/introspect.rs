use tokio_postgres::Client;

use super::model::{Column, DatabaseSchema, EnumType, Table};

/// Reads the live schema of the given schemas from the catalog into the same
/// model (and same type spelling / default form) the macros produce, so an
/// introspected schema can be diffed against — or snapshotted like — a
/// Rust-defined one.
pub async fn introspect(client: &Client, schemas: &[String]) -> anyhow::Result<DatabaseSchema> {
    let names: Vec<&str> = schemas.iter().map(String::as_str).collect();
    let mut database = DatabaseSchema::default();
    load_enums(client, &names, &mut database).await?;
    load_columns(client, &names, &mut database).await?;
    Ok(database)
}

async fn load_enums(client: &Client, schemas: &[&str], database: &mut DatabaseSchema) -> anyhow::Result<()> {
    let rows = client
        .query(
            "SELECT n.nspname AS schema_name, t.typname AS type_name, e.enumlabel AS label
             FROM pg_type t
             JOIN pg_namespace n ON n.oid = t.typnamespace
             JOIN pg_enum e ON e.enumtypid = t.oid
             WHERE t.typtype = 'e' AND n.nspname = ANY($1)
             ORDER BY n.nspname, t.typname, e.enumsortorder",
            &[&schemas],
        )
        .await?;

    for row in rows {
        let name = format!("{}.{}", row.get::<_, String>("schema_name"), row.get::<_, String>("type_name"));
        database
            .enums
            .entry(name.clone())
            .or_insert_with(|| EnumType { name, values: Vec::new() })
            .values
            .push(row.get("label"));
    }
    Ok(())
}

async fn load_columns(
    client: &Client,
    schemas: &[&str],
    database: &mut DatabaseSchema,
) -> anyhow::Result<()> {
    let rows = client
        .query(
            "SELECT c.table_schema AS schema_name, c.table_name AS table_name, c.column_name AS column_name,
                    c.is_nullable AS is_nullable, c.column_default AS column_default,
                    c.udt_schema AS udt_schema, c.udt_name AS udt_name
             FROM information_schema.columns c
             JOIN information_schema.tables t
               ON t.table_schema = c.table_schema AND t.table_name = c.table_name
             WHERE t.table_type = 'BASE TABLE'
               AND c.table_schema = ANY($1)
               AND c.table_name <> '_orm_migrations'
             ORDER BY c.table_schema, c.table_name, c.ordinal_position",
            &[&schemas],
        )
        .await?;

    for row in rows {
        let schema: String = row.get("schema_name");
        let table_name: String = row.get("table_name");
        let udt_schema: String = row.get("udt_schema");
        let udt_name: String = row.get("udt_name");

        let sql_type = if udt_schema == "pg_catalog" {
            rust_type_name(&udt_name)
        } else {
            format!("{udt_schema}.{udt_name}")
        };

        let default = row
            .get::<_, Option<String>>("column_default")
            .map(|value| strip_casts(&value).trim().to_string());

        let qualified = format!("{schema}.{table_name}");
        database
            .tables
            .entry(qualified)
            .or_insert_with(|| Table { schema, name: table_name, columns: Vec::new() })
            .columns
            .push(Column {
                name: row.get("column_name"),
                sql_type,
                nullable: row.get::<_, String>("is_nullable") == "YES",
                primary_key: false,
                unique: false,
                default,
                foreign_key: None,
            });
    }
    Ok(())
}

/// Maps a built-in Postgres type (catalog `udt_name`) to the spelling the
/// `SqlType` macros emit, so the two compare equal.
fn rust_type_name(udt_name: &str) -> String {
    match udt_name {
        "bool" => "boolean",
        "float4" => "real",
        "float8" => "double precision",
        other => other,
    }
    .to_string()
}

/// Removes Postgres `::type` cast suffixes (`'web'::text` → `'web'`,
/// `nextval('s'::regclass)` → `nextval('s')`) so defaults compare cleanly.
fn strip_casts(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character == ':' && chars.peek() == Some(&':') {
            chars.next();
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || matches!(next, '_' | '.' | '"' | ' ') {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}
