use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The full database schema as defined by the Rust types. This is what gets
/// snapshotted and diffed; ordering is deterministic so snapshots are stable.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSchema {
    pub enums: BTreeMap<String, EnumType>,
    pub tables: BTreeMap<String, Table>,
    #[serde(default)]
    pub views: BTreeMap<String, View>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumType {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub columns: Vec<Column>,
}

impl Table {
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }

    pub fn reference(&self) -> TableReference {
        TableReference { schema: self.schema.clone(), name: self.name.clone() }
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|column| column.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
    pub default: Option<String>,
    pub foreign_key: Option<ForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    pub fn to_sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

/// A lightweight handle to a table, used by schema changes that don't need the
/// whole table definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableReference {
    pub schema: String,
    pub name: String,
}

impl TableReference {
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}

/// A read-only database VIEW defined by a `#[view_type]` projection. `definition`
/// is the raw SELECT body as declared in Rust. The migration engine diffs it as an
/// opaque string (declared-vs-snapshot), so it round-trips without Postgres'
/// view-definition normalization causing spurious diffs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub schema: String,
    pub name: String,
    pub definition: String,
}

impl View {
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }

    pub fn reference(&self) -> ViewReference {
        ViewReference { schema: self.schema.clone(), name: self.name.clone() }
    }
}

/// A lightweight handle to a view, for changes that don't need the definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewReference {
    pub schema: String,
    pub name: String,
}

impl ViewReference {
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}
