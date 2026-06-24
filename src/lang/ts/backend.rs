//! The TypeScript [`ExportBackend`](crate::export::ExportBackend). Other
//! languages would live in sibling modules under [`crate::lang`].

use crate::export::{ExportBackend, Field, FieldType};

/// Emits TypeScript `export type` declarations.
pub struct TypeScript;

impl ExportBackend for TypeScript {
    fn doc(&self, lines: &[&str]) -> String {
        if lines.is_empty() {
            return String::new();
        }

        let body = lines
            .iter()
            .map(|line| if line.is_empty() { " *".to_string() } else { format!(" * {line}") })
            .collect::<Vec<_>>()
            .join("\n");

        format!("/**\n{body}\n */\n")
    }

    fn type_expr(&self, ty: &FieldType) -> String {
        match ty {
            FieldType::Bool => "boolean".into(),
            FieldType::Number => "number".into(),
            FieldType::String | FieldType::Timestamp | FieldType::Uuid => "string".into(),
            FieldType::Json => "unknown".into(),
            FieldType::Array(inner) => format!("Array<{}>", self.type_expr(inner)),
            FieldType::Named(name) => (*name).to_string(),
        }
    }

    fn struct_decl(&self, name: &str, docs: &str, fields: &[Field]) -> String {
        let body: String = fields
            .iter()
            .map(|f| {
                let optional = if f.optional { "?" } else { "" };
                format!("{}{optional}: {}, ", f.name, self.type_expr(f.ty))
            })
            .collect();
        format!("{docs}export type {name} = {{ {body}}};")
    }

    fn enum_decl(&self, name: &str, docs: &str, variants: &[&str]) -> String {
        let union = variants
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(" | ");

        format!("{docs}export type {name} = {union};")
    }

    fn import(&self, module: &str, names: &[&str]) -> String {
        format!("import type {{ {} }} from \"{module}\";", names.join(", "))
    }
}
