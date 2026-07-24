use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Fields, Ident, ItemStruct, Lit, LitStr, Token, Type, parenthesized, parse::ParseStream,
};

pub struct TableDef {
    pub name: syn::Ident,
    pub name_snake: String,
    pub fields: Vec<FieldDef>,
    pub config: TableConfig,
    pub constraints: Vec<ConstraintSpec>,
    pub indexes: Vec<IndexSpec>,
}

/// A table-level constraint declared by `#[table(unique(...), check(...))]`, or
/// a single-column check lifted from a field's `#[pg(check("..."))]`.
pub struct ConstraintSpec {
    pub name: String,
    pub kind: ConstraintKindSpec,
}

pub enum ConstraintKindSpec {
    Unique { columns: Vec<String> },
    Check { expression: String },
}

pub struct IndexSpec {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub predicate: Option<String>,
}

pub struct FieldDef {
    pub name: syn::Ident,
    pub name_str: String,
    pub ty: Type,
    pub is_optional: bool,
    pub is_auto_generated: bool,
    pub is_insert_skip: bool,
    pub is_update_skip: bool,
    pub is_primary: bool,
    pub is_unique: bool,
    pub default: Option<String>,
    pub foreign_key: Option<ForeignKeySpec>,
    pub check: Option<String>,
}

pub struct ForeignKeySpec {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub on_update: ReferentialActionToken,
    pub on_delete: ReferentialActionToken,
}

pub enum ReferentialActionToken {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialActionToken {
    pub fn path(&self) -> TokenStream {
        match self {
            Self::NoAction => quote! { ::orm::schema::ReferentialAction::NoAction },
            Self::Restrict => quote! { ::orm::schema::ReferentialAction::Restrict },
            Self::Cascade => quote! { ::orm::schema::ReferentialAction::Cascade },
            Self::SetNull => quote! { ::orm::schema::ReferentialAction::SetNull },
            Self::SetDefault => quote! { ::orm::schema::ReferentialAction::SetDefault },
        }
    }
}

pub struct TableConfig {
    pub schema: String,
    pub table: String,
    pub export_to: String,
}

impl TableDef {
    pub fn parse(input: &ItemStruct) -> Self {
        let name = input.ident.clone();
        let name_snake = name.to_string().to_snake_case();
        let config = TableConfig::parse(&input.attrs);

        let fields: Vec<FieldDef> = match &input.fields {
            Fields::Named(fields) => fields.named.iter().map(FieldDef::parse).collect(),
            _ => panic!("TableType only supports structs with named fields"),
        };

        let mut table = TableSpec::parse(&input.attrs, &config.table);
        table.assert_names_fit();

        // A field's own `#[pg(check(...))]` is just a one-column table check;
        // naming it after the column keeps the diff key stable.
        let mut constraints = table.constraints;
        for field in &fields {
            let Some(expression) = &field.check else {
                continue;
            };

            constraints.push(ConstraintSpec {
                name: format!("{}_{}_check", config.table, field.name_str),
                kind: ConstraintKindSpec::Check { expression: expression.clone() },
            });
        }

        Self { name, name_snake, fields, config, constraints, indexes: table.indexes }
    }

    pub fn export_path(&self) -> &str {
        &self.config.export_to
    }

    pub fn full_table_name(&self) -> String {
        format!("{}.{}", self.config.schema, self.config.table)
    }

    pub fn column_list(&self) -> String {
        self.fields.iter().map(|f| f.name_str.as_str()).collect::<Vec<_>>().join(", ")
    }

    pub fn insert_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter().filter(|f| !f.is_insert_skip)
    }

    pub fn update_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter().filter(|f| !f.is_update_skip && !f.is_insert_skip)
    }
}

impl FieldDef {
    pub fn parse(field: &syn::Field) -> Self {
        let name = field.ident.clone().expect("Field must have a name");
        let name_str = name.to_string();
        let ty = field.ty.clone();
        let is_optional = is_option_type(&ty);

        let pg = PgSpec::parse(&field.attrs);
        let crud = CrudSpec::parse(&field.attrs);

        // `#[pg(primary)]` columns are database-generated and immutable, so they
        // are optional on insert and excluded from updates.
        let is_auto_generated = pg.primary || crud.insert_optional;
        let is_insert_skip = crud.insert_skip;
        let is_update_skip = pg.primary || crud.update_skip;

        Self {
            name,
            name_str,
            ty,
            is_optional,
            is_auto_generated,
            is_insert_skip,
            is_update_skip,
            is_primary: pg.primary,
            is_unique: pg.unique,
            default: pg.default,
            foreign_key: pg.foreign,
            check: pg.check,
        }
    }

    pub fn as_option_type(&self) -> TokenStream {
        let ty = &self.ty;
        if self.is_optional {
            quote! { #ty }
        } else {
            quote! { Option<#ty> }
        }
    }
}

impl TableConfig {
    fn parse(attrs: &[Attribute]) -> Self {
        for attr in attrs {
            if !attr.path().is_ident("table_type") {
                continue;
            }

            let mut schema = None;
            let mut table = None;
            let mut export_to = None;

            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("schema")
                    && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
                {
                    schema = Some(s.value());
                } else if meta.path.is_ident("name")
                    && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
                {
                    table = Some(s.value());
                } else if meta.path.is_ident("export_to")
                    && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
                {
                    export_to = Some(s.value());
                }
                Ok(())
            });

            if let (Some(schema), Some(table), Some(export_to)) = (schema, table, export_to) {
                return Self { schema, table, export_to };
            }
        }

        panic!("table_type requires schema, name, and export_to attributes");
    }
}

/// Storage metadata from `#[pg(primary, unique, check("..."), default(...), foreign(...))]`.
#[derive(Default)]
struct PgSpec {
    primary: bool,
    unique: bool,
    default: Option<String>,
    foreign: Option<ForeignKeySpec>,
    check: Option<String>,
}

impl PgSpec {
    fn parse(attrs: &[Attribute]) -> Self {
        let mut spec = Self::default();
        let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("pg")) else {
            return spec;
        };

        let _ = attr.parse_args_with(|input: ParseStream| {
            while !input.is_empty() {
                let key: Ident = input.parse()?;
                match key.to_string().as_str() {
                    "primary" => spec.primary = true,
                    "unique" => spec.unique = true,
                    "default" => {
                        let content;
                        parenthesized!(content in input);
                        spec.default = Some(parse_default_value(&content)?);
                    }
                    "foreign" => {
                        let content;
                        parenthesized!(content in input);
                        spec.foreign = Some(parse_foreign_value(&content)?);
                    }
                    "check" => {
                        let content;
                        parenthesized!(content in input);
                        spec.check = Some(content.parse::<LitStr>()?.value());
                    }
                    other => return Err(syn::Error::new(key.span(), format!("unknown pg option `{other}`"))),
                }
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
            }
            Ok(())
        });

        spec
    }
}

/// Table-level storage rules from
/// `#[table(unique(a, b), check(name = "expr"), index(a, b), index(unique, a, where = "expr"))]`.
///
/// Constraint and index names are the diff keys, so an unnamed entry gets a
/// deterministic one derived from the table and its columns — the same name
/// Postgres would choose, which keeps a declared rule and an introspected one
/// comparing equal.
#[derive(Default)]
struct TableSpec {
    constraints: Vec<ConstraintSpec>,
    indexes: Vec<IndexSpec>,
}

/// Postgres' identifier limit. A longer name is silently truncated on the way
/// in, so the stored name stops matching the declared one and every diff
/// reports the same phantom change forever.
const MAX_IDENTIFIER_BYTES: usize = 63;

impl TableSpec {
    /// Fails the build on a constraint or index name Postgres would truncate.
    fn assert_names_fit(&self) {
        let names = self
            .constraints
            .iter()
            .map(|constraint| &constraint.name)
            .chain(self.indexes.iter().map(|index| &index.name));

        for name in names {
            if name.len() <= MAX_IDENTIFIER_BYTES {
                continue;
            }

            panic!(
                "constraint/index name `{name}` is {} bytes; Postgres truncates at \
                 {MAX_IDENTIFIER_BYTES}, which would make every diff report it as changed. \
                 Shorten the rule label.",
                name.len(),
            );
        }
    }

    fn parse(attrs: &[Attribute], table: &str) -> Self {
        let mut spec = Self::default();

        for attr in attrs.iter().filter(|attr| attr.path().is_ident("table")) {
            let result = attr.parse_args_with(|input: ParseStream| {
                while !input.is_empty() {
                    let key: Ident = input.parse()?;
                    let content;
                    parenthesized!(content in input);

                    match key.to_string().as_str() {
                        "unique" => spec.constraints.push(parse_unique(&content, table)?),
                        "check" => spec.constraints.push(parse_check(&content, table)?),
                        "index" => spec.indexes.push(parse_index(&content, table)?),
                        other => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("unknown table option `{other}`; expected unique, check, or index"),
                            ));
                        }
                    }

                    if input.peek(Token![,]) {
                        input.parse::<Token![,]>()?;
                    }
                }
                Ok(())
            });

            if let Err(error) = result {
                panic!("{error}");
            }
        }

        spec
    }
}

/// `unique(a, b)` — Postgres names a unique key `<table>_<cols>_key`.
fn parse_unique(input: ParseStream, table: &str) -> syn::Result<ConstraintSpec> {
    let columns = parse_column_list(input)?;

    if columns.is_empty() {
        return Err(input.error("unique expects at least one column"));
    }

    Ok(ConstraintSpec {
        name: format!("{table}_{}_key", columns.join("_")),
        kind: ConstraintKindSpec::Unique { columns },
    })
}

/// `check(positive_weight = "current_kg >= 0")` — the ident names the rule so
/// the constraint has a stable, readable identity in errors and in the diff.
fn parse_check(input: ParseStream, table: &str) -> syn::Result<ConstraintSpec> {
    let label: Ident = input.parse()?;
    input.parse::<Token![=]>()?;
    let expression = input.parse::<LitStr>()?.value();

    Ok(ConstraintSpec {
        name: format!("{table}_{label}_check"),
        kind: ConstraintKindSpec::Check { expression },
    })
}

/// `index(a, b)`, `index(unique, a)`, `index(a, where = "expr")`.
fn parse_index(input: ParseStream, table: &str) -> syn::Result<IndexSpec> {
    let mut unique = false;
    let mut columns = Vec::new();
    let mut predicate = None;

    while !input.is_empty() {
        if input.peek(Token![where]) {
            input.parse::<Token![where]>()?;
            input.parse::<Token![=]>()?;
            predicate = Some(input.parse::<LitStr>()?.value());
        } else {
            let ident: Ident = input.parse()?;

            if ident == "unique" {
                unique = true;
            } else {
                columns.push(ident.to_string());
            }
        }

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }

    if columns.is_empty() {
        return Err(input.error("index expects at least one column"));
    }

    let suffix = if predicate.is_some() { "partial_idx" } else { "idx" };

    Ok(IndexSpec {
        name: format!("{table}_{}_{suffix}", columns.join("_")),
        columns,
        unique,
        predicate,
    })
}

fn parse_column_list(input: ParseStream) -> syn::Result<Vec<String>> {
    let mut columns = Vec::new();

    while !input.is_empty() {
        columns.push(input.parse::<Ident>()?.to_string());

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }

    Ok(columns)
}

/// CRUD-struct shaping from `#[crud(insert(optional), insert(skip), update(skip))]`.
#[derive(Default)]
struct CrudSpec {
    insert_optional: bool,
    insert_skip: bool,
    update_skip: bool,
}

impl CrudSpec {
    fn parse(attrs: &[Attribute]) -> Self {
        let mut spec = Self::default();
        let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("crud")) else {
            return spec;
        };

        let _ = attr.parse_args_with(|input: ParseStream| {
            while !input.is_empty() {
                let key: Ident = input.parse()?;
                let content;
                parenthesized!(content in input);
                let value: Ident = content.parse()?;
                match (key.to_string().as_str(), value.to_string().as_str()) {
                    ("insert", "optional") => spec.insert_optional = true,
                    ("insert", "skip") => spec.insert_skip = true,
                    ("update", "skip") => spec.update_skip = true,
                    _ => {
                        return Err(syn::Error::new(
                            key.span(),
                            "expected insert(optional|skip) or update(skip)",
                        ));
                    }
                }
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
            }
            Ok(())
        });

        spec
    }
}

fn parse_default_value(input: ParseStream) -> syn::Result<String> {
    if input.peek(Ident) && input.peek2(syn::token::Paren) {
        let function: Ident = input.parse()?;
        if function != "sql" {
            return Err(syn::Error::new(function.span(), "expected `sql(\"...\")` or a literal"));
        }
        let content;
        parenthesized!(content in input);
        Ok(content.parse::<LitStr>()?.value())
    } else {
        Ok(render_default_literal(&input.parse::<Lit>()?))
    }
}

fn render_default_literal(literal: &Lit) -> String {
    match literal {
        Lit::Str(value) => format!("'{}'", value.value()),
        Lit::Int(value) => value.base10_digits().to_string(),
        Lit::Float(value) => value.base10_digits().to_string(),
        Lit::Bool(value) => value.value().to_string(),
        other => quote! { #other }.to_string(),
    }
}

fn parse_foreign_value(input: ParseStream) -> syn::Result<ForeignKeySpec> {
    let mut path = vec![input.parse::<Ident>()?];
    while input.peek(Token![.]) {
        input.parse::<Token![.]>()?;
        path.push(input.parse::<Ident>()?);
    }
    if path.len() != 3 {
        return Err(syn::Error::new(path[0].span(), "foreign expects schema.table.column"));
    }

    let mut on_update = ReferentialActionToken::NoAction;
    let mut on_delete = ReferentialActionToken::NoAction;
    while input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
        let key: Ident = input.parse()?;
        let content;
        parenthesized!(content in input);
        let action = parse_referential_action(&content.parse::<Ident>()?)?;
        match key.to_string().as_str() {
            "on_update" => on_update = action,
            "on_delete" => on_delete = action,
            _ => return Err(syn::Error::new(key.span(), "expected on_update or on_delete")),
        }
    }

    Ok(ForeignKeySpec {
        schema: path[0].to_string(),
        table: path[1].to_string(),
        column: path[2].to_string(),
        on_update,
        on_delete,
    })
}

fn parse_referential_action(ident: &Ident) -> syn::Result<ReferentialActionToken> {
    match ident.to_string().as_str() {
        "no_action" => Ok(ReferentialActionToken::NoAction),
        "restrict" => Ok(ReferentialActionToken::Restrict),
        "cascade" => Ok(ReferentialActionToken::Cascade),
        "set_null" => Ok(ReferentialActionToken::SetNull),
        "set_default" => Ok(ReferentialActionToken::SetDefault),
        _ => Err(syn::Error::new(ident.span(), "unknown referential action")),
    }
}

fn is_option_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().map(|s| s.ident == "Option").unwrap_or(false))
}
