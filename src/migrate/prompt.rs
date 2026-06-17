use std::io::{self, Write};

use crate::schema::{RenameResolver, TableReference};
use crate::style;

/// Interactive rename resolver: asks the user y/N for each table or column
/// rename the differ proposes, so a rename preserves data instead of becoming a
/// destructive drop + add.
pub struct Prompt;

impl RenameResolver for Prompt {
    fn confirm_table_rename(&mut self, schema: &str, from: &str, to: &str) -> bool {
        ask(&format!("Rename table {schema}.{from} to {schema}.{to}?"))
    }

    fn confirm_column_rename(&mut self, table: &TableReference, from: &str, to: &str) -> bool {
        let table = table.qualified_name();
        ask(&format!("Rename column {table}.{from} to {table}.{to}?"))
    }
}

/// Prompts a yes/no question on stdin, defaulting to no.
pub fn ask(question: &str) -> bool {
    print!("{} ", style::cyan(&format!("{question} [y/N]")));
    let _ = io::stdout().flush();

    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
