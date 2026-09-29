use std::marker::PhantomData;

use anyhow::{Context, Result, bail};
use tokio_postgres::types::ToSql;

use super::client::Executor;
use super::fluent::{append_returning, query_model, query_models, validate_column};
use super::{InsertValues, TableModel, UpdateValues};

/// A single-row INSERT, optionally followed by an atomic conflict action.
#[must_use = "an insert does nothing until a terminal method is awaited"]
pub struct Insert<'a, C: ?Sized, M> {
    client: &'a C,
    values: InsertValues,
    conflict: Option<(Vec<String>, ConflictAction)>,
    model: PhantomData<fn() -> M>,
}

enum ConflictAction {
    Nothing,
    Excluded(Vec<String>),
    Update(UpdateValues),
}

/// A conflict target waiting for its explicit action.
#[must_use = "choose do_nothing, do_update, or do_update_excluded"]
pub struct OnConflict<'a, C: ?Sized, M> {
    insert: Insert<'a, C, M>,
    columns: Vec<String>,
}

impl<'a, C: Executor + ?Sized, M: TableModel> Insert<'a, C, M> {
    pub(crate) fn new(client: &'a C) -> Self {
        Self {
            client,
            values: InsertValues::new(),
            conflict: None,
            model: PhantomData,
        }
    }

    pub fn value<V: ToSql + Send + Sync + 'static>(mut self, field: impl Into<String>, value: V) -> Self {
        self.values = self.values.value(field, value);
        self
    }

    pub fn values(mut self, values: InsertValues) -> Self {
        self.values = values;
        self
    }

    pub fn on_conflict(self, columns: &[&str]) -> OnConflict<'a, C, M> {
        OnConflict {
            insert: self,
            columns: columns.iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    fn sql(&self, returning: bool) -> Result<String> {
        let mut sql = format!("INSERT INTO {} AS __orm_insert", M::RELATION);
        if self.values.is_empty() {
            sql.push_str(" DEFAULT VALUES");
        } else {
            let (columns, placeholders) = self.values.build(M::COLUMNS)?;
            sql.push_str(&format!(" ({columns}) VALUES ({placeholders})"));
        }

        if let Some((target, action)) = &self.conflict {
            validate_columns::<M>(target)?;
            sql.push_str(&format!(" ON CONFLICT ({})", target.join(", ")));
            match action {
                ConflictAction::Nothing => sql.push_str(" DO NOTHING"),
                ConflictAction::Excluded(columns) => {
                    validate_columns::<M>(columns)?;
                    let assignments = columns.iter()
                        .map(|field| format!("{field} = EXCLUDED.{field}"))
                        .collect::<Vec<_>>()
                        .join(", ");

                    sql.push_str(&format!(" DO UPDATE SET {assignments}"));
                }
                ConflictAction::Update(values) => {
                    let (assignments, _) = values.build_qualified(
                        self.values.params()
                            .len() + 1,
                        M::COLUMNS,
                        Some("__orm_insert")
                    )?;

                    sql.push_str(&format!(" DO UPDATE SET {assignments}"));
                }
            }
        }

        append_returning::<M>(&mut sql, returning);
        Ok(sql)
    }

    fn params(&self) -> Vec<&(dyn ToSql + Sync)> {
        let mut params = self.values.params();
        if let Some((_, ConflictAction::Update(values))) = &self.conflict {
            params.extend(values.params());
        }

        params
    }

    pub async fn execute(self) -> Result<u64> {
        let sql = self.sql(false)?;
        self.client.execute_statement(&sql, &self.params())
            .await
            .with_context(|| format!("Failed to insert into {}", M::RELATION))
    }

    pub async fn returning(self) -> Result<Vec<M>> {
        let sql = self.sql(true)?;
        query_models(self.client, &sql, &self.params())
            .await
    }

    /// Requires one returned row. DO NOTHING with a conflict yields an error.
    pub async fn returning_one(self) -> Result<M> {
        let sql = self.sql(true)?;
        query_model(self.client, &sql, &self.params())
            .await
    }
}

impl<'a, C: Executor + ?Sized, M: TableModel> OnConflict<'a, C, M> {
    pub fn do_nothing(mut self) -> Insert<'a, C, M> {
        self.insert.conflict = Some((self.columns, ConflictAction::Nothing));
        self.insert
    }

    pub fn do_update(mut self, values: UpdateValues) -> Insert<'a, C, M> {
        self.insert.conflict = Some((self.columns, ConflictAction::Update(values)));
        self.insert
    }

    pub fn do_update_excluded(mut self, columns: &[&str]) -> Insert<'a, C, M> {
        self.insert.conflict = Some((
            self.columns,
            ConflictAction::Excluded(
                columns.iter()
                    .map(|s| s.to_string())
                    .collect()
            ),
        ));

        self.insert
    }
}

fn validate_columns<M: TableModel>(columns: &[String]) -> Result<()> {
    if columns.is_empty() {
        bail!("conflict targets and update columns must not be empty");
    }

    let mut seen = Vec::new();
    for column in columns {
        validate_column::<M>(column)?;
        if seen.contains(&column) {
            bail!("duplicate conflict column `{column}`");
        }

        seen.push(column);
    }

    Ok(())
}
