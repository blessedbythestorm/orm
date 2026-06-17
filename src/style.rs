//! CLI styling built on the `colored` crate, which auto-disables color when
//! stdout isn't a terminal or `NO_COLOR` is set.

use colored::Colorize;

/// `✓ message` — a completed action.
pub fn success(message: &str) -> String {
    format!("{} {message}", "✓".green().bold())
}

/// `→ message` — an in-progress step or neutral info.
pub fn step(message: &str) -> String {
    format!("{} {message}", "→".cyan().bold())
}

/// `! message` — a warning or attention-worthy state.
pub fn warn(message: &str) -> String {
    format!("{} {message}", "!".yellow().bold())
}

/// `✗ message` — a failure.
pub fn failure(message: &str) -> String {
    format!("{} {message}", "✗".red().bold())
}

pub fn bold(text: &str) -> String {
    text.bold().to_string()
}

pub fn cyan(text: &str) -> String {
    text.cyan().to_string()
}
