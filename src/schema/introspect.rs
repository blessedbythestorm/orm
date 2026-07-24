use tokio_postgres::Client;

use super::model::{
    Column, Constraint, ConstraintKind, DatabaseSchema, EnumType, ForeignKey, Index,
    ReferentialAction, Table,
};

/// Reads the live schema of the given schemas from the catalog into the same
/// model (and same type spelling / default form) the macros produce, so an
/// introspected schema can be diffed against — or snapshotted like — a
/// Rust-defined one.
pub async fn introspect(client: &Client, schemas: &[String]) -> anyhow::Result<DatabaseSchema> {
    let names: Vec<&str> = schemas.iter().map(String::as_str).collect();
    let mut database = DatabaseSchema::default();
    load_enums(client, &names, &mut database).await?;
    load_columns(client, &names, &mut database).await?;
    load_constraints(client, &names, &mut database).await?;
    load_indexes(client, &names, &mut database).await?;
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
            .or_insert_with(|| Table { schema, name: table_name, ..Default::default() })
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

/// Reads primary keys, uniques, foreign keys, and checks out of `pg_constraint`.
/// Single-column keys land on their [`Column`] — matching what the macros emit
/// from `#[pg(primary)]` / `#[pg(unique)]` / `#[pg(foreign(...))]` — so the two
/// sides compare equal; multi-column uniques and checks become table-level
/// [`Constraint`]s keyed by their catalog name.
async fn load_constraints(
    client: &Client,
    schemas: &[&str],
    database: &mut DatabaseSchema,
) -> anyhow::Result<()> {
    let rows = client
        .query(
            "SELECT n.nspname AS schema_name, c.relname AS table_name,
                    con.conname AS constraint_name, con.contype::text AS constraint_type,
                    ARRAY(
                        SELECT a.attname
                        FROM unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
                        JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
                        ORDER BY k.ord
                    ) AS columns,
                    fn.nspname AS foreign_schema, fc.relname AS foreign_table,
                    ARRAY(
                        SELECT a.attname
                        FROM unnest(con.confkey) WITH ORDINALITY AS k(attnum, ord)
                        JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum
                        ORDER BY k.ord
                    ) AS foreign_columns,
                    con.confupdtype::text AS on_update, con.confdeltype::text AS on_delete,
                    pg_get_expr(con.conbin, con.conrelid) AS check_expression
             FROM pg_constraint con
             JOIN pg_class c ON c.oid = con.conrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_class fc ON fc.oid = con.confrelid
             LEFT JOIN pg_namespace fn ON fn.oid = fc.relnamespace
             WHERE n.nspname = ANY($1) AND con.contype IN ('p', 'u', 'f', 'c')
             ORDER BY n.nspname, c.relname, con.conname",
            &[&schemas],
        )
        .await?;

    for row in rows {
        let qualified =
            format!("{}.{}", row.get::<_, String>("schema_name"), row.get::<_, String>("table_name"));
        let Some(table) = database.tables.get_mut(&qualified) else {
            continue;
        };

        let name: String = row.get("constraint_name");
        let kind: String = row.get("constraint_type");
        let columns: Vec<String> = row.get("columns");

        match kind.as_str() {
            "p" => {
                for column in &columns {
                    if let Some(target) = table.columns.iter_mut().find(|c| &c.name == column) {
                        target.primary_key = true;
                    }
                }
            }
            "u" => {
                let inline = columns.len() == 1 && name == format!("{}_{}_key", table.name, columns[0]);

                if inline {
                    if let Some(target) = table.columns.iter_mut().find(|c| c.name == columns[0]) {
                        target.unique = true;
                    }

                    continue;
                }

                table.constraints.push(Constraint {
                    name,
                    kind: ConstraintKind::Unique { columns },
                });
            }
            "f" => {
                let foreign_columns: Vec<String> = row.get("foreign_columns");

                if columns.len() != 1 || foreign_columns.len() != 1 {
                    continue;
                }

                let Some(target) = table.columns.iter_mut().find(|c| c.name == columns[0]) else {
                    continue;
                };

                target.foreign_key = Some(ForeignKey {
                    schema: row.get("foreign_schema"),
                    table: row.get("foreign_table"),
                    column: foreign_columns[0].clone(),
                    on_update: referential_action(&row.get::<_, String>("on_update")),
                    on_delete: referential_action(&row.get::<_, String>("on_delete")),
                });
            }
            _ => {
                let Some(expression) = row.get::<_, Option<String>>("check_expression") else {
                    continue;
                };

                table.constraints.push(Constraint {
                    name,
                    kind: ConstraintKind::Check { expression },
                });
            }
        }
    }

    Ok(())
}

/// Reads standalone indexes — those not backing a constraint, which
/// [`load_constraints`] has already captured.
async fn load_indexes(
    client: &Client,
    schemas: &[&str],
    database: &mut DatabaseSchema,
) -> anyhow::Result<()> {
    let rows = client
        .query(
            "SELECT n.nspname AS schema_name, c.relname AS table_name,
                    ic.relname AS index_name, i.indisunique AS is_unique,
                    ARRAY(
                        SELECT a.attname
                        FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord)
                        JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
                        ORDER BY k.ord
                    ) AS columns,
                    pg_get_expr(i.indpred, i.indrelid) AS predicate
             FROM pg_index i
             JOIN pg_class c ON c.oid = i.indrelid
             JOIN pg_class ic ON ic.oid = i.indexrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = ANY($1)
               AND NOT i.indisprimary
               AND NOT EXISTS (SELECT 1 FROM pg_constraint con WHERE con.conindid = i.indexrelid)
             ORDER BY n.nspname, c.relname, ic.relname",
            &[&schemas],
        )
        .await?;

    for row in rows {
        let qualified =
            format!("{}.{}", row.get::<_, String>("schema_name"), row.get::<_, String>("table_name"));
        let Some(table) = database.tables.get_mut(&qualified) else {
            continue;
        };

        let columns: Vec<String> = row.get("columns");

        if columns.is_empty() {
            continue;
        }

        table.indexes.push(Index {
            name: row.get("index_name"),
            columns,
            unique: row.get("is_unique"),
            predicate: row.get("predicate"),
        });
    }

    Ok(())
}

/// The catalog's one-letter referential action codes.
fn referential_action(code: &str) -> ReferentialAction {
    match code {
        "r" => ReferentialAction::Restrict,
        "c" => ReferentialAction::Cascade,
        "n" => ReferentialAction::SetNull,
        "d" => ReferentialAction::SetDefault,
        _ => ReferentialAction::NoAction,
    }
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
