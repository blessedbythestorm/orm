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

/// Applies every migration not yet recorded in `_orm_migrations`.
pub fn apply(directory: &Path, database_url: Option<String>) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let url = database::resolve_url(database_url)?;

    block_on(async move {
        let mut db = Database::connect(&url).await?;
        db.ensure_migrations_table().await?;
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
    interactive: bool,
) -> anyhow::Result<()> {
    let store = MigrationStore::new(directory);
    let url = database::resolve_url(database_url)?;

    block_on(async move {
        let desired = assemble_desired_schema();
        let db = Database::connect(&url).await?;
        let current = db.introspect(&owned_schemas(&desired)).await?;

        // Same resolver as `generate`, so a renamed column is offered as a rename
        // (data-preserving) instead of a destructive drop + add.
        let changes = diff(&current, &desired, resolver(interactive).as_mut());
        if changes.is_empty() {
            println!("{}", style::success("No drift — the database matches the Rust schema."));
            return Ok(());
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

/// The distinct schemas the Rust types live in — the set we introspect and diff,
/// so unrelated schemas in the same database are left untouched.
fn owned_schemas(schema: &DatabaseSchema) -> Vec<String> {
    let mut schemas = BTreeSet::new();
    for table in schema.tables.values() {
        schemas.insert(table.schema.clone());
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
