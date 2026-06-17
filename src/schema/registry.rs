use super::model::{Column, DatabaseSchema, EnumType, ForeignKey, ReferentialAction, Table};

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

inventory::collect!(EnumItem);
inventory::collect!(TableItem);

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

    schema
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
