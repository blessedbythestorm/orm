use tokio_postgres::types::ToSql;

pub trait FromRow: Sized {
    fn from_row(row: &tokio_postgres::Row) -> Result<Self, tokio_postgres::Error>;
}

#[allow(async_fn_in_trait)]
pub trait QueryExt {
    async fn query_typed<T: FromRow>(
        &self,
        statement: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<T>, tokio_postgres::Error>;

    async fn query_one_typed<T: FromRow>(
        &self,
        statement: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<T, tokio_postgres::Error>;

    async fn query_opt_typed<T: FromRow>(
        &self,
        statement: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<T>, tokio_postgres::Error>;
}

/// `tokio_postgres::Client` and `deadpool_postgres::Client` expose the same
/// `query`/`query_one`/`query_opt` surface, so `QueryExt` is identical for both.
macro_rules! impl_query_ext {
    ($client:ty) => {
        impl QueryExt for $client {
            async fn query_typed<T: FromRow>(
                &self,
                statement: &str,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<Vec<T>, tokio_postgres::Error> {
                let rows = self.query(statement, params).await?;
                rows.iter().map(T::from_row).collect()
            }

            async fn query_one_typed<T: FromRow>(
                &self,
                statement: &str,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<T, tokio_postgres::Error> {
                let row = self.query_one(statement, params).await?;
                T::from_row(&row)
            }

            async fn query_opt_typed<T: FromRow>(
                &self,
                statement: &str,
                params: &[&(dyn ToSql + Sync)],
            ) -> Result<Option<T>, tokio_postgres::Error> {
                let row = self.query_opt(statement, params).await?;
                row.map(|r| T::from_row(&r)).transpose()
            }
        }
    };
}

impl_query_ext!(tokio_postgres::Client);
impl_query_ext!(deadpool_postgres::Client);
