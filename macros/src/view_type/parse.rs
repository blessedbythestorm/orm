use syn::{Attribute, Fields, Ident, ItemStruct, Lit, Token, parenthesized, parse::ParseStream};

/// A read-only projection backed by a database VIEW. Each field declares its
/// source `table.column` via `#[pg(view(...))]`; the FROM table and JOINs are
/// inferred from the tables' foreign keys when the schema is assembled.
pub struct ViewDef {
    pub name: syn::Ident,
    pub fields: Vec<ViewField>,
    pub config: ViewConfig,
}

pub struct ViewField {
    pub name: syn::Ident,
    pub name_str: String,
    pub source: ColumnSource,
}

/// A field's source column: `table.column` (schema defaults to `public`) or
/// `schema.table.column`.
pub struct ColumnSource {
    pub schema: String,
    pub table: String,
    pub column: String,
}

pub struct ViewConfig {
    pub schema: String,
    pub view: String,
    pub export_to: String,
    pub filter: Option<String>,
    pub order_by: Option<String>,
}

impl ViewDef {
    pub fn parse(input: &ItemStruct) -> Self {
        let name = input.ident.clone();
        let config = ViewConfig::parse(&input.attrs);

        let fields = match &input.fields {
            Fields::Named(fields) => fields.named.iter().map(ViewField::parse).collect(),
            _ => panic!("view_type only supports structs with named fields"),
        };

        Self { name, fields, config }
    }

    pub fn export_path(&self) -> &str {
        &self.config.export_to
    }

    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.config.schema, self.config.view)
    }

    /// The view's output columns (the field/alias names) — used as the SELECT list
    /// when querying the view.
    pub fn column_list(&self) -> String {
        self.fields.iter().map(|f| f.name_str.as_str()).collect::<Vec<_>>().join(", ")
    }
}

impl ViewField {
    fn parse(field: &syn::Field) -> Self {
        let name = field.ident.clone().expect("view field must have a name");
        let name_str = name.to_string();
        let source = ColumnSource::parse(&field.attrs, &name_str);
        Self { name, name_str, source }
    }
}

impl ColumnSource {
    fn parse(attrs: &[Attribute], field: &str) -> Self {
        let attr = attrs
            .iter()
            .find(|a| a.path().is_ident("pg"))
            .unwrap_or_else(|| panic!("view field `{field}` needs #[pg(view(table.column))]"));

        let mut parts: Option<Vec<String>> = None;
        let _ = attr.parse_args_with(|input: ParseStream| {
            let key: Ident = input.parse()?;
            if key != "view" {
                return Err(syn::Error::new(key.span(), "view fields only accept #[pg(view(...))]"));
            }
            let content;
            parenthesized!(content in input);
            let mut path = vec![content.parse::<Ident>()?.to_string()];
            while content.peek(Token![.]) {
                content.parse::<Token![.]>()?;
                path.push(content.parse::<Ident>()?.to_string());
            }
            parts = Some(path);
            Ok(())
        });

        match parts.as_deref() {
            Some([table, column]) => {
                Self { schema: "public".into(), table: table.clone(), column: column.clone() }
            }
            Some([schema, table, column]) => {
                Self { schema: schema.clone(), table: table.clone(), column: column.clone() }
            }
            _ => panic!("view field `{field}`: expected #[pg(view(table.column))] or #[pg(view(schema.table.column))]"),
        }
    }
}

impl ViewConfig {
    fn parse(attrs: &[Attribute]) -> Self {
        for attr in attrs {
            if !attr.path().is_ident("view_type") {
                continue;
            }

            let mut schema = None;
            let mut view = None;
            let mut export_to = None;
            let mut filter = None;
            let mut order_by = None;

            let _ = attr.parse_nested_meta(|meta| {
                let key = &meta.path;
                if let Ok(Lit::Str(s)) = meta.value().and_then(|v| v.parse::<Lit>()) {
                    if key.is_ident("schema") {
                        schema = Some(s.value());
                    } else if key.is_ident("name") {
                        view = Some(s.value());
                    } else if key.is_ident("export_to") {
                        export_to = Some(s.value());
                    } else if key.is_ident("filter") {
                        filter = Some(s.value());
                    } else if key.is_ident("order_by") {
                        order_by = Some(s.value());
                    }
                }
                Ok(())
            });

            if let (Some(schema), Some(view), Some(export_to)) = (schema, view, export_to) {
                return Self { schema, view, export_to, filter, order_by };
            }
        }

        panic!("view_type requires schema, name, and export_to attributes");
    }
}
