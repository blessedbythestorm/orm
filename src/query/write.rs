use std::sync::Arc;

use anyhow::{Result, bail};
use tokio_postgres::types::ToSql;

use super::builder::is_identifier;

#[derive(Debug)]
struct WriteValue {
    field: String,
    value: Arc<dyn ToSql + Send + Sync>,
}

#[derive(Debug, Default)]
pub struct InsertValues {
    values: Vec<WriteValue>,
}

impl InsertValues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn value<T>(mut self, field: impl Into<String>, value: T) -> Self
    where
        T: ToSql + Send + Sync + 'static,
    {
        self.values.push(WriteValue {
            field: field.into(),
            value: Arc::new(value),
        });
        self
    }

    pub fn build(&self, allowed: &[&str]) -> Result<(String, String)> {
        validate_fields(self.values.iter().map(|value| value.field.as_str()), allowed)?;

        let columns = self.values
            .iter()
            .map(|value| value.field.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=self.values.len())
            .map(|index| format!("${index}"))
            .collect::<Vec<_>>()
            .join(", ");

        Ok((columns, placeholders))
    }

    pub fn params(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.values
            .iter()
            .map(|value| value.value.as_ref() as &(dyn ToSql + Sync))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Debug)]
enum UpdateOperation {
    Assign(Arc<dyn ToSql + Send + Sync>),
    Add(Arc<dyn ToSql + Send + Sync>),
    Subtract(Arc<dyn ToSql + Send + Sync>),
    Null,
    DatabaseNow,
}

#[derive(Debug)]
struct UpdateValue {
    field: String,
    operation: UpdateOperation,
}

#[derive(Debug, Default)]
pub struct UpdateValues {
    values: Vec<UpdateValue>,
}

impl UpdateValues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn assign<T>(mut self, field: impl Into<String>, value: T) -> Self
    where
        T: ToSql + Send + Sync + 'static,
    {
        self.values.push(UpdateValue {
            field: field.into(),
            operation: UpdateOperation::Assign(Arc::new(value)),
        });
        self
    }

    pub fn add<T>(mut self, field: impl Into<String>, value: T) -> Self
    where
        T: ToSql + Send + Sync + 'static,
    {
        self.values.push(UpdateValue {
            field: field.into(),
            operation: UpdateOperation::Add(Arc::new(value)),
        });
        self
    }

    pub fn subtract<T>(mut self, field: impl Into<String>, value: T) -> Self
    where
        T: ToSql + Send + Sync + 'static,
    {
        self.values.push(UpdateValue {
            field: field.into(),
            operation: UpdateOperation::Subtract(Arc::new(value)),
        });
        self
    }

    pub fn null(mut self, field: impl Into<String>) -> Self {
        self.values.push(UpdateValue {
            field: field.into(),
            operation: UpdateOperation::Null,
        });
        self
    }

    pub fn database_now(mut self, field: impl Into<String>) -> Self {
        self.values.push(UpdateValue {
            field: field.into(),
            operation: UpdateOperation::DatabaseNow,
        });
        self
    }

    pub fn build(&self, param_offset: usize, allowed: &[&str]) -> Result<(String, usize)> {
        if self.values.is_empty() {
            bail!("filtered update requires at least one value");
        }

        validate_fields(self.values.iter().map(|value| value.field.as_str()), allowed)?;

        let mut param_index = param_offset;
        let mut assignments = Vec::with_capacity(self.values.len());

        for value in &self.values {
            let assignment = match value.operation {
                UpdateOperation::Assign(_) => {
                    let assignment = format!("{} = ${param_index}", value.field);
                    param_index += 1;
                    assignment
                }
                UpdateOperation::Add(_) => {
                    let assignment = format!("{} = {} + ${param_index}", value.field, value.field);
                    param_index += 1;
                    assignment
                }
                UpdateOperation::Subtract(_) => {
                    let assignment = format!("{} = {} - ${param_index}", value.field, value.field);
                    param_index += 1;
                    assignment
                }
                UpdateOperation::Null => format!("{} = NULL", value.field),
                UpdateOperation::DatabaseNow => format!("{} = now()", value.field),
            };

            assignments.push(assignment);
        }

        Ok((assignments.join(", "), param_index))
    }

    pub fn params(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.values
            .iter()
            .filter_map(|value| match &value.operation {
                UpdateOperation::Assign(value)
                | UpdateOperation::Add(value)
                | UpdateOperation::Subtract(value) => {
                    Some(value.as_ref() as &(dyn ToSql + Sync))
                }
                UpdateOperation::Null | UpdateOperation::DatabaseNow => None,
            })
            .collect()
    }
}

fn validate_fields<'a>(fields: impl Iterator<Item = &'a str>, allowed: &[&str]) -> Result<()> {
    let mut seen = Vec::new();

    for field in fields {
        if !is_identifier(field) || !allowed.contains(&field) {
            bail!("unknown write column `{field}`");
        }

        if seen.contains(&field) {
            bail!("duplicate write column `{field}`");
        }

        seen.push(field);
    }

    Ok(())
}
