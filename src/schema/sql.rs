use std::fmt::{self, Display, Formatter};

use super::diff::{ColumnOp, EnumDependent, SchemaChange};
use super::model::{Column, EnumType, ForeignKey, Table};

/// Renders an ordered list of schema changes into a single SQL migration script.
pub fn render(changes: &[SchemaChange]) -> String {
    let mut script = changes.iter().map(SchemaChange::to_string).collect::<Vec<_>>().join("\n\n");
    script.push('\n');
    script
}

impl Display for SchemaChange {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateSchema(name) => write!(f, "CREATE SCHEMA IF NOT EXISTS {name};"),
            Self::DropSchema(name) => write!(f, "DROP SCHEMA IF EXISTS {name};"),
            Self::CreateEnum(enum_type) => write!(f, "{}", create_type(enum_type)),
            Self::DropEnum(name) => write!(f, "DROP TYPE IF EXISTS {name};"),
            Self::ReplaceEnum { old, new, dependents } => {
                write!(f, "{}", replace_enum(old, new, dependents))
            }
            Self::CreateTable(table) => write!(f, "{}", create_table(table)),
            Self::DropTable(table) => write!(f, "DROP TABLE {};", table.qualified_name()),
            Self::RenameTable { table, to } => {
                write!(f, "ALTER TABLE {} RENAME TO {to};", table.qualified_name())
            }
            Self::AlterColumn { table, op } => write!(f, "ALTER TABLE {} {op};", table.qualified_name()),
            Self::CreateView(view) => {
                write!(f, "CREATE VIEW {} AS {};", view.qualified_name(), view.definition)
            }
            Self::DropView(view) => write!(f, "DROP VIEW IF EXISTS {};", view.qualified_name()),
            Self::Comment(text) => write!(f, "-- {text}"),
        }
    }
}

impl Display for ColumnOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add(column) => write!(f, "ADD COLUMN {}", column_definition(column)),
            Self::Drop(column) => write!(f, "DROP COLUMN {column}"),
            Self::Rename { from, to } => write!(f, "RENAME COLUMN {from} TO {to}"),
            Self::SetType { column, sql_type, using: Some(expression) } => {
                write!(f, "ALTER COLUMN {column} TYPE {sql_type} USING {expression}")
            }
            Self::SetType { column, sql_type, using: None } => {
                write!(f, "ALTER COLUMN {column} TYPE {sql_type}")
            }
            Self::SetNullable { column, nullable } => {
                let action = if *nullable { "DROP NOT NULL" } else { "SET NOT NULL" };
                write!(f, "ALTER COLUMN {column} {action}")
            }
            Self::SetDefault { column, default: Some(value) } => {
                write!(f, "ALTER COLUMN {column} SET DEFAULT {value}")
            }
            Self::SetDefault { column, default: None } => write!(f, "ALTER COLUMN {column} DROP DEFAULT"),
        }
    }
}

fn create_type(enum_type: &EnumType) -> String {
    let values: Vec<String> = enum_type.values.iter().map(|value| format!("'{value}'")).collect();
    format!("CREATE TYPE {} AS ENUM ({});", enum_type.name, values.join(", "))
}

/// Recreates a changed enum in place — Postgres cannot remove, rename, or
/// reorder enum values, so: rename the old type aside, create the new one under
/// the original name, re-point every dependent column with a text cast (rows
/// holding a value the new enum dropped fall back per [`fallback_value`], and a
/// column default is dropped and restored around the cast), then drop the old
/// type.
fn replace_enum(old: &EnumType, new: &EnumType, dependents: &[EnumDependent]) -> String {
    let renamed = match new.name.rsplit_once('.') {
        Some((schema, bare)) => format!("{schema}.{bare}_old"),
        None => format!("{}_old", new.name),
    };
    let bare_renamed = match new.name.rsplit_once('.') {
        Some((_, bare)) => format!("{bare}_old"),
        None => format!("{}_old", new.name),
    };

    let mut statements = vec![
        format!("ALTER TYPE {} RENAME TO {bare_renamed};", new.name),
        create_type(new),
    ];

    for dependent in dependents {
        let table = dependent.table.qualified_name();
        let column = &dependent.column;

        if dependent.default.is_some() {
            statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT;"));
        }

        statements.push(format!(
            "ALTER TABLE {table} ALTER COLUMN {column} TYPE {} USING {};",
            new.name,
            cast_expression(old, new, dependent),
        ));

        if let Some(default) = &dependent.default {
            statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT {default};"));
        }
    }

    statements.push(format!("DROP TYPE {renamed};"));
    statements.join("\n")
}

/// The USING expression that re-points a column at the recreated enum. When
/// every old value survives it is a plain text cast; otherwise surviving values
/// cast through and removed values collapse to the fallback.
fn cast_expression(old: &EnumType, new: &EnumType, dependent: &EnumDependent) -> String {
    let column = &dependent.column;
    let kept: Vec<String> = old
        .values
        .iter()
        .filter(|value| new.values.contains(value))
        .map(|value| format!("'{value}'"))
        .collect();

    if kept.len() == old.values.len() {
        return format!("{column}::text::{}", new.name);
    }

    if kept.is_empty() {
        return format!("{}::{}", fallback_value(new, dependent), new.name);
    }

    format!(
        "(CASE WHEN {column}::text IN ({}) THEN {column}::text ELSE {} END)::{}",
        kept.join(", "),
        fallback_value(new, dependent),
        new.name,
    )
}

/// The value rows fall back to when they hold an enum value the new type no
/// longer has: the column's default when it is still a valid value, else NULL
/// when the column is nullable, else the first value of the new enum.
fn fallback_value(new: &EnumType, dependent: &EnumDependent) -> String {
    let default = dependent.default.as_deref().and_then(default_literal);

    if let Some(value) = default {
        if new.values.iter().any(|candidate| candidate == value) {
            return format!("'{value}'");
        }
    }

    if dependent.nullable {
        return "NULL".to_string();
    }

    match new.values.first() {
        Some(value) => format!("'{value}'"),
        None => "NULL".to_string(),
    }
}

/// Extracts the quoted literal from a column default like `'open'` or
/// `'open'::core.order_status`.
fn default_literal(default: &str) -> Option<&str> {
    let start = default.find('\'')? + 1;
    let end = default[start..].find('\'')? + start;
    Some(&default[start..end])
}

fn create_table(table: &Table) -> String {
    let mut entries: Vec<String> = table.columns.iter().map(column_definition).collect();

    let primary_key: Vec<&str> =
        table.columns.iter().filter(|column| column.primary_key).map(|column| column.name.as_str()).collect();
    if !primary_key.is_empty() {
        entries.push(format!("PRIMARY KEY ({})", primary_key.join(", ")));
    }

    format!("CREATE TABLE {} (\n    {}\n);", table.qualified_name(), entries.join(",\n    "))
}

/// A column's definition for `CREATE TABLE` / `ADD COLUMN`, including an inline
/// foreign-key reference when present.
fn column_definition(column: &Column) -> String {
    let mut parts = vec![column.name.clone(), column.sql_type.clone()];
    if !column.nullable {
        parts.push("NOT NULL".to_string());
    }
    if let Some(default) = &column.default {
        parts.push(format!("DEFAULT {default}"));
    }
    if column.unique {
        parts.push("UNIQUE".to_string());
    }
    if let Some(foreign_key) = &column.foreign_key {
        parts.push(references_clause(foreign_key));
    }
    parts.join(" ")
}

fn references_clause(foreign_key: &ForeignKey) -> String {
    format!(
        "REFERENCES {}.{} ({}) ON UPDATE {} ON DELETE {}",
        foreign_key.schema,
        foreign_key.table,
        foreign_key.column,
        foreign_key.on_update.to_sql(),
        foreign_key.on_delete.to_sql(),
    )
}
