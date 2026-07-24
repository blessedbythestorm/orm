use std::collections::BTreeMap;

use super::model::{
    Column, Constraint, ConstraintKind, DatabaseSchema, EnumType, ForeignKey, Index,
    ReferentialAction, Table, View,
};

/// Compile-time schema entries submitted by the macros. These mirror the owned
/// model types but use `&'static str` so they can live in `inventory` statics.
pub struct EnumItem {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

pub struct TableItem {
    pub schema: &'static str,
    pub name: &'static str,
    pub columns: &'static [ColumnItem],
    pub constraints: &'static [ConstraintItem],
    pub indexes: &'static [IndexItem],
}

/// A table-level constraint from `#[table(unique(...))]` / `#[table(check(...))]`
/// or a field's `#[pg(check("..."))]`.
pub struct ConstraintItem {
    pub name: &'static str,
    pub kind: ConstraintKindItem,
}

pub enum ConstraintKindItem {
    Unique { columns: &'static [&'static str] },
    Check { expression: &'static str },
}

pub struct IndexItem {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    pub unique: bool,
    pub predicate: Option<&'static str>,
}

pub struct ColumnItem {
    pub name: &'static str,
    pub sql_type: &'static str,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
    pub default: Option<&'static str>,
    pub foreign_key: Option<ForeignKeyItem>,
}

pub struct ForeignKeyItem {
    pub schema: &'static str,
    pub table: &'static str,
    pub column: &'static str,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
}

/// A view declared field-by-field: each column names its source `table.column`,
/// and the JOINs are inferred from the tables' foreign keys at assembly time (the
/// macro can't see other tables, but the assembled schema can).
pub struct ViewItem {
    pub schema: &'static str,
    pub name: &'static str,
    pub columns: &'static [ViewColumnItem],
    pub filter: Option<&'static str>,
    pub order_by: Option<&'static str>,
}

pub struct ViewColumnItem {
    /// Output column / struct field name.
    pub alias: &'static str,
    /// Source column's schema, table, and column.
    pub schema: &'static str,
    pub table: &'static str,
    pub column: &'static str,
}

inventory::collect!(EnumItem);
inventory::collect!(TableItem);
inventory::collect!(ViewItem);

/// Drains the registry into the owned schema model defined by the Rust types.
pub fn assemble_desired_schema() -> DatabaseSchema {
    let mut schema = DatabaseSchema::default();

    for item in inventory::iter::<EnumItem> {
        schema.enums.insert(item.name.to_string(), EnumType::from(item));
    }

    for item in inventory::iter::<TableItem> {
        let table = Table::from(item);
        schema.tables.insert(table.qualified_name(), table);
    }

    // Views are resolved last: building a view's SELECT needs the tables' foreign
    // keys (for the JOINs), so they must already be in `schema.tables`.
    for item in inventory::iter::<ViewItem> {
        let view = build_view(item, &schema.tables);
        schema.views.insert(view.qualified_name(), view);
    }

    schema
}

/// Assembles a view's SELECT from its declared columns, inferring the FROM table
/// (the first column's table) and the JOINs (from foreign keys between tables).
fn build_view(item: &ViewItem, tables: &BTreeMap<String, Table>) -> View {
    let base = item.columns.first().unwrap_or_else(|| {
        panic!("view {}.{} declares no columns", item.schema, item.name)
    });

    let select = item
        .columns
        .iter()
        .map(|c| format!("{}.{} AS {}", c.table, c.column, c.alias))
        .collect::<Vec<_>>()
        .join(", ");

    // One JOIN per distinct non-base table, in first-seen order.
    let mut joins = Vec::new();
    let mut seen = vec![(base.schema, base.table)];
    for column in item.columns {
        let key = (column.schema, column.table);
        if !seen.contains(&key) {
            seen.push(key);
            joins.push(resolve_join(item, base, column, tables));
        }
    }

    let mut sql = format!("SELECT {select} FROM {}.{}", base.schema, base.table);
    for join in &joins {
        sql.push(' ');
        sql.push_str(join);
    }
    if let Some(filter) = item.filter {
        sql.push_str(&format!(" WHERE {filter}"));
    }
    if let Some(order_by) = item.order_by {
        sql.push_str(&format!(" ORDER BY {order_by}"));
    }

    View { schema: item.schema.to_string(), name: item.name.to_string(), definition: sql }
}

/// Builds the `JOIN <other> ON …` clause linking `base` to the `joined` column's
/// table via whichever foreign key connects them (base→other or other→base).
fn resolve_join(
    item: &ViewItem,
    base: &ViewColumnItem,
    joined: &ViewColumnItem,
    tables: &BTreeMap<String, Table>,
) -> String {
    let target = format!("{}.{}", joined.schema, joined.table);

    if let Some(base_table) = tables.get(&format!("{}.{}", base.schema, base.table)) {
        for column in &base_table.columns {
            if let Some(fk) = &column.foreign_key {
                if fk.schema == joined.schema && fk.table == joined.table {
                    return format!(
                        "JOIN {target} ON {}.{} = {}.{}",
                        base.table, column.name, joined.table, fk.column
                    );
                }
            }
        }
    }

    if let Some(joined_table) = tables.get(&target) {
        for column in &joined_table.columns {
            if let Some(fk) = &column.foreign_key {
                if fk.schema == base.schema && fk.table == base.table {
                    return format!(
                        "JOIN {target} ON {}.{} = {}.{}",
                        joined.table, column.name, base.table, fk.column
                    );
                }
            }
        }
    }

    panic!(
        "view {}.{}: no foreign key links {}.{} to {}.{} — declare one to join them",
        item.schema, item.name, base.schema, base.table, joined.schema, joined.table
    );
}

impl From<&EnumItem> for EnumType {
    fn from(item: &EnumItem) -> Self {
        EnumType {
            name: item.name.to_string(),
            values: item.values.iter().map(|value| value.to_string()).collect(),
        }
    }
}

impl From<&TableItem> for Table {
    fn from(item: &TableItem) -> Self {
        Table {
            schema: item.schema.to_string(),
            name: item.name.to_string(),
            columns: item.columns.iter().map(Column::from).collect(),
            constraints: item.constraints.iter().map(Constraint::from).collect(),
            indexes: item.indexes.iter().map(Index::from).collect(),
        }
    }
}

impl From<&ColumnItem> for Column {
    fn from(item: &ColumnItem) -> Self {
        Column {
            name: item.name.to_string(),
            sql_type: item.sql_type.to_string(),
            nullable: item.nullable,
            primary_key: item.primary_key,
            unique: item.unique,
            default: item.default.map(str::to_string),
            foreign_key: item.foreign_key.as_ref().map(ForeignKey::from),
        }
    }
}

impl From<&ConstraintItem> for Constraint {
    fn from(item: &ConstraintItem) -> Self {
        let kind = match &item.kind {
            ConstraintKindItem::Unique { columns } => {
                ConstraintKind::Unique { columns: columns.iter().map(|c| c.to_string()).collect() }
            }
            ConstraintKindItem::Check { expression } => {
                ConstraintKind::Check { expression: expression.to_string() }
            }
        };

        Constraint { name: item.name.to_string(), kind }
    }
}

impl From<&IndexItem> for Index {
    fn from(item: &IndexItem) -> Self {
        Index {
            name: item.name.to_string(),
            columns: item.columns.iter().map(|c| c.to_string()).collect(),
            unique: item.unique,
            predicate: item.predicate.map(str::to_string),
        }
    }
}

impl From<&ForeignKeyItem> for ForeignKey {
    fn from(item: &ForeignKeyItem) -> Self {
        ForeignKey {
            schema: item.schema.to_string(),
            table: item.table.to_string(),
            column: item.column.to_string(),
            on_update: item.on_update,
            on_delete: item.on_delete,
        }
    }
}
