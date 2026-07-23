//! Language-neutral validation model and the validator backend trait. Mirrors
//! [`crate::export`] for `#[api(validate(...))]` rules; each target library
//! (valibot, zod, ...) is a [`ValidatorBackend`] under [`crate::lang`].

use std::path::Path;

use tracing::info;

pub enum BaseType {
    Bool,
    Number,
    String,
    Timestamp,
    Uuid,
    /// A reference to another exported type by name; enums render as their
    /// closed value set so schemas and exported unions can't drift.
    Named(&'static str),
    Unknown,
}

pub enum Rule {
    Email,
    Url,
    MinLength(u64),
    MaxLength(u64),
    ExactLength(u64),
    Min(i64),
    Max(i64),
    Regex(&'static str),
}

pub struct Field {
    pub name: &'static str,
    pub base: BaseType,
    pub rules: &'static [Rule],
    pub optional: bool,
    pub array: bool,
}

/// A validator schema for an `#[api_type]`, registered via `inventory`.
pub struct ValidatorSchema {
    pub name: &'static str,
    pub fields: &'static [Field],
}

inventory::collect!(ValidatorSchema);

/// Renders the neutral model into a target validation library.
pub trait ValidatorBackend {
    /// Output filename (e.g. `schemas.ts`).
    fn file_name(&self) -> &str;

    /// File header, including the library import.
    fn header(&self) -> String;

    /// A single `export const NameSchema = ...;` declaration.
    fn schema(&self, schema: &ValidatorSchema) -> String;
}

/// Write every registered schema into `<out_dir>` using `backend`.
pub fn export_validators(out_dir: &str, backend: &dyn ValidatorBackend) -> anyhow::Result<()> {
    let mut schemas: Vec<&ValidatorSchema> = inventory::iter::<ValidatorSchema>.into_iter().collect();
    if schemas.is_empty() {
        return Ok(());
    }

    schemas.sort_by_key(|s| s.name);

    let mut out = backend.header();
    for schema in &schemas {
        out.push_str(&backend.schema(schema));
        out.push('\n');
    }

    let file = Path::new(out_dir).join(backend.file_name());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, out)?;

    info!("Exported {} validator schema(s)", schemas.len());
    Ok(())
}
