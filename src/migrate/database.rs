use std::collections::HashSet;

use anyhow::Context;
use tokio_postgres::{Client, NoTls};

use crate::schema::{DatabaseSchema, introspect};

const MIGRATIONS_TABLE: &str = "_orm_migrations";

/// A live connection to the target database plus the `_orm_migrations`
/// bookkeeping the commands need. The background connection task is aborted
/// when the `Database` is dropped, so callers never manage it directly.
pub struct Database {
    client: Client,
    connection: tokio::task::JoinHandle<()>,
}

impl Database {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let (client, connection) =
            tokio_postgres::connect(url, NoTls).await.context("connecting to database")?;
        let connection = tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::error!("postgres connection error: {error}");
            }
        });

        let database = Self { client, connection };
        // Quiet server NOTICEs (e.g. "relation already exists" from CREATE IF NOT EXISTS).
        let _ = database.client.batch_execute("SET client_min_messages = warning").await;
        Ok(database)
    }

    pub async fn ensure_migrations_table(&self) -> anyhow::Result<()> {
        self.client
            .batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (\
                    name text PRIMARY KEY, \
                    applied_at timestamptz NOT NULL DEFAULT now())"
            ))
            .await
            .context("ensuring migrations table")?;
        Ok(())
    }

    /// The set of migration stems recorded as applied.
    pub async fn applied(&self) -> anyhow::Result<HashSet<String>> {
        let rows = self.client.query(&format!("SELECT name FROM {MIGRATIONS_TABLE}"), &[]).await?;
        Ok(rows.iter().map(|row| row.get::<_, String>("name")).collect())
    }

    pub async fn introspect(&self, schemas: &[String]) -> anyhow::Result<DatabaseSchema> {
        introspect(&self.client, schemas).await
    }

    /// Records a migration as applied without running its SQL (used by baseline).
    pub async fn record_applied(&self, stem: &str) -> anyhow::Result<()> {
        self.client.execute(&format!("INSERT INTO {MIGRATIONS_TABLE} (name) VALUES ($1)"), &[&stem]).await?;
        Ok(())
    }

    /// Runs a migration's up SQL and records it as applied, in one transaction.
    pub async fn apply(&mut self, stem: &str, sql: &str) -> anyhow::Result<()> {
        let transaction = self.client.transaction().await?;
        transaction.batch_execute(sql).await.with_context(|| format!("applying {stem}"))?;
        transaction.execute(&format!("INSERT INTO {MIGRATIONS_TABLE} (name) VALUES ($1)"), &[&stem]).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Runs a migration's down SQL and un-records it, in one transaction.
    pub async fn revert(&mut self, stem: &str, sql: &str) -> anyhow::Result<()> {
        let transaction = self.client.transaction().await?;
        transaction.batch_execute(sql).await.with_context(|| format!("reverting {stem}"))?;
        transaction.execute(&format!("DELETE FROM {MIGRATIONS_TABLE} WHERE name = $1"), &[&stem]).await?;
        transaction.commit().await?;
        Ok(())
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        self.connection.abort();
    }
}

/// Resolves the database URL from an explicit value or the `DATABASE_URL` env var.
pub fn resolve_url(explicit: Option<String>) -> anyhow::Result<String> {
    explicit
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .context("no database url (set DATABASE_URL or pass --database-url)")
}
