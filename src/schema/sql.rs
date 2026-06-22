use std::fmt::{self, Display, Formatter};

use super::diff::{ColumnOp, SchemaChange};
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
            Self::AddEnumValues { name, values } => write!(f, "{}", add_enum_values(name, values)),
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
            Self::SetType { column, sql_type } => write!(f, "ALTER COLUMN {column} TYPE {sql_type}"),
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

fn add_enum_values(name: &str, values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("ALTER TYPE {name} ADD VALUE IF NOT EXISTS '{value}';"))
        .collect::<Vec<_>>()
        .join("\n")
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
