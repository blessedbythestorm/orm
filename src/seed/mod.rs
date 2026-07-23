//! Seed data: plain `NNNN_name.sql` scripts in a `seeds/` directory, applied
//! once each and recorded in `_orm_seeds` — the same run-once model as
//! migrations. Scripts should still write idempotent inserts (`ON CONFLICT DO
//! NOTHING`) so re-running against a re-baselined database stays safe.

use std::path::Path;

use anyhow::Context;
use tokio_postgres::NoTls;

const SEEDS_TABLE: &str = "_orm_seeds";

const TEMPLATE: &str = "-- Seed: {name}\n\
    -- Applied once and recorded in _orm_seeds. Keep the inserts idempotent\n\
    -- anyway (ON CONFLICT DO NOTHING) so re-running is always safe.\n\n";

/// Writes an empty numbered seed script and returns its path.
pub fn generate(directory: &Path, name: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("creating {}", directory.display()))?;

    let version = next_version(directory)?;
    let stem = format!("{version:04}_{}", slugify(name));
    let path = directory.join(format!("{stem}.sql"));

    std::fs::write(&path, TEMPLATE.replace("{name}", name))?;
    println!("✓ Generated {}", path.display());
    Ok(())
}

/// Applies every pending seed in version order, each in its own transaction,
/// recording it in `_orm_seeds` so it never runs twice.
pub fn apply(directory: &Path, database_url: Option<String>) -> anyhow::Result<()> {
    run(async {
        let url = resolve_url(database_url)?;
        let (client, connection) =
            tokio_postgres::connect(&url, NoTls).await.context("connecting to database")?;
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });

        let _ = client.batch_execute("SET client_min_messages = warning").await;
        client
            .batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS {SEEDS_TABLE} (\
                 name text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())"
            ))
            .await?;

        let applied: Vec<String> = client
            .query(&format!("SELECT name FROM {SEEDS_TABLE}"), &[])
            .await?
            .iter()
            .map(|row| row.get(0))
            .collect();

        let mut ran = 0usize;
        for stem in stems(directory)? {
            if applied.contains(&stem) {
                continue;
            }

            let sql = std::fs::read_to_string(directory.join(format!("{stem}.sql")))?;
            client.batch_execute("BEGIN").await?;
            client
                .batch_execute(&sql)
                .await
                .with_context(|| format!("applying seed {stem}"))?;
            client
                .execute(&format!("INSERT INTO {SEEDS_TABLE} (name) VALUES ($1)"), &[&stem])
                .await?;
            client.batch_execute("COMMIT").await?;

            println!("✓ Seeded {stem}");
            ran += 1;
        }

        if ran == 0 {
            println!("✓ No pending seeds.");
        }

        connection.abort();
        Ok(())
    })
}

/// Lists seeds and whether each has been applied.
pub fn status(directory: &Path, database_url: Option<String>) -> anyhow::Result<()> {
    run(async {
        let url = resolve_url(database_url)?;
        let (client, connection) =
            tokio_postgres::connect(&url, NoTls).await.context("connecting to database")?;
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });

        let applied: Vec<String> = client
            .query(&format!("SELECT name FROM {SEEDS_TABLE}"), &[])
            .await
            .map(|rows| rows.iter().map(|row| row.get(0)).collect())
            .unwrap_or_default();

        println!("Seeds in {}:", directory.display());
        for stem in stems(directory)? {
            let mark = if applied.contains(&stem) { "✓" } else { "!" };
            println!("  {mark} {stem}");
        }

        connection.abort();
        Ok(())
    })
}

fn run<F: std::future::Future<Output = anyhow::Result<()>>>(future: F) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

fn resolve_url(explicit: Option<String>) -> anyhow::Result<String> {
    explicit
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .context("no database url (set DATABASE_URL or pass --database-url)")
}

fn stems(directory: &Path) -> anyhow::Result<Vec<String>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut stems: Vec<String> = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".sql").map(str::to_string))
        .collect();
    stems.sort_by_key(|stem| version_of(stem));
    Ok(stems)
}

fn next_version(directory: &Path) -> anyhow::Result<u32> {
    Ok(stems(directory)?.iter().map(|stem| version_of(stem)).max().unwrap_or(0) + 1)
}

fn version_of(stem: &str) -> u32 {
    stem.split('_').next().and_then(|prefix| prefix.parse().ok()).unwrap_or(0)
}

fn slugify(name: &str) -> String {
    name.chars()
        .map(|character| if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '_' })
        .collect()
}
