use std::collections::BTreeSet;

use super::model::{
    Column, Constraint, ConstraintKind, DatabaseSchema, EnumType, Index, Table, TableReference, View,
    ViewReference,
};

#[derive(Debug, Clone)]
pub enum SchemaChange {
    CreateSchema(String),
    DropSchema(String),
    CreateEnum(EnumType),
    DropEnum(String),
    ReplaceEnum { old: EnumType, new: EnumType, dependents: Vec<EnumDependent> },
    CreateTable(Table),
    DropTable(TableReference),
    RenameTable { table: TableReference, to: String },
    AlterColumn { table: TableReference, op: ColumnOp },
    AlterTable { table: TableReference, op: TableOp },
    CreateView(View),
    DropView(ViewReference),
    Comment(String),
}

/// A column that stores a replaced enum, carrying what the rewrite needs: the
/// default is dropped and restored around the cast, and nullability picks the
/// fallback for rows holding a value the new enum no longer has.
#[derive(Debug, Clone)]
pub struct EnumDependent {
    pub table: TableReference,
    pub column: String,
    pub nullable: bool,
    pub default: Option<String>,
}

/// A single constraint- or index-level operation, scoped to a table by
/// [`SchemaChange::AlterTable`]. A redefinition is a drop followed by an add,
/// since Postgres cannot alter either in place.
#[derive(Debug, Clone)]
pub enum TableOp {
    AddConstraint(Constraint),
    DropConstraint(String),
    CreateIndex(Index),
    DropIndex { schema: String, name: String },
}

/// A single column-level operation, scoped to a table by [`SchemaChange::AlterColumn`].
#[derive(Debug, Clone)]
pub enum ColumnOp {
    Add(Column),
    Drop(String),
    Rename { from: String, to: String },
    SetType { column: String, sql_type: String, using: Option<String> },
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
    desired_enums: BTreeSet<String>,
}

/// The table-level work of a diff, resolved up front so table drops can be
/// emitted before enum changes while creates, renames, and column diffs come
/// after them.
struct TablePlan<'a> {
    created: Vec<&'a Table>,
    renamed: Vec<(&'a Table, &'a Table)>,
    dropped: Vec<&'a Table>,
}

impl<'a> Differ<'a> {
    fn new(resolver: &'a mut dyn RenameResolver) -> Self {
        Self { resolver, changes: Vec::new(), desired_enums: BTreeSet::new() }
    }

    /// Walks the schemas in dependency order: views drop first (they depend on
    /// tables), then doomed tables (they may hold columns of a changed enum),
    /// then enum creates/replaces (types must exist before the tables and
    /// columns that use them), then table creates/renames/column changes, then
    /// enum drops (a column must move off a type before it can go), then views.
    fn run(mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) -> Vec<SchemaChange> {
        self.desired_enums = desired.enums.keys().cloned().collect();
        self.diff_schemas(baseline, desired);

        let plan = self.plan_tables(baseline, desired);

        self.drop_views(baseline, desired);

        for table in &plan.dropped {
            self.emit(SchemaChange::DropTable(table.reference()));
        }

        self.diff_enums(baseline, desired, &plan);

        for table in order_by_dependencies(plan.created.clone()) {
            self.emit(SchemaChange::CreateTable(table.clone()));
        }

        for (old_table, new_table) in &plan.renamed {
            self.emit(SchemaChange::RenameTable { table: old_table.reference(), to: new_table.name.clone() });
            self.diff_columns(old_table, new_table);
            self.diff_constraints(old_table, new_table);
        }

        for desired_table in desired.tables.values() {
            if let Some(baseline_table) = baseline.tables.get(&desired_table.qualified_name()) {
                self.diff_columns(baseline_table, desired_table);
                self.diff_constraints(baseline_table, desired_table);
            }
        }

        for table in order_by_dependencies(plan.created.clone()) {
            for index in &table.indexes {
                self.emit(SchemaChange::AlterTable {
                    table: table.reference(),
                    op: TableOp::CreateIndex(index.clone()),
                });
            }
        }

        self.drop_enums(baseline, desired);
        self.create_views(baseline, desired);
        self.changes
    }

    fn emit(&mut self, change: SchemaChange) {
        self.changes.push(change);
    }

    fn alter_column(&mut self, table: &TableReference, op: ColumnOp) {
        self.emit(SchemaChange::AlterColumn { table: table.clone(), op });
    }

    fn alter_table(&mut self, table: &TableReference, op: TableOp) {
        self.emit(SchemaChange::AlterTable { table: table.clone(), op });
    }

    fn diff_schemas(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        let existing = schemas_of(baseline);
        for schema in schemas_of(desired) {
            if schema != "public" && !existing.contains(&schema) {
                self.emit(SchemaChange::CreateSchema(schema));
            }
        }
    }

    /// Emits enum creates and replaces. Postgres cannot remove, rename, or
    /// reorder enum values (and `ADD VALUE` can't be used later inside the same
    /// transaction our migrations run in), so ANY value change becomes a
    /// [`SchemaChange::ReplaceEnum`]: recreate the type and re-point every
    /// surviving column that stores it. Columns of tables dropped by this same
    /// migration are already gone by the time the replace runs.
    fn diff_enums(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema, plan: &TablePlan) {
        let doomed: BTreeSet<String> = plan.dropped.iter().map(|table| table.qualified_name()).collect();

        for (name, desired_enum) in &desired.enums {
            let Some(baseline_enum) = baseline.enums.get(name) else {
                self.emit(SchemaChange::CreateEnum(desired_enum.clone()));
                continue;
            };

            if baseline_enum.values == desired_enum.values {
                continue;
            }

            self.emit(SchemaChange::ReplaceEnum {
                old: baseline_enum.clone(),
                new: desired_enum.clone(),
                dependents: enum_dependents(baseline, name, &doomed),
            });
        }
    }

    /// Drops enums that vanished from the desired schema. Runs after all table
    /// changes so every column has already moved off the type.
    fn drop_enums(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        for name in baseline.enums.keys() {
            if !desired.enums.contains_key(name) {
                self.emit(SchemaChange::DropEnum(name.clone()));
            }
        }
    }

    /// Drops views that were removed or whose definition changed. A changed view
    /// is dropped here and recreated in [`create_views`] (CREATE OR REPLACE can't
    /// alter a view's output columns, and a clean drop+recreate also frees any
    /// base column the new definition no longer needs).
    fn drop_views(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        for (name, baseline_view) in &baseline.views {
            let gone_or_changed = match desired.views.get(name) {
                None => true,
                Some(desired_view) => desired_view.definition != baseline_view.definition,
            };
            if gone_or_changed {
                self.emit(SchemaChange::DropView(baseline_view.reference()));
            }
        }
    }

    /// Creates views that are new or whose definition changed (the matching drop
    /// was emitted by [`drop_views`] before any table changes).
    fn create_views(&mut self, baseline: &DatabaseSchema, desired: &DatabaseSchema) {
        for (name, desired_view) in &desired.views {
            let new_or_changed = match baseline.views.get(name) {
                None => true,
                Some(baseline_view) => baseline_view.definition != desired_view.definition,
            };
            if new_or_changed {
                self.emit(SchemaChange::CreateView(desired_view.clone()));
            }
        }
    }

    /// Splits tables into created / renamed / dropped, resolving renames through
    /// the resolver up front so the phases of [`run`] can be emitted in order.
    fn plan_tables<'b>(&mut self, baseline: &'b DatabaseSchema, desired: &'b DatabaseSchema) -> TablePlan<'b> {
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

        TablePlan { created: newly_created, renamed, dropped }
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

    /// Diffs table-level constraints and indexes by name. Postgres cannot alter
    /// either in place, so a changed definition becomes a drop and a re-add.
    fn diff_constraints(&mut self, baseline_table: &Table, desired_table: &Table) {
        let table = desired_table.reference();

        for old in &baseline_table.constraints {
            let survives = desired_table
                .constraint(&old.name)
                .is_some_and(|new| new.kind == old.kind);

            if !survives {
                self.alter_table(&table, TableOp::DropConstraint(old.name.clone()));
            }
        }

        for new in &desired_table.constraints {
            let unchanged = baseline_table
                .constraint(&new.name)
                .is_some_and(|old| old.kind == new.kind);

            if !unchanged {
                self.alter_table(&table, TableOp::AddConstraint(new.clone()));
            }
        }

        for old in &baseline_table.indexes {
            let survives = desired_table
                .index(&old.name)
                .is_some_and(|new| new == old);

            if !survives {
                let op = TableOp::DropIndex {
                    schema: baseline_table.schema.clone(),
                    name: old.name.clone(),
                };
                self.alter_table(&table, op);
            }
        }

        for new in &desired_table.indexes {
            let unchanged = baseline_table
                .index(&new.name)
                .is_some_and(|old| old == new);

            if !unchanged {
                self.alter_table(&table, TableOp::CreateIndex(new.clone()));
            }
        }
    }

    fn diff_column_attributes(&mut self, table: &TableReference, old: &Column, new: &Column) {
        let column = new.name.clone();
        if old.unique != new.unique {
            let name = format!("{}_{column}_key", table.name);
            let op = if new.unique {
                TableOp::AddConstraint(Constraint {
                    name,
                    kind: ConstraintKind::Unique { columns: vec![column.clone()] },
                })
            } else {
                TableOp::DropConstraint(name)
            };

            self.alter_table(table, op);
        }
        if old.sql_type != new.sql_type {
            let using = self
                .desired_enums
                .contains(&new.sql_type)
                .then(|| format!("{}::text::{}", column, new.sql_type));
            self.alter_column(
                table,
                ColumnOp::SetType { column: column.clone(), sql_type: new.sql_type.clone(), using },
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

/// Every surviving column that stores `enum_name`: the columns a
/// [`SchemaChange::ReplaceEnum`] must re-point. Tables in `doomed` are dropped
/// by the same migration before the replace runs, so they are skipped.
fn enum_dependents(
    baseline: &DatabaseSchema,
    enum_name: &str,
    doomed: &BTreeSet<String>,
) -> Vec<EnumDependent> {
    let mut dependents = Vec::new();

    for table in baseline.tables.values() {
        if doomed.contains(&table.qualified_name()) {
            continue;
        }

        for column in &table.columns {
            if column.sql_type == enum_name {
                dependents.push(EnumDependent {
                    table: table.reference(),
                    column: column.name.clone(),
                    nullable: column.nullable,
                    default: column.default.clone(),
                });
            }
        }
    }

    dependents
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
        ReplaceEnum { old, new, dependents } => ReplaceEnum {
            old: new.clone(),
            new: old.clone(),
            dependents: dependents.clone(),
        },
        Comment(text) => Comment(text.clone()),
        CreateTable(table) => DropTable(table.reference()),
        DropTable(table) => recreate_table(baseline, table),
        RenameTable { table, to } => RenameTable {
            table: TableReference { schema: table.schema.clone(), name: to.clone() },
            to: table.name.clone(),
        },
        AlterColumn { table, op } => invert_column(baseline, table, op),
        AlterTable { table, op } => invert_table(baseline, table, op),
        CreateView(view) => DropView(view.reference()),
        DropView(view) => recreate_view(baseline, view),
    }
}

/// Undoes a constraint or index change, reading the dropped definition back out
/// of the baseline so the down migration can recreate it exactly.
fn invert_table(baseline: &DatabaseSchema, table: &TableReference, op: &TableOp) -> SchemaChange {
    let baseline_table = baseline.tables.get(&table.qualified_name());

    let inverted = match op {
        TableOp::AddConstraint(constraint) => TableOp::DropConstraint(constraint.name.clone()),
        TableOp::DropConstraint(name) => {
            let Some(constraint) = baseline_table.and_then(|table| table.constraint(name)) else {
                return SchemaChange::Comment(format!(
                    "cannot recreate constraint {name} on {}: not in baseline",
                    table.qualified_name()
                ));
            };

            TableOp::AddConstraint(constraint.clone())
        }
        TableOp::CreateIndex(index) => {
            TableOp::DropIndex { schema: table.schema.clone(), name: index.name.clone() }
        }
        TableOp::DropIndex { name, .. } => {
            let Some(index) = baseline_table.and_then(|table| table.index(name)) else {
                return SchemaChange::Comment(format!(
                    "cannot recreate index {name} on {}: not in baseline",
                    table.qualified_name()
                ));
            };

            TableOp::CreateIndex(index.clone())
        }
    };

    SchemaChange::AlterTable { table: table.clone(), op: inverted }
}

fn recreate_view(baseline: &DatabaseSchema, view: &ViewReference) -> SchemaChange {
    match baseline.views.get(&view.qualified_name()) {
        Some(view) => SchemaChange::CreateView(view.clone()),
        None => SchemaChange::Comment(format!(
            "cannot recreate view {}: not in baseline",
            view.qualified_name()
        )),
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
            let using = baseline
                .enums
                .contains_key(&existing.sql_type)
                .then(|| format!("{}::text::{}", column, existing.sql_type));
            ColumnOp::SetType { column: column.clone(), sql_type: existing.sql_type.clone(), using }
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
    for view in database.views.values() {
        schemas.insert(view.schema.clone());
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
