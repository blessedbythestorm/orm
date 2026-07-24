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

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub columns: Vec<Column>,
    /// Table-level constraints: multi-column uniques and checks. Single-column
    /// uniques stay on their [`Column`]; everything else lives here.
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// Standalone indexes — including the partial and expression indexes a
    /// constraint cannot express.
    #[serde(default)]
    pub indexes: Vec<Index>,
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

    pub fn constraint(&self, name: &str) -> Option<&Constraint> {
        self.constraints.iter().find(|constraint| constraint.name == name)
    }

    pub fn index(&self, name: &str) -> Option<&Index> {
        self.indexes.iter().find(|index| index.name == name)
    }
}

/// A named table-level constraint. The name is the diff key, so it must be
/// stable: the macros derive it from the table and columns when not given one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    pub name: String,
    pub kind: ConstraintKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintKind {
    Unique { columns: Vec<String> },
    Check { expression: String },
}

impl Constraint {
    /// The body of the `ADD CONSTRAINT <name> …` clause.
    pub fn definition(&self) -> String {
        match &self.kind {
            ConstraintKind::Unique { columns } => format!("UNIQUE ({})", columns.join(", ")),
            ConstraintKind::Check { expression } => format!("CHECK ({expression})"),
        }
    }
}

/// A standalone index. `predicate` makes it partial; `unique` makes it enforce
/// uniqueness over just the rows the predicate selects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub predicate: Option<String>,
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
