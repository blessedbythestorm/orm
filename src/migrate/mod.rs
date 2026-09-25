//! Drizzle-style schema migrations driven by the registered Rust types.
//!
//! Each command is a thin orchestration over two owned pieces: [`MigrationStore`]
//! (the `migrations/` directory and its snapshots) and [`Database`] (the live
//! connection plus the `_orm_migrations` bookkeeping).

mod database;
mod prompt;
mod store;

use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;

use anyhow::Context;

use crate::schema::{
    DatabaseSchema, NoRenames, RenameResolver, assemble_desired_schema, diff, invert, render,
};
use crate::style;

use database::Database;
use prompt::{Prompt, ask};
use store::MigrationStore;

/// Appends a migration: diffs the Rust schema against the tip snapshot and
/// writes its up SQL, the inverted down SQL, and a snapshot of the new state.
pub fn generate(directory: &Path, name: &str, interactive: bool) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let desired = assemble_desired_schema();
    let baseline = store.load_tip_snapshot()?;

    let up = diff(&baseline, &desired, resolver(interactive).as_mut());
    if up.is_empty() {
        println!("{}", style::step("Schema is up to date; nothing to generate."));
        return Ok(());
    }

    let down = invert(&up, &baseline);
    let stem = store.write_migration(name, &up, &down, &desired)?;
    println!("{}", style::success(&format!("Generated {} ({} change(s))", style::bold(&stem), up.len())));
    Ok(())
}

/// Appends a migration by introspecting the database selected through an
/// environment variable and diffing that live schema against the Rust models.
/// The variable name is accepted separately so credentials never need to be
/// passed in process arguments.
pub fn generate_from_database(
    directory: &Path,
    name: &str,
    interactive: bool,
    database_url_env: &str,
) -> anyhow::Result<()> {
    if database_url_env.is_empty() {
        anyhow::bail!("database URL environment variable name cannot be empty");
    }

    let url = std::env::var(database_url_env)
        .with_context(|| format!("{database_url_env} is not set"))?;

    let store = MigrationStore::new(directory);
    block_on(async move {
        let db = Database::connect(&url).await?;
        verify_database_history(&store, &db).await
    })?;

    generate(directory, name, interactive)
}

/// Applies every migration not yet recorded in `_orm_migrations`.
pub fn apply(directory: &Path, database_url: Option<String>) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let url = database::resolve_url(database_url)?;

    block_on(async move {
        let mut db = Database::connect(&url).await?;
        db.ensure_migrations_table().await?;
        verify_database_history(&store, &db).await?;
        let applied = db.applied().await?;

        let pending: Vec<String> =
            store.stems()?.into_iter().filter(|stem| !applied.contains(stem)).collect();
        if pending.is_empty() {
            println!("{}", style::step("No pending migrations."));
            return Ok(());
        }

        for stem in &pending {
            db.apply(stem, &store.read_up(stem)?).await?;
            println!("{}", style::success(&format!("Applied {}", style::bold(stem))));
        }
        println!("{}", style::success(&format!("Applied {} migration(s).", pending.len())));
        Ok(())
    })
}

/// Rolls back the most recent migration: if it's applied, runs its down SQL
/// against the database and un-records it; either way, removes its files.
pub fn revert(directory: &Path, database_url: Option<String>, assume_yes: bool) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let Some(tip) = store.tip()? else {
        println!("No migrations to revert.");
        return Ok(());
    };

    let Some(url) = database_url.or_else(|| std::env::var("DATABASE_URL").ok()) else {
        eprintln!("{}", style::warn(&format!("no DATABASE_URL; assuming {tip} is unapplied (down not run)")));
        store.remove(&tip)?;
        println!("{}", style::success(&format!("Removed {}", style::bold(&tip))));
        return Ok(());
    };

    block_on(async move {
        let mut db = Database::connect(&url).await?;
        db.ensure_migrations_table().await?;

        if db.applied().await?.contains(&tip) {
            if !assume_yes && !ask(&format!("Revert applied migration {tip}? Runs its down migration.")) {
                println!("{}", style::warn("Aborted."));
                return Ok(());
            }
            db.revert(&tip, &store.read_down(&tip)?).await?;
            println!("{}", style::success(&format!("Rolled back {} in the database", style::bold(&tip))));
        } else {
            println!("{}", style::step(&format!("{tip} is not applied; removing files only")));
        }

        store.remove(&tip)?;
        println!("{}", style::success(&format!("Removed {}", style::bold(&tip))));
        Ok(())
    })
}

/// Adopts an existing database: introspects it, writes a baseline migration
/// describing the current state, and records it as applied WITHOUT running any
/// SQL — so existing data is never touched. Must be the first migration.
pub fn baseline(directory: &Path, name: &str, database_url: Option<String>) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    if !store.stems()?.is_empty() {
        anyhow::bail!("baseline needs an empty migrations directory; it records the starting point");
    }
    let url = database::resolve_url(database_url)?;

    block_on(async move {
        let desired = assemble_desired_schema();
        let db = Database::connect(&url).await?;
        db.ensure_migrations_table().await?;
        let current = db.introspect(&owned_schemas(&desired)).await?;

        let empty = DatabaseSchema::default();
        let up = diff(&empty, &current, &mut NoRenames);
        let down = invert(&up, &empty);
        let stem = store.write_migration(name, &up, &down, &current)?;
        db.record_applied(&stem).await?;

        println!(
            "{}",
            style::success(&format!(
                "Baselined existing database as {} (recorded as applied; no SQL executed)",
                style::bold(&stem)
            ))
        );
        println!(
            "{}",
            style::step(
                "Next: `migrate generate <name>` to reconcile the existing schema to the Rust types."
            )
        );
        Ok(())
    })
}

/// Introspects the live database and reports how it differs from the Rust
/// schema. Prints the reconciling SQL, or with `write` emits it as a migration.
pub fn diff_live(
    directory: &Path,
    database_url: Option<String>,
    write: Option<String>,
    check: bool,
    interactive: bool,
) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let url = database::resolve_url(database_url)?;

    block_on(async move {
        let desired = assemble_desired_schema();
        let db = Database::connect(&url).await?;
        verify_database_history(&store, &db).await?;
        let mut current = db.introspect(&owned_schemas(&desired)).await?;
        adopt_matching_expressions(&db, &mut current, &desired).await?;

        // Same resolver as `generate`, so a renamed column is offered as a rename
        // (data-preserving) instead of a destructive drop + add.
        let changes = diff(&current, &desired, resolver(interactive).as_mut());
        if changes.is_empty() {
            println!("{}", style::success("No drift — the database matches the Rust schema."));
            return Ok(());
        }

        if check && write.is_none() {
            anyhow::bail!("database schema differs from the Rust models");
        }

        match write {
            None => {
                println!(
                    "{}",
                    style::warn(&format!(
                        "Drift detected ({} change(s)). SQL to reconcile the database:",
                        changes.len()
                    ))
                );
                println!("\n{}", render(&changes));
            }
            Some(name) => {
                let down = invert(&changes, &current);
                let stem = store.write_migration(&name, &changes, &down, &desired)?;
                println!(
                    "{}",
                    style::success(&format!(
                        "Wrote {} ({} change(s)). Review it, then `migrate apply`.",
                        style::bold(&stem),
                        changes.len()
                    ))
                );
            }
        }
        Ok(())
    })
}

/// Verifies that the live schema still matches the snapshot of its latest
/// recorded migration. Pending local migrations are allowed; gaps, unknown
/// applied migrations, and out-of-band schema changes are not.
async fn verify_database_history(store: &MigrationStore, db: &Database) -> anyhow::Result<()> {
    db.ensure_migrations_table().await?;

    let stems = store.stems()?;
    let applied = db.applied().await?;
    let unknown: Vec<&String> = applied.iter().filter(|stem| !stems.contains(stem)).collect();
    if !unknown.is_empty() {
        anyhow::bail!(
            "database contains migration(s) absent from the repository: {}",
            unknown.iter().map(|stem| stem.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    let applied_count = stems.iter().take_while(|stem| applied.contains(*stem)).count();
    if stems.iter().skip(applied_count).any(|stem| applied.contains(stem)) {
        anyhow::bail!("database migration history contains a gap; applied migrations must be a prefix");
    }

    let expected = match applied_count.checked_sub(1) {
        Some(index) => store.load_snapshot(&stems[index])?,
        None => DatabaseSchema::default(),
    };
    let mut current = db.introspect(&owned_schemas(&expected)).await?;
    adopt_matching_expressions(db, &mut current, &expected).await?;
    let drift = diff(&current, &expected, &mut NoRenames);
    if !drift.is_empty() {
        anyhow::bail!(
            "database drifted from its latest applied migration ({} change(s)); reconcile it before generating a new migration\n{}",
            drift.len(),
            render(&drift)
        );
    }

    let pending = stems.len().saturating_sub(applied_count);
    println!(
        "{}",
        style::success(&format!(
            "Verified database migration history ({} applied, {} pending).",
            applied_count, pending
        ))
    );
    Ok(())
}

/// Lists migrations and, when a database is reachable, which are applied vs pending.
pub fn status(directory: &Path, database_url: Option<String>) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let stems = store.stems()?;
    if stems.is_empty() {
        println!("{}", style::step(&format!("No migrations in {}", store.display())));
        return Ok(());
    }

    let applied = match database_url.or_else(|| std::env::var("DATABASE_URL").ok()) {
        Some(url) => match block_on(async move {
            let db = Database::connect(&url).await?;
            db.ensure_migrations_table().await?;
            db.applied().await
        }) {
            Ok(applied) => Some(applied),
            Err(error) => {
                eprintln!("{}", style::warn(&format!("couldn't read applied migrations: {error}")));
                None
            }
        },
        None => None,
    };

    println!("{}", style::bold(&format!("Migrations in {}:", store.display())));
    for stem in &stems {
        match &applied {
            Some(applied) if applied.contains(stem) => println!("  {}", style::success(stem)),
            Some(_) => println!("  {}", style::warn(&format!("{stem} (pending)"))),
            None => println!("  {stem}"),
        }
    }
    if applied.is_none() {
        println!("{}", style::step("Set DATABASE_URL (or pass --database-url) to show applied/pending."));
    }
    Ok(())
}

/// The resolver `generate`/`diff` use to turn ambiguous drop+add pairs into
/// data-preserving renames: interactive prompts, or always-no when scripted.
fn resolver(interactive: bool) -> Box<dyn RenameResolver> {
    if interactive { Box::new(Prompt) } else { Box::new(NoRenames) }
}

/// Adopts declared expression spelling only after comparing PostgreSQL's
/// catalog-normalized representation with the live definition.
async fn adopt_matching_expressions(
    db: &Database,
    current: &mut DatabaseSchema,
    desired: &DatabaseSchema,
) -> anyhow::Result<()> {
    use crate::schema::{ConstraintKind, Table};

    for (name, current_table) in current.tables.iter_mut() {
        let Some(desired_table): Option<&Table> = desired.tables.get(name) else {
            continue;
        };

        for current_column in current_table.columns.iter_mut() {
            let Some(desired_column) = desired_table.column(&current_column.name) else {
                continue;
            };

            if defaults_equivalent(
                current_column.default.as_deref(),
                desired_column.default.as_deref(),
            ) {
                current_column.default = desired_column.default.clone();
            }
        }

        for desired_constraint in desired_table.constraints.iter() {
            let ConstraintKind::Unique { columns } = &desired_constraint.kind else {
                continue;
            };
            let [column] = columns.as_slice() else {
                continue;
            };
            let constraint_missing = current_table
                .constraint(&desired_constraint.name)
                .is_none();
            let Some(current_column) = current_table.columns.iter_mut().find(|item| item.name == *column) else {
                continue;
            };

            if current_column.unique
                && constraint_missing
            {
                current_column.unique = false;
                current_table.constraints.push(desired_constraint.clone());
            }
        }

        for constraint in current_table.constraints.iter_mut() {
            let ConstraintKind::Check { .. } = constraint.kind else {
                continue;
            };

            let Some(declared) = desired_table.constraint(&constraint.name) else {
                continue;
            };

            let ConstraintKind::Check { expression: current_expression } = &constraint.kind else {
                continue;
            };
            let ConstraintKind::Check { expression: declared_expression } = &declared.kind else {
                continue;
            };
            let declared_canonical = db
                .canonical_check(&current_table.schema, &current_table.name, declared_expression)
                .await
                .with_context(|| format!("verifying CHECK {} on {name}", constraint.name))?;

            if *current_expression == declared_canonical {
                constraint.kind = declared.kind.clone();
            }
        }

        for index in current_table.indexes.iter_mut() {
            let Some(declared) = desired_table.index(&index.name) else {
                continue;
            };

            let (Some(current_predicate), Some(declared_predicate)) =
                (&index.predicate, &declared.predicate)
            else {
                continue;
            };

            if index.columns != declared.columns || index.unique != declared.unique {
                continue;
            }

            let declared_canonical = db
                .canonical_index_predicate(
                    &current_table.schema,
                    &current_table.name,
                    &declared.columns,
                    declared_predicate,
                )
                .await
                .with_context(|| format!("verifying partial index {} on {name}", index.name))?;

            if *current_predicate == declared_canonical {
                index.predicate = declared.predicate.clone();
            }
        }
    }

    for (name, current_view) in current.views.iter_mut() {
        if let Some(declared) = desired.views.get(name) {
            let declared_canonical = db
                .canonical_view(&declared.definition)
                .await
                .with_context(|| format!("verifying view {name}"))?;

            if current_view.definition == declared_canonical {
                current_view.definition = declared.definition.clone();
            }
        }
    }

    Ok(())
}

fn defaults_equivalent(current: Option<&str>, desired: Option<&str>) -> bool {
    match (current, desired) {
        (None, None) => true,
        (Some(current), Some(desired)) => normalize_default(current) == normalize_default(desired),
        _ => false,
    }
}

fn normalize_default(value: &str) -> String {
    let without_casts = crate::schema::strip_default_casts(value);
    let value = without_casts.trim();

    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        let inner = &value[1..value.len() - 1];
        if matches!(inner, "true" | "false") || inner.parse::<f64>().is_ok() {
            return inner.to_owned();
        }
    }

    value.to_owned()
}

#[cfg(test)]
mod live_diff_tests {
    use super::{Database, adopt_matching_expressions, defaults_equivalent};
    use crate::schema::{DatabaseSchema, View};

    #[test]
    fn postgres_default_spellings_compare_equally() {
        assert!(defaults_equivalent(Some("false"), Some("'false'")));
        assert!(defaults_equivalent(Some("0"), Some("'0'")));
        assert!(defaults_equivalent(Some("'[]'"), Some("'[]'::jsonb")));
        assert!(defaults_equivalent(Some("'{}'"), Some("'{}'::jsonb")));
    }

    #[tokio::test]
    async fn view_adoption_requires_the_same_catalog_definition() {
        let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
            eprintln!("skipping: set ORM_TEST_DATABASE_URL to run the view drift test");
            return;
        };
        let database = Database::connect(&url).await.expect("connect");
        database
            .execute_test_sql(
                "DROP SCHEMA IF EXISTS orm_view_plan_test CASCADE;
                 CREATE SCHEMA orm_view_plan_test;
                 CREATE TABLE orm_view_plan_test.items (id uuid PRIMARY KEY, active boolean NOT NULL);
                 CREATE VIEW orm_view_plan_test.active_items AS SELECT id FROM orm_view_plan_test.items WHERE active = false;",
            )
            .await
            .expect("create view plan fixture");
        let declared = View {
            schema: "orm_view_plan_test".to_string(),
            name: "active_items".to_string(),
            definition: "SELECT id FROM orm_view_plan_test.items WHERE active = true".to_string(),
        };
        let expected = DatabaseSchema {
            views: [(declared.qualified_name(), declared.clone())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut current = database
            .introspect(&["orm_view_plan_test".to_string()])
            .await
            .expect("introspect changed view");
        let changed = current.views.get("orm_view_plan_test.active_items").cloned();

        adopt_matching_expressions(&database, &mut current, &expected)
            .await
            .expect("compare changed view");
        assert_eq!(current.views.get("orm_view_plan_test.active_items"), changed.as_ref());

        database
            .execute_test_sql(
                "CREATE OR REPLACE VIEW orm_view_plan_test.active_items AS SELECT items.id FROM orm_view_plan_test.items WHERE (items.active = true);",
            )
            .await
            .expect("replace view");
        let mut current = database
            .introspect(&["orm_view_plan_test".to_string()])
            .await
            .expect("introspect equivalent view");
        adopt_matching_expressions(&database, &mut current, &expected)
            .await
            .expect("compare equivalent view");
        assert_eq!(current.views.get("orm_view_plan_test.active_items"), Some(&declared));

        database
            .execute_test_sql("DROP SCHEMA orm_view_plan_test CASCADE;")
            .await
            .expect("drop view plan fixture");
    }
}

/// The distinct schemas the Rust types live in — the set we introspect and diff,
/// so unrelated schemas in the same database are left untouched.
fn owned_schemas(schema: &DatabaseSchema) -> Vec<String> {
    let mut schemas = BTreeSet::new();
    for table in schema.tables.values() {
        schemas.insert(table.schema.clone());
    }
    for view in schema.views.values() {
        schemas.insert(view.schema.clone());
    }
    for enum_type in schema.enums.values() {
        if let Some((name, _)) = enum_type.name.split_once('.') {
            schemas.insert(name.to_string());
        }
    }
    schemas.into_iter().collect()
}

fn block_on<T>(future: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(future)
}
