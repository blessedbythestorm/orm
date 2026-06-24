//! Generic orm CLI entrypoint: `generate --lang <lang> [--out <dir>]` and the
//! `migrate` subcommands.
//!
//! Each consumer workspace adds a tiny `[[bin]]` target that calls
//! `orm::cli::main(default_out)`. The bin must transitively link every crate
//! that defines `#[table_type]` / `#[enum_type]` / `#[api_type]` / `#[json_type]`
//! types, because `inventory` only sees what the binary links against.
//!
//! ```sh
//! cargo run --bin mntr-cli -- generate --lang ts            # uses default_out
//! cargo run --bin mntr-cli -- generate --lang ts --out ./x  # override
//! ```

use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "orm", about = "orm ORM CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate all client bindings (types, validators, client) for a language.
    Generate {
        /// Target language (currently: `ts`).
        #[arg(long)]
        lang: String,
        /// Output directory. Defaults to the value the caller passed to `main`.
        #[arg(long, short)]
        out: Option<String>,
    },
    /// Generate and inspect schema migrations from the registered types.
    #[command(subcommand)]
    Migrate(MigrateCommand),
}

impl Command {
    fn run(self, default_out: &str) -> anyhow::Result<()> {
        match self {
            Command::Generate { lang, out } => generate(&lang, &out.unwrap_or_else(|| default_out.to_string())),
            Command::Migrate(command) => command.run(),
        }
    }
}

fn generate(lang: &str, out: &str) -> anyhow::Result<()> {
    match lang {
        "ts" | "typescript" => {
            crate::export::export_all_types(out, &crate::lang::ts::TypeScript)?;
            crate::validator::export_validators(out, &crate::lang::ts::Valibot)?;
            crate::lang::ts::generate_client(out, ".")?;
        }
        other => anyhow::bail!("unsupported language: {other} (supported: ts)"),
    }

    eprintln!("Generated {lang} bindings → {out}");
    Ok(())
}

#[derive(Subcommand)]
enum MigrateCommand {
    /// Diff the Rust schema against the snapshot and write a new migration.
    Generate {
        /// Name for the migration file.
        name: String,
        /// Directory holding migrations and the schema snapshot.
        #[arg(long, default_value = "migrations")]
        dir: String,
        /// Don't prompt for renames; treat every change as drop + add.
        #[arg(long)]
        no_input: bool,
    },
    /// Apply pending migrations to the target database.
    Apply {
        #[arg(long, default_value = "migrations")]
        dir: String,
        /// Defaults to the DATABASE_URL environment variable.
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Roll back the most recent migration (runs its down if applied) and remove it.
    Revert {
        #[arg(long, default_value = "migrations")]
        dir: String,
        #[arg(long)]
        database_url: Option<String>,
        /// Don't prompt before reverting an already-applied migration.
        #[arg(long)]
        yes: bool,
    },
    /// Adopt an existing database: record its current state as an already-applied
    /// baseline migration without running any SQL.
    Baseline {
        name: String,
        #[arg(long, default_value = "migrations")]
        dir: String,
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Show how the live database differs from the Rust schema; with --write,
    /// emit the reconcile as a new migration instead of printing it.
    Diff {
        #[arg(long, default_value = "migrations")]
        dir: String,
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        write: Option<String>,
        /// Don't prompt for renames; treat them as drop + add.
        #[arg(long)]
        no_input: bool,
    },
    /// List migrations and, with a database, which are applied vs pending.
    Status {
        #[arg(long, default_value = "migrations")]
        dir: String,
        #[arg(long)]
        database_url: Option<String>,
    },
}

impl MigrateCommand {
    fn run(self) -> anyhow::Result<()> {
        use crate::migrate;

        match self {
            MigrateCommand::Generate { name, dir, no_input } => {
                migrate::generate(Path::new(&dir), &name, !no_input)
            }
            MigrateCommand::Apply { dir, database_url } => migrate::apply(Path::new(&dir), database_url),
            MigrateCommand::Revert { dir, database_url, yes } => {
                migrate::revert(Path::new(&dir), database_url, yes)
            }
            MigrateCommand::Baseline { name, dir, database_url } => {
                migrate::baseline(Path::new(&dir), &name, database_url)
            }
            MigrateCommand::Diff { dir, database_url, write, no_input } => {
                migrate::diff_live(Path::new(&dir), database_url, write, !no_input)
            }
            MigrateCommand::Status { dir, database_url } => migrate::status(Path::new(&dir), database_url),
        }
    }
}

/// Parse argv and dispatch. Returns a process exit code.
///
/// `default_out` is used when `--out` is omitted; consumer binaries typically
/// pass a compile-time path (e.g. `concat!(env!("CARGO_MANIFEST_DIR"), ...)`)
/// so the command is cross-platform and CWD-independent.
pub fn main(default_out: &str) -> ExitCode {
    report(Cli::parse().command.run(default_out))
}

fn report(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", crate::style::failure(&format!("{error:#}")));
            ExitCode::FAILURE
        }
    }
}
