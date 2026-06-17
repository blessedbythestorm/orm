use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::schema::{DatabaseSchema, SchemaChange, render};

const META_DIR: &str = "meta";

/// The on-disk migration directory: the `NNNN_name.{up,down}.sql` files plus the
/// per-migration schema snapshots under `meta/`. Owns the path so the commands
/// don't have to thread it through every call.
pub struct MigrationStore {
    directory: PathBuf,
}

impl MigrationStore {
    pub fn new(directory: &Path) -> Self {
        Self { directory: directory.to_path_buf() }
    }

    pub fn display(&self) -> std::path::Display<'_> {
        self.directory.display()
    }

    /// Migration identifiers (`NNNN_name`), sorted by version, derived from the
    /// `.up.sql` files present.
    pub fn stems(&self) -> anyhow::Result<Vec<String>> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }

        let mut stems: Vec<String> = std::fs::read_dir(&self.directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter_map(|name| name.strip_suffix(".up.sql").map(str::to_string))
            .collect();
        stems.sort_by_key(|stem| version_of(stem));
        Ok(stems)
    }

    /// The most recent migration identifier, if any.
    pub fn tip(&self) -> anyhow::Result<Option<String>> {
        Ok(self.stems()?.pop())
    }

    pub fn read_up(&self, stem: &str) -> anyhow::Result<String> {
        self.read_sql(stem, "up")
    }

    pub fn read_down(&self, stem: &str) -> anyhow::Result<String> {
        self.read_sql(stem, "down")
    }

    fn read_sql(&self, stem: &str, direction: &str) -> anyhow::Result<String> {
        let file = format!("{stem}.{direction}.sql");
        std::fs::read_to_string(self.directory.join(&file)).with_context(|| format!("reading {file}"))
    }

    /// Loads the snapshot of the most recent migration — the baseline a new
    /// migration is diffed against. Empty if there are no migrations yet.
    pub fn load_tip_snapshot(&self) -> anyhow::Result<DatabaseSchema> {
        let Some(tip) = self.tip()? else {
            return Ok(DatabaseSchema::default());
        };

        let path = self.snapshot_path(version_of(&tip));
        let contents =
            std::fs::read_to_string(&path).with_context(|| format!("reading snapshot {}", path.display()))?;
        serde_json::from_str(&contents).with_context(|| format!("parsing snapshot {}", path.display()))
    }

    /// Writes a migration's up/down SQL and snapshot, returning its stem.
    pub fn write_migration(
        &self,
        name: &str,
        up: &[SchemaChange],
        down: &[SchemaChange],
        snapshot: &DatabaseSchema,
    ) -> anyhow::Result<String> {
        std::fs::create_dir_all(self.directory.join(META_DIR))
            .with_context(|| format!("creating {}", self.directory.display()))?;

        let version = self.next_version()?;
        let stem = format!("{version:04}_{}", slugify(name));
        std::fs::write(self.directory.join(format!("{stem}.up.sql")), render(up))?;
        std::fs::write(self.directory.join(format!("{stem}.down.sql")), render(down))?;
        save_snapshot(&self.snapshot_path(version), snapshot)?;
        Ok(stem)
    }

    /// Removes a migration's up/down SQL and snapshot.
    pub fn remove(&self, stem: &str) -> anyhow::Result<()> {
        remove_if_present(&self.directory.join(format!("{stem}.up.sql")))?;
        remove_if_present(&self.directory.join(format!("{stem}.down.sql")))?;
        remove_if_present(&self.snapshot_path(version_of(stem)))?;
        Ok(())
    }

    fn next_version(&self) -> anyhow::Result<u32> {
        Ok(self.stems()?.iter().map(|stem| version_of(stem)).max().unwrap_or(0) + 1)
    }

    fn snapshot_path(&self, version: u32) -> PathBuf {
        self.directory.join(META_DIR).join(format!("{version:04}.json"))
    }
}

fn version_of(stem: &str) -> u32 {
    stem.split('_').next().and_then(|prefix| prefix.parse().ok()).unwrap_or(0)
}

fn slugify(name: &str) -> String {
    name.chars()
        .map(|character| if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '_' })
        .collect()
}

fn save_snapshot(path: &Path, schema: &DatabaseSchema) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(schema)?)
        .with_context(|| format!("writing snapshot {}", path.display()))?;
    Ok(())
}

fn remove_if_present(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}
