use std::collections::BTreeMap;

use super::model::{
    Column, Constraint, ConstraintKind, DatabaseSchema, EnumType, ForeignKey, Index,
    ReferentialAction, Table, View,
};

/// Compile-time schema entries submitted by the macros. These mirror the owned
/// model types but use `&'static str` so they can live in `inventory` statics.
pub struct EnumItem {
    pub name: &'static str,
    pub rust_name: &'static str,
    pub variants: &'static [EnumVariantItem],
}

pub struct EnumVariantItem {
    pub rust_name: &'static str,
    pub value: &'static str,
}

pub struct TableItem {
    pub schema: &'static str,
    pub name: &'static str,
    pub columns: &'static [ColumnItem],
    pub constraints: &'static [ConstraintItem],
    pub indexes: &'static [IndexItem],
}

/// A table-level constraint from `#[pg(unique(...))]` or
/// `#[pg(validate(...))]`.
pub struct ConstraintItem {
    pub name: &'static str,
    /// Present only for unnamed general CHECK constraints. Their final name is
    /// derived after enum markers have been replaced with PostgreSQL labels.
    pub inferred_name_prefix: Option<&'static str>,
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
    let mut index_owners = BTreeMap::new();

    for item in inventory::iter::<EnumItem> {
        schema.enums.insert(item.name.to_string(), EnumType::from(item));
    }

    let enum_items = inventory::iter::<EnumItem>.into_iter()
        .collect::<Vec<_>>();

    for item in inventory::iter::<TableItem> {
        let mut table = Table::from(item);
        resolve_table_enum_markers(&mut table, item, &enum_items);
        validate_table_object_names(&table);
        for index in &table.indexes {
            let key = (table.schema.clone(), index.name.clone());
            if let Some(owner) = index_owners.insert(key, table.qualified_name()) {
                panic!(
                    "duplicate index name `{}.{}` declared by `{owner}` and `{}`",
                    table.schema,
                    index.name,
                    table.qualified_name(),
                );
            }
        }

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
    let base = item.columns.first()
        .unwrap_or_else(|| {
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
            joins.push(resolve_join(
                item,
                base,
                column,
                tables
            ));
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
            values: item.variants.iter()
                .map(|variant| variant.value.to_string())
                .collect(),
        }
    }
}

fn resolve_table_enum_markers(table: &mut Table, item: &TableItem, enums: &[&EnumItem]) {
    for column in &mut table.columns {
        if let Some(default) = &mut column.default {
            *default = resolve_enum_markers(default, enums);
        }
    }

    for (constraint, declared) in table.constraints.iter_mut()
        .zip(item.constraints)
    {
        if let ConstraintKind::Check { expression } = &mut constraint.kind {
            *expression = resolve_enum_markers(expression, enums);
            if let Some(prefix) = declared.inferred_name_prefix {
                constraint.name = stable_check_name(prefix, expression);
            }
        }
    }

    for index in &mut table.indexes {
        if let Some(predicate) = &mut index.predicate {
            *predicate = resolve_enum_markers(predicate, enums);
        }
    }
}

fn stable_check_name(prefix: &str, predicate: &str) -> String {
    const MAX_IDENTIFIER_BYTES: usize = 63;
    let digest = predicate.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });

    let suffix = format!("_{digest:016x}_check");
    let max_prefix_bytes = MAX_IDENTIFIER_BYTES - suffix.len();
    let mut end = prefix.len()
        .min(max_prefix_bytes);

    while !prefix.is_char_boundary(end) {
        end -= 1;
    }

    let prefix = prefix[..end].trim_end_matches('_');

    format!("{prefix}{suffix}")
}

fn validate_table_object_names(table: &Table) {
    let mut names = BTreeMap::new();
    for (kind, name) in table.constraints.iter()
        .map(|constraint| ("constraint", constraint.name.as_str()))
        .chain(
            table.indexes.iter()
                .map(|index| ("index", index.name.as_str()))
        ) {
        if let Some(previous) = names.insert(name, kind) {
            panic!(
                "{} declares duplicate schema object name `{name}` ({previous} and {kind})",
                table.qualified_name(),
            );
        }
    }
}

fn resolve_enum_markers(expression: &str, enums: &[&EnumItem]) -> String {
    const PREFIX: &str = "'__orm_enum__";
    let mut resolved = expression.to_string();
    while let Some(start) = resolved.find(PREFIX) {
        let value_start = start + PREFIX.len();
        let rest = &resolved[value_start..];
        let end = rest.find('\'')
            .unwrap_or_else(|| {
                panic!("unterminated enum marker in schema expression `{expression}`")
            });

        let marker = &rest[..end];
        let (rust_enum, rust_variant) = marker.rsplit_once("__")
            .unwrap_or_else(|| {
                panic!("malformed enum marker `{marker}` in schema expression `{expression}`")
            });

        let matches = enums
            .iter()
            .filter(|item| item.rust_name == rust_enum)
            .flat_map(|item| {
                item.variants
                    .iter()
                    .filter(move |variant| variant.rust_name == rust_variant)
            })
            .collect::<Vec<_>>();

        let variant = match matches.as_slice() {
            [variant] => *variant,
            [] => panic!("unknown registered enum variant `{rust_enum}::{rust_variant}`"),
            _ => panic!("ambiguous registered enum type `{rust_enum}`; use distinct Rust enum names"),
        };

        let quoted = format!("'{}'", variant.value.replace('\'', "''"));
        resolved.replace_range(start..value_start + end + 1, &quoted);
    }

    resolved
}

impl From<&TableItem> for Table {
    fn from(item: &TableItem) -> Self {
        Table {
            schema: item.schema.to_string(),
            name: item.name.to_string(),
            columns: item.columns.iter()
                .map(Column::from)
                .collect(),
            constraints: item.constraints.iter()
                .map(Constraint::from)
                .collect(),
            indexes: item.indexes.iter()
                .map(Index::from)
                .collect(),
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
            foreign_key: item.foreign_key.as_ref()
                .map(ForeignKey::from),
        }
    }
}

impl From<&ConstraintItem> for Constraint {
    fn from(item: &ConstraintItem) -> Self {
        let kind = match &item.kind {
            ConstraintKindItem::Unique { columns } => {
                ConstraintKind::Unique {
                    columns: columns.iter()
                        .map(|c| c.to_string())
                        .collect(),
                }
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
            columns: item.columns.iter()
                .map(|c| c.to_string())
                .collect(),
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
