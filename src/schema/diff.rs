use std::collections::BTreeSet;

use super::model::{Column, DatabaseSchema, EnumType, Table, TableReference};

#[derive(Debug, Clone)]
pub enum SchemaChange {
    CreateSchema(String),
    DropSchema(String),
    CreateEnum(EnumType),
    DropEnum(String),
    AddEnumValues { name: String, values: Vec<String> },
    CreateTable(Table),
    DropTable(TableReference),
    RenameTable { table: TableReference, to: String },
    AlterColumn { table: TableReference, op: ColumnOp },
    Comment(String),
}

/// A single column-level operation, scoped to a table by [`SchemaChange::AlterColumn`].
#[derive(Debug, Clone)]
pub enum ColumnOp {
    Add(Column),
    Drop(String),
    Rename { from: String, to: String },
    SetType { column: String, sql_type: String },
    SetNullable { column: String, nullable: bool },
    SetDefault { column: String, default: Option<String> },
}

/// Resolves the inherently-ambiguous "is this a rename or a drop + add?"
/// question. A pure schema diff can't know, so the decision comes from here.
pub trait RenameResolver {
    fn confirm_table_rename(&mut self, schema: &str, from: &str, to: &str) -> bool;
    fn confirm_column_rename(&mut self, table: &TableReference, from: &str, to: &str) -> bool;
}

/// Treats every removed/added pair as a drop + add. Used for non-interactive runs.
pub struct NoRenames;

impl RenameResolver for NoRenames {
    fn confirm_table_rename(&mut self, _schema: &str, _from: &str, _to: &str) -> bool {
        false
    }

    fn confirm_column_rename(&mut self, _table: &TableReference, _from: &str, _to: &str) -> bool {
        false
    }
}

/// Computes the changes that turn `baseline` into `desired`.
pub fn diff(
    baseline: &DatabaseSchema,
    desired: &DatabaseSchema,
    resolver: &mut dyn RenameResolver,
) -> Vec<SchemaChange> {
    Differ::new(resolver).run(baseline, desired)
}

/// Accumulates schema changes while walking the two schemas, so the traversal
/// methods don't have to thread a change list and resolver through every call.
struct Differ<'a> {
    resolver: &'a mut dyn RenameResolver,
    changes: Vec<SchemaChange>,
}

impl<'a> Differ<'a> {
    fn new(resolver: &'a mut dyn RenameResolver) -> Self {
        Self { resolver, changes: Vec::new() }
    }

    fn run(mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) -> Vec<SchemaChange> {
        self.diff_schemas(baseline, desired);
        self.diff_enums(baseline, desired);
        self.diff_tables(baseline, desired);
        self.changes
    }

    fn emit(&mut self, change: SchemaChange) {
        self.changes.push(change);
    }

    fn alter_column(&mut self, table: &TableReference, op: ColumnOp) {
        self.emit(SchemaChange::AlterColumn { table: table.clone(), op });
    }

    fn diff_schemas(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        let existing = schemas_of(baseline);
        for schema in schemas_of(desired) {
            if schema != "public" && !existing.contains(&schema) {
                self.emit(SchemaChange::CreateSchema(schema));
            }
        }
    }

    fn diff_enums(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        for (name, desired_enum) in &desired.enums {
            let Some(baseline_enum) = baseline.enums.get(name) else {
                self.emit(SchemaChange::CreateEnum(desired_enum.clone()));
                continue;
            };

            let added: Vec<String> = desired_enum
                .values
                .iter()
                .filter(|value| !baseline_enum.values.contains(value))
                .cloned()
                .collect();
            if !added.is_empty() {
                self.emit(SchemaChange::AddEnumValues { name: name.clone(), values: added });
            }
        }
    }

    fn diff_tables(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        let created = tables_missing_from(desired, baseline);
        let mut dropped = tables_missing_from(baseline, desired);

        let mut renamed: Vec<(&Table, &Table)> = Vec::new();
        let mut newly_created: Vec<&Table> = Vec::new();
        for new_table in created {
            match self.find_table_rename(new_table, &dropped) {
                Some(index) => renamed.push((dropped.remove(index), new_table)),
                None => newly_created.push(new_table),
            }
        }

        for table in order_by_dependencies(newly_created) {
            self.emit(SchemaChange::CreateTable(table.clone()));
        }

        for (old_table, new_table) in renamed {
            self.emit(SchemaChange::RenameTable { table: old_table.reference(), to: new_table.name.clone() });
            self.diff_columns(old_table, new_table);
        }

        for desired_table in desired.tables.values() {
            if let Some(baseline_table) = baseline.tables.get(&desired_table.qualified_name()) {
                self.diff_columns(baseline_table, desired_table);
            }
        }

        for table in dropped {
            self.emit(SchemaChange::DropTable(table.reference()));
        }
    }

    fn diff_columns(&mut self, baseline_table: &Table, desired_table: &Table) {
        let table = desired_table.reference();
        let mut removed: Vec<&Column> = columns_missing_from(baseline_table, desired_table);

        for new_column in columns_missing_from(desired_table, baseline_table) {
            match self.find_column_rename(new_column, &removed, &table) {
                Some(index) => {
                    let old_column = removed.remove(index);
                    let rename =
                        ColumnOp::Rename { from: old_column.name.clone(), to: new_column.name.clone() };
                    self.alter_column(&table, rename);
                    self.diff_column_attributes(&table, old_column, new_column);
                }
                None => self.alter_column(&table, ColumnOp::Add(new_column.clone())),
            }
        }

        for old_column in removed {
            self.alter_column(&table, ColumnOp::Drop(old_column.name.clone()));
        }

        for new_column in &desired_table.columns {
            if let Some(old_column) = baseline_table.column(&new_column.name) {
                self.diff_column_attributes(&table, old_column, new_column);
            }
        }
    }

    fn diff_column_attributes(&mut self, table: &TableReference, old: &Column, new: &Column) {
        let column = new.name.clone();
        if old.sql_type != new.sql_type {
            self.alter_column(
                table,
                ColumnOp::SetType { column: column.clone(), sql_type: new.sql_type.clone() },
            );
        }
        if old.nullable != new.nullable {
            self.alter_column(
                table,
                ColumnOp::SetNullable { column: column.clone(), nullable: new.nullable },
            );
        }
        if old.default != new.default {
            self.alter_column(table, ColumnOp::SetDefault { column, default: new.default.clone() });
        }
    }

    fn find_table_rename(&mut self, new_table: &Table, candidates: &[&Table]) -> Option<usize> {
        candidates.iter().position(|old| {
            old.schema == new_table.schema
                && self.resolver.confirm_table_rename(&new_table.schema, &old.name, &new_table.name)
        })
    }

    fn find_column_rename(
        &mut self,
        new_column: &Column,
        candidates: &[&Column],
        table: &TableReference,
    ) -> Option<usize> {
        candidates.iter().position(|old| {
            old.sql_type == new_column.sql_type
                && self.resolver.confirm_column_rename(table, &old.name, &new_column.name)
        })
    }
}

/// Tables present in `a` but not in `b`, keyed by qualified name.
fn tables_missing_from<'a>(a: &'a DatabaseSchema, b: &DatabaseSchema) -> Vec<&'a Table> {
    a.tables.values().filter(|table| !b.tables.contains_key(&table.qualified_name())).collect()
}

/// Columns present in `a` but not in `b`, keyed by name.
fn columns_missing_from<'a>(a: &'a Table, b: &Table) -> Vec<&'a Column> {
    a.columns.iter().filter(|column| b.column(&column.name).is_none()).collect()
}

/// Produces the changes that undo `changes`, reading old definitions from the
/// schema as it was *before* those changes (the migration's baseline). The list
/// is reversed so dependent objects are torn down before what they depend on.
pub fn invert(changes: &[SchemaChange], baseline: &DatabaseSchema) -> Vec<SchemaChange> {
    let mut inverted: Vec<SchemaChange> = changes.iter().map(|change| invert_one(change, baseline)).collect();
    inverted.reverse();
    inverted
}

fn invert_one(change: &SchemaChange, baseline: &DatabaseSchema) -> SchemaChange {
    use SchemaChange::*;
    match change {
        CreateSchema(name) => DropSchema(name.clone()),
        DropSchema(name) => CreateSchema(name.clone()),
        CreateEnum(enum_type) => DropEnum(enum_type.name.clone()),
        DropEnum(name) => recreate_enum(baseline, name),
        AddEnumValues { name, values } => Comment(format!(
            "cannot drop value(s) {} from enum {name}; reverting needs a type recreate",
            values.join(", ")
        )),
        Comment(text) => Comment(text.clone()),
        CreateTable(table) => DropTable(table.reference()),
        DropTable(table) => recreate_table(baseline, table),
        RenameTable { table, to } => RenameTable {
            table: TableReference { schema: table.schema.clone(), name: to.clone() },
            to: table.name.clone(),
        },
        AlterColumn { table, op } => invert_column(baseline, table, op),
    }
}

fn recreate_enum(baseline: &DatabaseSchema, name: &str) -> SchemaChange {
    match baseline.enums.get(name) {
        Some(enum_type) => SchemaChange::CreateEnum(enum_type.clone()),
        None => SchemaChange::Comment(format!("cannot recreate enum {name}: not in baseline")),
    }
}

fn recreate_table(baseline: &DatabaseSchema, table: &TableReference) -> SchemaChange {
    match baseline.tables.get(&table.qualified_name()) {
        Some(table) => SchemaChange::CreateTable(table.clone()),
        None => SchemaChange::Comment(format!(
            "cannot recreate table {}: not in baseline",
            table.qualified_name()
        )),
    }
}

fn invert_column(baseline: &DatabaseSchema, table: &TableReference, op: &ColumnOp) -> SchemaChange {
    let alter = |op| SchemaChange::AlterColumn { table: table.clone(), op };
    match op {
        ColumnOp::Add(column) => alter(ColumnOp::Drop(column.name.clone())),
        ColumnOp::Rename { from, to } => alter(ColumnOp::Rename { from: to.clone(), to: from.clone() }),
        ColumnOp::Drop(column) => match baseline_column(baseline, table, column) {
            Some(existing) => alter(ColumnOp::Add(existing.clone())),
            None => cannot_revert(table, column, "restore"),
        },
        ColumnOp::SetType { column, .. } => revert_attribute(baseline, table, column, "type", |existing| {
            ColumnOp::SetType { column: column.clone(), sql_type: existing.sql_type.clone() }
        }),
        ColumnOp::SetNullable { column, .. } => {
            revert_attribute(baseline, table, column, "nullability", |existing| ColumnOp::SetNullable {
                column: column.clone(),
                nullable: existing.nullable,
            })
        }
        ColumnOp::SetDefault { column, .. } => {
            revert_attribute(baseline, table, column, "default", |existing| ColumnOp::SetDefault {
                column: column.clone(),
                default: existing.default.clone(),
            })
        }
    }
}

/// Reverts a column attribute to its baseline value, or a comment if the column
/// is gone from the baseline.
fn revert_attribute(
    baseline: &DatabaseSchema,
    table: &TableReference,
    column: &str,
    attribute: &str,
    build: impl Fn(&Column) -> ColumnOp,
) -> SchemaChange {
    match baseline_column(baseline, table, column) {
        Some(existing) => SchemaChange::AlterColumn { table: table.clone(), op: build(existing) },
        None => cannot_revert(table, column, attribute),
    }
}

fn cannot_revert(table: &TableReference, column: &str, what: &str) -> SchemaChange {
    SchemaChange::Comment(format!("cannot revert {what} of {}.{column}", table.qualified_name()))
}

fn baseline_column<'a>(
    baseline: &'a DatabaseSchema,
    table: &TableReference,
    column: &str,
) -> Option<&'a Column> {
    baseline.tables.get(&table.qualified_name())?.column(column)
}

fn schemas_of(database: &DatabaseSchema) -> BTreeSet<String> {
    let mut schemas = BTreeSet::new();
    for table in database.tables.values() {
        schemas.insert(table.schema.clone());
    }
    for enum_type in database.enums.values() {
        if let Some((schema, _)) = enum_type.name.split_once('.') {
            schemas.insert(schema.to_string());
        }
    }
    schemas
}

/// Orders newly-created tables so a table referenced by a foreign key is created
/// before the table that references it. Self-references and references to
/// already-existing tables impose no ordering; cyclic groups keep their original
/// order (inline foreign keys can't express a cycle anyway).
fn order_by_dependencies(tables: Vec<&Table>) -> Vec<&Table> {
    let creating: BTreeSet<String> = tables.iter().map(|table| table.qualified_name()).collect();

    let mut ordered = Vec::new();
    let mut placed = BTreeSet::new();
    let mut remaining = tables;

    while !remaining.is_empty() {
        let progressed_before = placed.len();
        let mut deferred = Vec::new();

        for table in remaining {
            if dependencies_satisfied(table, &creating, &placed) {
                placed.insert(table.qualified_name());
                ordered.push(table);
            } else {
                deferred.push(table);
            }
        }

        if placed.len() == progressed_before {
            ordered.extend(deferred);
            break;
        }
        remaining = deferred;
    }

    ordered
}

fn dependencies_satisfied(table: &Table, creating: &BTreeSet<String>, placed: &BTreeSet<String>) -> bool {
    table.columns.iter().all(|column| match &column.foreign_key {
        Some(foreign_key) => {
            let referenced = format!("{}.{}", foreign_key.schema, foreign_key.table);
            referenced == table.qualified_name()
                || !creating.contains(&referenced)
                || placed.contains(&referenced)
        }
        None => true,
    })
}
