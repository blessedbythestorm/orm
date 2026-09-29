use anyhow::Result;
use tokio_postgres::{Row, types::ToSql};

/// The sealed execution surface shared by fluent query builders.
pub trait Executor: Sync {
    fn query_row(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<Row>> + Send;

    fn query_rows(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<Vec<Row>>> + Send;

    fn execute_statement(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64>> + Send;
}

macro_rules! impl_executor {
    ($client:ty) => {
        impl Executor for $client {
            async fn query_row(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
                Ok(self.query_one(sql, params).await?)
            }

            async fn query_rows(
                &self,
                sql: &str,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<Vec<Row>> {
                Ok(self.query(sql, params).await?)
            }

            async fn execute_statement(
                &self,
                sql: &str,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<u64> {
                Ok(self.execute(sql, params).await?)
            }
        }
    };
}

impl_executor!(tokio_postgres::Client);
impl_executor!(tokio_postgres::Transaction<'_>);
impl_executor!(deadpool_postgres::Client);

impl Executor for deadpool_postgres::Pool {
    async fn query_row(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
        Ok(
            self.get()
                .await?
                .query_one(sql, params)
                .await?
        )
    }

    async fn query_rows(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>> {
        Ok(
            self.get()
                .await?
                .query(sql, params)
                .await?
        )
    }

    async fn execute_statement(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64> {
        Ok(
            self.get()
                .await?
                .execute(sql, params)
                .await?
        )
    }
}
