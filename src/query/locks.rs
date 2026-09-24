use tokio_postgres::Transaction;

/// Serializes commands sharing a text key for the lifetime of a transaction.
pub async fn advisory_xact_lock_key(transaction: &Transaction<'_>, key: &str) -> anyhow::Result<()> {
    transaction
        .query_one("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", &[&key])
        .await
        .map_err(|error| anyhow::Error::new(error).context("Failed to acquire advisory transaction lock"))?;

    Ok(())
}

/// Serializes commands sharing a fixed 64-bit key for the lifetime of a transaction.
pub async fn advisory_xact_lock_id(transaction: &Transaction<'_>, id: i64) -> anyhow::Result<()> {
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&id])
        .await
        .map_err(|error| anyhow::Error::new(error).context("Failed to acquire advisory transaction lock"))?;

    Ok(())
}
