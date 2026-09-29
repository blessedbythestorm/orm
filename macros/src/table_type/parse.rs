use std::collections::{HashMap, HashSet};

use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Expr, ExprCall, ExprPath, Fields, Ident, ItemStruct, LitStr, Token, Type,
    parenthesized, parse::ParseStream, spanned::Spanned,
};

use super::check_name::{MAX_IDENTIFIER_BYTES, stable_check_name, stable_index_name};

use super::expression::{predicate_can_be_unknown, render_default, render_predicate};

pub struct TableDef {
    pub name: syn::Ident,
    pub name_snake: String,
    pub fields: Vec<FieldDef>,
    pub config: TableConfig,
    pub constraints: Vec<ConstraintSpec>,
    pub indexes: Vec<IndexSpec>,
}

/// A database constraint declared through struct- or field-level `#[pg]`.
pub struct ConstraintSpec {
    pub name: String,
    pub inferred_name_prefix: Option<String>,
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
    name_is_inferred: bool,
}

pub struct FieldDef {
    pub name: syn::Ident,
    pub name_str: String,
    pub ty: Type,
    pub is_optional: bool,
    pub is_auto_generated: bool,
    pub is_insert_skip: bool,
    pub is_insert_internal: bool,
    pub is_update_skip: bool,
    pub is_primary: bool,
    pub is_unique: bool,
    pub default: Option<String>,
    pub foreign_key: Option<ForeignKeySpec>,
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
    pub fn parse(input: &ItemStruct) -> syn::Result<Self> {
        let name = input.ident.clone();
        let name_snake = name.to_string()
            .to_snake_case();

        let config = TableConfig::parse(&input.attrs)?;

        let named_fields = match &input.fields {
            Fields::Named(fields) => &fields.named,
            _ => return Err(syn::Error::new(input.span(), "TableType only supports structs with named fields")),
        };

        let columns = named_fields
            .iter()
            .map(
                |field| field.ident.as_ref()
                    .expect("named field")
                    .to_string()
            )
            .collect::<HashSet<_>>();

        let nullable_columns = named_fields
            .iter()
            .filter(|field| is_option_type(&field.ty))
            .map(
                |field| field.ident.as_ref()
                    .expect("named field")
                    .to_string()
            )
            .collect::<HashSet<_>>();

        let mut fields = Vec::new();
        let mut constraints = Vec::new();
        let mut indexes = Vec::new();
        for field in named_fields {
            let parsed = FieldDef::parse(
                field,
                &config.table,
                &columns,
                &nullable_columns
            )?;

            fields.push(parsed.field);
            constraints.extend(parsed.constraints);
            indexes.extend(parsed.indexes);
        }

        let table = TableSpec::parse(
            &input.attrs,
            &config.table,
            &columns,
            &nullable_columns,
        )?;

        constraints.extend(table.constraints);
        indexes.extend(table.indexes);
        stabilize_colliding_index_names(&config.table, &mut indexes);
        validate_schema_objects(&config.table, &constraints, &indexes)?;

        Ok(Self {
            name,
            name_snake,
            fields,
            config,
            constraints,
            indexes,
        })
    }

    pub fn export_path(&self) -> &str {
        &self.config.export_to
    }

    pub fn full_table_name(&self) -> String {
        format!("{}.{}", self.config.schema, self.config.table)
    }

    pub fn insert_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter()
            .filter(|f| !f.is_insert_skip)
    }

    pub fn client_insert_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.insert_fields()
            .filter(|f| !f.is_insert_internal)
    }

    pub fn update_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter()
            .filter(|f| !f.is_update_skip && !f.is_insert_skip)
    }

    pub fn primary_key_name(&self) -> &str {
        let mut primary_keys = self.fields.iter()
            .filter(|field| field.is_primary);

        let primary_key = primary_keys.next();

        assert!(
            primary_key.is_none() || primary_keys.next().is_none(),
            "{} uses a composite primary key, which single-record CRUD does not support",
            self.name,
        );

        primary_key
            .or_else(
                || self.fields.iter()
                    .find(|field| field.name_str == "id")
            )
            .map(|field| field.name_str.as_str())
            .unwrap_or_else(|| panic!("{} must declare a #[pg(primary)] field or an id field", self.name))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use syn::parse_quote;

    use super::{ConstraintKindSpec, MAX_IDENTIFIER_BYTES, TableDef, stable_check_name, stable_index_name};

    #[test]
    fn single_record_crud_uses_the_declared_primary_key() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(primary)]
                user_id: uuid::Uuid,
                display_name: String,
            }
        };

        assert_eq!(TableDef::parse(&item).unwrap().primary_key_name(), "user_id");
    }

    #[test]
    fn explicit_schema_object_names_are_preserved() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(primary)]
                id: uuid::Uuid,
                #[pg(index(
                    name = "legacy_profiles_owner_idx",
                    unique,
                    where(active == true),
                ))]
                owner_id: uuid::Uuid,
                #[pg(index)]
                name: String,
                #[pg(validate(active == true, name = "legacy_profiles_active_check"))]
                active: bool,
            }
        };

        let table = TableDef::parse(&item)
            .unwrap();

        assert_eq!(table.constraints[0].name, "legacy_profiles_active_check");
        assert_eq!(table.indexes[0].name, "legacy_profiles_owner_idx");
        assert!(table.indexes[0].unique);
        assert_eq!(table.indexes[1].name, "profiles_name_idx");
    }

    #[test]
    fn repeated_pg_attributes_accumulate_and_render_structured_rules() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            #[pg(validate(
                present_iff(retired_at, status == Status::Retired),
                name = "profiles_retired_at_presence_check",
            ))]
            #[pg(index(
                columns(owner_id, created_at),
                name = "profiles_owner_created_idx",
                where(status != Status::Retired),
            ))]
            struct Profile {
                #[pg(primary)]
                #[pg(default_value(gen_random_uuid()))]
                id: uuid::Uuid,
                owner_id: uuid::Uuid,
                status: Status,
                retired_at: Option<String>,
                created_at: String,
            }
        };

        let table = TableDef::parse(&item)
            .unwrap();

        assert_eq!(table.fields[0].default.as_deref(), Some("gen_random_uuid()"));
        assert_eq!(table.constraints[0].name, "profiles_retired_at_presence_check");
        let ConstraintKindSpec::Check { expression } = &table.constraints[0].kind else {
            panic!("expected a check constraint")
        };

        assert_eq!(
            expression,
            "(retired_at IS NOT NULL) = (status = '__orm_enum__Status__Retired')",
        );

        assert_eq!(table.indexes[0].columns, ["owner_id", "created_at"]);
        assert_eq!(
            table.indexes[0].predicate.as_deref(),
            Some("status <> '__orm_enum__Status__Retired'"),
        );
    }

    #[test]
    fn default_value_distinguishes_literals_generators_and_signed_numbers() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "defaults", export_to = "types/defaults.ts")]
            struct Defaults {
                #[pg(primary, default_value(gen_random_uuid()))]
                id: uuid::Uuid,
                #[pg(default_value("now()"))]
                text: String,
                #[pg(default_value("O'Reilly"))]
                quoted_text: String,
                #[pg(default_value(true))]
                enabled: bool,
                #[pg(default_value(-12.50))]
                adjustment: f64,
                #[pg(default_value(now()))]
                created_at: chrono::DateTime<chrono::Utc>,
                #[pg(default_value(Status::Ready))]
                status: Status,
            }
        };

        let table = TableDef::parse(&item)
            .unwrap();

        let defaults = table
            .fields
            .iter()
            .map(
                |field| field.default.as_deref()
                    .unwrap()
            )
            .collect::<Vec<_>>();

        assert_eq!(
            defaults,
            [
                "gen_random_uuid()",
                "'now()'",
                "'O''Reilly'",
                "true",
                "-12.50",
                "now()",
                "'__orm_enum__Status__Ready'",
            ],
        );
    }

    #[test]
    fn validation_scope_and_presence_rules_are_checked() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "memberships", export_to = "types/memberships.ts")]
            #[pg(validate(
                present_iff(revoked_at, state == State::Revoked),
                name = "memberships_revoked_at_check",
            ))]
            #[pg(validate(
                required_if(actor_id, state != State::Draft),
                name = "memberships_actor_id_check",
            ))]
            struct Membership {
                #[pg(primary)]
                id: uuid::Uuid,
                state: State,
                revoked_at: Option<String>,
                actor_id: Option<uuid::Uuid>,
                #[pg(validate(amount > 0, name = "memberships_amount_check"))]
                amount: i32,
            }
        };

        let table = TableDef::parse(&item)
            .unwrap();

        let expressions = table
            .constraints
            .iter()
            .map(|constraint| match &constraint.kind {
                ConstraintKindSpec::Check { expression } => expression.as_str(),
                ConstraintKindSpec::Unique { .. } => panic!("expected checks"),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            expressions,
            [
                "amount > 0",
                "(revoked_at IS NOT NULL) = (state = '__orm_enum__State__Revoked')",
                "NOT (state <> '__orm_enum__State__Draft') OR actor_id IS NOT NULL",
            ],
        );

        let non_nullable_target = parse_quote! {
            #[table_type(schema = "public", name = "memberships", export_to = "types/memberships.ts")]
            #[pg(validate(present_iff(revoked_at, active == true)))]
            struct Membership {
                revoked_at: String,
                active: bool,
            }
        };

        assert!(
            TableDef::parse(&non_nullable_target)
                .err()
                .unwrap()
                .to_string()
                .contains("must be an Option column"),
        );

        let cross_column_field_rule = parse_quote! {
            #[table_type(schema = "public", name = "memberships", export_to = "types/memberships.ts")]
            struct Membership {
                #[pg(validate(started_at <= finished_at))]
                started_at: i32,
                finished_at: i32,
            }
        };

        assert!(
            TableDef::parse(&cross_column_field_rule)
                .err()
                .unwrap()
                .to_string()
                .contains("cross-column rules on the struct"),
        );

        let single_column_struct_rule = parse_quote! {
            #[table_type(schema = "public", name = "memberships", export_to = "types/memberships.ts")]
            #[pg(validate(amount > 0))]
            struct Membership {
                amount: i32,
            }
        };

        assert!(
            TableDef::parse(&single_column_struct_rule)
                .err()
                .unwrap()
                .to_string()
                .contains("belongs on that field"),
        );
    }

    #[test]
    fn index_shapes_preserve_keys_predicates_names_and_uniqueness() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "memberships", export_to = "types/memberships.ts")]
            #[pg(unique(
                columns(owner_id, label),
                name = "memberships_owner_label_key",
            ))]
            #[pg(index(
                columns(owner_id, created_at),
                unique,
                name = "memberships_owner_created_idx",
                where(active == true),
            ))]
            struct Membership {
                #[pg(primary)]
                id: uuid::Uuid,
                #[pg(index)]
                owner_id: uuid::Uuid,
                label: String,
                active: bool,
                created_at: String,
                #[pg(index(
                    unique,
                    name = "memberships_active_token_idx",
                    where(active == true),
                ))]
                token: Option<String>,
                #[pg(index(
                    name = "memberships_positive_score_idx",
                    where(score > 0),
                ))]
                score: Option<i32>,
            }
        };

        let table = TableDef::parse(&item)
            .unwrap();

        assert_eq!(table.indexes[0].columns, ["owner_id"]);
        assert_eq!(table.indexes[1].columns, ["token"]);
        assert!(table.indexes[1].unique);
        assert_eq!(table.indexes[1].predicate.as_deref(), Some("active = true"));
        assert_eq!(table.indexes[2].columns, ["score"]);
        assert_eq!(table.indexes[2].predicate.as_deref(), Some("score > 0"));
        assert_eq!(table.indexes[3].columns, ["owner_id", "created_at"]);
        assert!(table.indexes[3].unique);
        assert_eq!(table.indexes[3].name, "memberships_owner_created_idx");
        assert_eq!(table.indexes[3].predicate.as_deref(), Some("active = true"));

        let ConstraintKindSpec::Unique { columns } = &table.constraints[0].kind else {
            panic!("expected compound unique constraint")
        };

        assert_eq!(columns, &["owner_id", "label"]);
        assert_eq!(table.constraints[0].name, "memberships_owner_label_key");
    }

    #[test]
    fn unnamed_partial_indexes_have_stable_distinct_names() {
        let item = parse_quote! {
            #[table_type(schema = "public", name = "inventory_movements", export_to = "types/movements.ts")]
            struct InventoryMovement {
                operation: String,
                #[pg(index(unique, where(operation != "run_input_return")))]
                #[pg(index(where(operation == "run_input_return")))]
                reverses_movement_id: Option<uuid::Uuid>,
                #[pg(index(where(is_not_null(actor_id))))]
                actor_id: Option<uuid::Uuid>,
            }
        };

        let table = TableDef::parse(&item)
            .unwrap();

        let prefix = "inventory_movements_reverses_movement_id";

        assert_eq!(
            table.indexes[0].name,
            stable_index_name(prefix, Some("operation <> 'run_input_return'"), true),
        );

        assert_eq!(
            table.indexes[1].name,
            stable_index_name(prefix, Some("operation = 'run_input_return'"), false),
        );

        assert_ne!(table.indexes[0].name, table.indexes[1].name);
        assert!(table.indexes.iter().all(|index| index.name.len() <= MAX_IDENTIFIER_BYTES));
        assert_eq!(table.indexes[2].name, "inventory_movements_actor_id_partial_idx");
    }

    #[test]
    fn invalid_schema_expressions_and_colliding_names_are_rejected() {
        let nullable_unknown = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(validate(score > 0))]
                score: Option<i32>,
            }
        };

        assert!(
            TableDef::parse(&nullable_unknown)
                .err()
                .unwrap()
                .to_string()
                .contains("can evaluate to SQL UNKNOWN"),
        );

        let guarded_nullable = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(validate(is_null(score) || score > 0))]
                score: Option<i32>,
            }
        };

        assert!(TableDef::parse(&guarded_nullable).is_ok());

        let unknown_column = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(validate(missing > 0))]
                amount: i32,
            }
        };

        assert!(
            TableDef::parse(&unknown_column)
                .err()
                .unwrap()
                .to_string()
                .contains("unknown table column `missing`"),
        );

        let unsupported_function = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(validate(lower(name) == "ready"))]
                name: String,
            }
        };

        assert!(
            TableDef::parse(&unsupported_function)
                .err()
                .unwrap()
                .to_string()
                .contains("unsupported PostgreSQL predicate function `lower`"),
        );

        let duplicate_name = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(validate(active == true, name = "profiles_rule"))]
                active: bool,
                #[pg(index(name = "profiles_rule"))]
                owner_id: uuid::Uuid,
            }
        };

        assert!(
            TableDef::parse(&duplicate_name)
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate schema object name `profiles_rule`"),
        );
    }

    #[test]
    fn inferred_check_names_are_stable_across_declaration_order() {
        let first: syn::ItemStruct = parse_quote! {
            #[table_type(schema = "public", name = "measurements", export_to = "types/measurements.ts")]
            #[pg(validate(gross > tare))]
            #[pg(validate(net == gross - tare))]
            struct Measurement {
                gross: i32,
                tare: i32,
                net: i32,
            }
        };

        let second: syn::ItemStruct = parse_quote! {
            #[table_type(schema = "public", name = "measurements", export_to = "types/measurements.ts")]
            #[pg(validate((net == (gross - tare))))]
            #[pg(validate((gross > tare)))]
            #[pg(validate(gross + net >= tare))]
            struct Measurement {
                gross: i32,
                tare: i32,
                net: i32,
            }
        };

        let names = |item: &syn::ItemStruct| {
            let table = TableDef::parse(item)
                .unwrap();

            table.constraints.into_iter()
                .map(|constraint| {
                    let ConstraintKindSpec::Check { expression } = constraint.kind else {
                        panic!("expected a check constraint")
                    };

                    (expression, constraint.name)
                })
                .collect::<HashMap<_, _>>()
        };

        let first_names = names(&first);
        let second_names = names(&second);
        let repeated_names = names(&first);

        for expression in ["gross > tare", "net = gross - tare"] {
            assert_eq!(first_names.get(expression), second_names.get(expression));
        }

        assert_eq!(first_names, repeated_names);
        assert!(first_names.values().all(|name| name.len() <= MAX_IDENTIFIER_BYTES));
        assert!(first_names.values().all(|name| !name.contains("validation_1")));
        assert_ne!(
            first_names.get("gross > tare"),
            second_names.get("gross + net >= tare"),
        );
    }

    #[test]
    fn duplicate_inferred_checks_report_the_stable_name_collision() {
        let duplicate: syn::ItemStruct = parse_quote! {
            #[table_type(schema = "public", name = "measurements", export_to = "types/measurements.ts")]
            #[pg(validate(gross > tare))]
            #[pg(validate((gross > tare)))]
            struct Measurement {
                gross: i32,
                tare: i32,
            }
        };

        let error = TableDef::parse(&duplicate)
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains("duplicate schema object name `measurements_gross_tare_"));
    }

    #[test]
    fn inferred_check_names_retain_the_digest_within_postgres_byte_limits() {
        let first = stable_check_name(
            "measurements_a_very_long_multibyte_éééééééééééééééééééé_column",
            "gross > tare",
        );

        let second = stable_check_name(
            "measurements_a_very_long_multibyte_éééééééééééééééééééé_column",
            "gross >= tare",
        );

        assert!(first.len() <= MAX_IDENTIFIER_BYTES);
        assert!(first.ends_with("_check"));
        assert_ne!(first, second);
    }

    #[test]
    fn malformed_repeated_pg_options_return_diagnostics() {
        let typo = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(primari)]
                id: uuid::Uuid,
            }
        };

        assert!(
            TableDef::parse(&typo)
                .err()
                .unwrap()
                .to_string()
                .contains("unknown pg field option"),
        );

        let duplicate = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(default_value(1))]
                #[pg(default_value(2))]
                id: i32,
            }
        };

        assert!(
            TableDef::parse(&duplicate)
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate default_value"),
        );

        let unsupported = parse_quote! {
            #[table_type(schema = "public", name = "profiles", export_to = "types/profiles.ts")]
            struct Profile {
                #[pg(default_value(clock_timestamp()))]
                id: String,
            }
        };

        assert!(
            TableDef::parse(&unsupported)
                .err()
                .unwrap()
                .to_string()
                .contains("unsupported database default generator"),
        );
    }
}

struct ParsedField {
    field: FieldDef,
    constraints: Vec<ConstraintSpec>,
    indexes: Vec<IndexSpec>,
}

impl FieldDef {
    fn parse(
        field: &syn::Field,
        table: &str,
        columns: &HashSet<String>,
        nullable_columns: &HashSet<String>,
    ) -> syn::Result<ParsedField> {
        let name = field.ident.clone()
            .expect("Field must have a name");

        let name_str = name.to_string()
            .trim_start_matches("r#")
            .to_string();

        let ty = field.ty.clone();
        let is_optional = is_option_type(&ty);

        let pg = PgSpec::parse(
            &field.attrs,
            table,
            &name_str,
            columns,
            nullable_columns,
        )?;

        let crud = CrudSpec::parse(&field.attrs)?;

        // `#[pg(primary)]` columns are database-generated and immutable, so they
        // are optional on insert and excluded from updates.
        let is_auto_generated = pg.primary || crud.insert_optional;
        let is_insert_skip = crud.insert_skip;
        let is_insert_internal = crud.insert_internal;
        let is_update_skip = pg.primary || crud.update_skip;

        Ok(ParsedField {
            field: Self {
                name,
                name_str,
                ty,
                is_optional,
                is_auto_generated,
                is_insert_skip,
                is_insert_internal,
                is_update_skip,
                is_primary: pg.primary,
                is_unique: pg.unique,
                default: pg.default,
                foreign_key: pg.foreign,
            },
            constraints: pg.constraints,
            indexes: pg.indexes,
        })
    }

    pub fn as_option_type(&self) -> TokenStream {
        let ty = &self.ty;
        if self.is_optional {
            quote! { #ty }
        } else {
            quote! { Option<#ty> }
        }
    }

    pub fn as_update_type(&self) -> TokenStream {
        let ty = &self.ty;

        quote! { Option<#ty> }
    }
}

impl TableConfig {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let attr = attrs
            .iter()
            .find(
                |attr| attr.path()
                    .is_ident("table_type")
            )
            .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing table_type attribute"))?;

        let mut schema = None;
        let mut table = None;
        let mut export_to = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("schema") {
                schema = Some(
                    meta.value()?
                        .parse::<LitStr>()?
                        .value()
                );
            } else if meta.path.is_ident("name") {
                table = Some(
                    meta.value()?
                        .parse::<LitStr>()?
                        .value()
                );
            } else if meta.path.is_ident("export_to") {
                export_to = Some(
                    meta.value()?
                        .parse::<LitStr>()?
                        .value()
                );
            } else {
                return Err(meta.error("unknown table_type option"));
            }

            Ok(())
        })?;

        Ok(Self {
            schema: schema.ok_or_else(|| syn::Error::new(attr.span(), "table_type requires schema"))?,
            table: table.ok_or_else(|| syn::Error::new(attr.span(), "table_type requires name"))?,
            export_to: export_to.ok_or_else(|| syn::Error::new(attr.span(), "table_type requires export_to"))?,
        })
    }
}

/// Storage metadata from repeatable field-level `#[pg(...)]` declarations.
#[derive(Default)]
struct PgSpec {
    primary: bool,
    unique: bool,
    default: Option<String>,
    foreign: Option<ForeignKeySpec>,
    constraints: Vec<ConstraintSpec>,
    indexes: Vec<IndexSpec>,
}

impl PgSpec {
    fn parse(
        attrs: &[Attribute],
        table: &str,
        field: &str,
        columns: &HashSet<String>,
        nullable_columns: &HashSet<String>,
    ) -> syn::Result<Self> {
        let mut spec = Self::default();
        for attr in attrs.iter()
            .filter(
                |attr| attr.path()
                    .is_ident("pg")
            ) {
            attr.parse_args_with(|input: ParseStream| {
                while !input.is_empty() {
                    let key: Ident = input.parse()?;
                    match key.to_string()
                        .as_str()
                    {
                        "primary" => set_once_flag(&mut spec.primary, &key, "primary")?,
                        "unique" => set_once_flag(&mut spec.unique, &key, "unique")?,
                        "default_value" => {
                            reject_duplicate(spec.default.is_some(), &key, "default_value")?;
                            let content;
                            parenthesized!(content in input);
                            spec.default = Some(render_default(&content.parse::<Expr>()?)?);
                        }
                        "foreign" => {
                            reject_duplicate(spec.foreign.is_some(), &key, "foreign")?;
                            let content;
                            parenthesized!(content in input);
                            spec.foreign = Some(parse_foreign_value(&content)?);
                        }
                        "validate" => {
                            let content;
                            parenthesized!(content in input);
                            spec.constraints.push(parse_validation(
                                &content,
                                table,
                                columns,
                                nullable_columns,
                                Some(field),
                            )?);
                        }
                        "index" => {
                            let options = if input.peek(syn::token::Paren) {
                                let content;
                                parenthesized!(content in input);
                                parse_index_options(&content, columns, Some(field))?
                            } else {
                                IndexOptions { columns: vec![field.to_string()], ..Default::default() }
                            };

                            spec.indexes.push(options.finish(table)?);
                        }
                        other => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("unknown pg field option `{other}`"),
                            ));
                        }
                    }

                    parse_comma(input)?;
                }

                Ok(())
            })?;
        }

        Ok(spec)
    }
}

/// Table-level storage rules from repeatable `#[pg(unique(...))]`,
/// `#[pg(validate(...))]`, and `#[pg(index(...))]` declarations.
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

impl TableSpec {
    fn parse(
        attrs: &[Attribute],
        table: &str,
        columns: &HashSet<String>,
        nullable_columns: &HashSet<String>,
    ) -> syn::Result<Self> {
        let mut spec = Self::default();

        for attr in attrs.iter()
            .filter(
                |attr| attr.path()
                    .is_ident("pg")
            ) {
            attr.parse_args_with(|input: ParseStream| {
                while !input.is_empty() {
                    let key: Ident = input.parse()?;
                    let content;
                    parenthesized!(content in input);
                    match key.to_string()
                        .as_str()
                    {
                        "validate" => spec.constraints.push(parse_validation(
                            &content,
                            table,
                            columns,
                            nullable_columns,
                            None,
                        )?),
                        "unique" => {
                            spec.constraints.push(parse_struct_unique(&content, table, columns)?);
                        }
                        "index" => {
                            spec.indexes.push(
                                parse_index_options(&content, columns, None)?
                                    .finish(table)?,
                            );
                        }
                        other => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!(
                                    "unknown pg struct option `{other}`; expected validate, unique, or index"
                                ),
                            ));
                        }
                    }

                    parse_comma(input)?;
                }

                Ok(())
            })?;
        }

        Ok(spec)
    }
}

fn parse_column_list(
    input: ParseStream,
    declared: &HashSet<String>,
) -> syn::Result<Vec<String>> {
    let mut columns = Vec::new();

    while !input.is_empty() {
        let column = input.parse::<Ident>()?;
        let name = column.to_string();
        if !declared.contains(&name) {
            return Err(syn::Error::new(
                column.span(),
                format!("unknown table column `{name}`"),
            ));
        }

        if columns.contains(&name) {
            return Err(syn::Error::new(
                column.span(),
                format!("duplicate index/constraint column `{name}`"),
            ));
        }

        columns.push(name);

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }

    Ok(columns)
}

fn parse_validation(
    input: ParseStream,
    table: &str,
    columns: &HashSet<String>,
    nullable_columns: &HashSet<String>,
    field: Option<&str>,
) -> syn::Result<ConstraintSpec> {
    let expression = input.parse::<Expr>()?;
    validate_presence_targets(&expression, nullable_columns)?;
    let mut referenced_columns = HashSet::new();
    collect_column_references(&expression, columns, &mut referenced_columns);
    match field {
        Some(field) if referenced_columns.iter()
            .any(|column| column != field) => {
                return Err(syn::Error::new(
                    expression.span(),
                    "a field validation may reference only its own column; put cross-column rules on the struct",
                ));
            }
        None if referenced_columns.len() == 1 => {
            return Err(syn::Error::new(
                expression.span(),
                "a single-column validation belongs on that field",
            ));
        }
        _ => {}
    }

    let mut explicit_name = None;
    while !input.is_empty() {
        input.parse::<Token![,]>()?;
        if input.is_empty() {
            break;
        }

        let key: Ident = input.parse()?;
        if key != "name" {
            return Err(syn::Error::new(
                key.span(),
                "validate expects only `name = \"...\"` after its predicate",
            ));
        }

        input.parse::<Token![=]>()?;
        reject_duplicate(explicit_name.is_some(), &key, "validation name")?;
        explicit_name = Some(
            input.parse::<LitStr>()?
                .value()
        );
    }

    let rendered = render_predicate(&expression, columns)?;
    if predicate_can_be_unknown(&expression, nullable_columns)? {
        return Err(syn::Error::new(
            expression.span(),
            "database validation can evaluate to SQL UNKNOWN; handle nullable columns explicitly with is_null/is_not_null or a total presence rule",
        ));
    }

    let (name, inferred_name_prefix) = if let Some(name) = explicit_name {
        (name, None)
    }
    else if let Some(field) = field {
        (format!("{table}_{field}_check"), None)
    }
    else if let Some(name) = presence_validation_name(table, &expression) {
        (name, None)
    }
    else {
        let prefix = validation_name_prefix(table, &referenced_columns);
        (stable_check_name(&prefix, &rendered), Some(prefix))
    };

    Ok(ConstraintSpec {
        name,
        inferred_name_prefix,
        kind: ConstraintKindSpec::Check { expression: rendered },
    })
}

fn presence_validation_name(table: &str, expression: &Expr) -> Option<String> {
    if let Expr::Call(ExprCall { func, args, .. }) = expression
        && let Expr::Path(ExprPath { path, .. }) = func.as_ref()
        && matches!(
            path.segments.last().map(|segment| segment.ident.to_string()).as_deref(),
            Some("present_iff" | "required_if")
        )
        && let Some(Expr::Path(target)) = args.first()
        && let Some(segment) = target.path.segments.last()
    {
        return Some(format!("{table}_{}_presence_check", segment.ident));
    }

    None
}

fn validation_name_prefix(table: &str, referenced_columns: &HashSet<String>) -> String {
    let mut columns = referenced_columns.iter()
        .map(String::as_str)
        .collect::<Vec<_>>();

    columns.sort_unstable();
    let columns = columns.into_iter()
        .take(2)
        .collect::<Vec<_>>();

    if columns.is_empty() {
        format!("{table}_validation")
    }
    else {
        format!("{table}_{}", columns.join("_"))
    }
}

fn validate_presence_targets(
    expression: &Expr,
    nullable_columns: &HashSet<String>,
) -> syn::Result<()> {
    match expression {
        Expr::Call(call) => {
            let function = match call.func.as_ref() {
                Expr::Path(path) if path.path.segments.len() == 1 => {
                    path.path.segments[0].ident.to_string()
                }
                _ => String::new(),
            };

            if matches!(function.as_str(), "present_iff" | "required_if") {
                let Some(Expr::Path(target)) = call.args.first() else {
                    return Err(syn::Error::new(
                        call.span(),
                        format!("{function}'s first argument must be a nullable column"),
                    ));
                };

                let Some(segment) = target.path.segments.first() else {
                    return Err(syn::Error::new(target.span(), "missing presence target"));
                };

                let name = segment.ident.to_string();
                if target.path.segments.len() != 1 || !nullable_columns.contains(&name) {
                    return Err(syn::Error::new(
                        target.span(),
                        format!("{function} target `{name}` must be an Option column"),
                    ));
                }
            }

            for argument in &call.args {
                validate_presence_targets(argument, nullable_columns)?;
            }
        }
        Expr::Binary(binary) => {
            validate_presence_targets(&binary.left, nullable_columns)?;
            validate_presence_targets(&binary.right, nullable_columns)?;
        }
        Expr::Unary(unary) => validate_presence_targets(&unary.expr, nullable_columns)?,
        Expr::Paren(paren) => validate_presence_targets(&paren.expr, nullable_columns)?,
        Expr::Group(group) => validate_presence_targets(&group.expr, nullable_columns)?,
        Expr::Array(array) => {
            for element in &array.elems {
                validate_presence_targets(element, nullable_columns)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn collect_column_references(
    expression: &Expr,
    columns: &HashSet<String>,
    found: &mut HashSet<String>,
) {
    match expression {
        Expr::Path(path) if path.path.segments.len() == 1 => {
            let name = path.path.segments[0].ident.to_string();
            if columns.contains(&name) {
                found.insert(name);
            }
        }
        Expr::Call(call) => {
            for argument in &call.args {
                collect_column_references(argument, columns, found);
            }
        }
        Expr::Binary(binary) => {
            collect_column_references(&binary.left, columns, found);
            collect_column_references(&binary.right, columns, found);
        }
        Expr::Unary(unary) => collect_column_references(&unary.expr, columns, found),
        Expr::Paren(paren) => collect_column_references(&paren.expr, columns, found),
        Expr::Group(group) => collect_column_references(&group.expr, columns, found),
        Expr::Array(array) => {
            for element in &array.elems {
                collect_column_references(element, columns, found);
            }
        }
        _ => {}
    }
}

fn parse_struct_unique(
    input: ParseStream,
    table: &str,
    declared: &HashSet<String>,
) -> syn::Result<ConstraintSpec> {
    let mut columns = None;
    let mut name = None;
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        match key.to_string()
            .as_str()
        {
            "columns" => {
                reject_duplicate(columns.is_some(), &key, "unique columns")?;
                let content;
                parenthesized!(content in input);
                columns = Some(parse_column_list(&content, declared)?);
            }
            "name" => {
                reject_duplicate(name.is_some(), &key, "unique name")?;
                input.parse::<Token![=]>()?;
                name = Some(
                    input.parse::<LitStr>()?
                        .value()
                );
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    "unique expects columns(...) and optional name",
                ));
            }
        }

        parse_comma(input)?;
    }

    let columns = columns.ok_or_else(|| input.error("struct unique requires columns(...)"))?;
    if columns.len() < 2 {
        return Err(input.error("a single-column unique constraint belongs on that field"));
    }

    let name = name.unwrap_or_else(|| format!("{table}_{}_key", columns.join("_")));
    Ok(ConstraintSpec {
        name,
        inferred_name_prefix: None,
        kind: ConstraintKindSpec::Unique { columns },
    })
}

#[derive(Default)]
struct IndexOptions {
    columns: Vec<String>,
    unique: bool,
    name: Option<String>,
    predicate: Option<String>,
}

impl IndexOptions {
    fn finish(self, table: &str) -> syn::Result<IndexSpec> {
        if self.columns.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "index requires at least one column",
            ));
        }

        let name_is_inferred = self.name.is_none();
        let suffix = if self.predicate.is_some() { "partial_idx" } else { "idx" };
        let name = self
            .name
            .unwrap_or_else(|| format!("{table}_{}_{suffix}", self.columns.join("_")));

        Ok(IndexSpec {
            name,
            columns: self.columns,
            unique: self.unique,
            predicate: self.predicate,
            name_is_inferred,
        })
    }
}

fn parse_index_options(
    input: ParseStream,
    declared: &HashSet<String>,
    field: Option<&str>,
) -> syn::Result<IndexOptions> {
    let mut options = IndexOptions::default();
    if let Some(field) = field {
        options.columns.push(field.to_string());
    }

    while !input.is_empty() {
        if input.peek(Token![where]) {
            let keyword: Token![where] = input.parse()?;
            reject_duplicate(options.predicate.is_some(), &keyword, "index predicate")?;
            let content;
            parenthesized!(content in input);
            options.predicate = Some(render_predicate(&content.parse::<Expr>()?, declared)?);
        } else {
            let key: Ident = input.parse()?;
            match key.to_string()
                .as_str()
            {
                "unique" => set_once_flag(&mut options.unique, &key, "index unique")?,
                "name" => {
                    reject_duplicate(options.name.is_some(), &key, "index name")?;
                    input.parse::<Token![=]>()?;
                    options.name = Some(
                        input.parse::<LitStr>()?
                            .value()
                    );
                }
                "columns" if field.is_none() => {
                    if !options.columns.is_empty() {
                        return Err(syn::Error::new(key.span(), "duplicate index columns"));
                    }

                    let content;
                    parenthesized!(content in input);
                    options.columns = parse_column_list(&content, declared)?;
                }
                "columns" => {
                    return Err(syn::Error::new(
                        key.span(),
                        "field indexes use their field as the key; declare compound indexes on the struct",
                    ));
                }
                _ => return Err(syn::Error::new(key.span(), "unknown index option")),
            }
        }

        parse_comma(input)?;
    }

    if field.is_none() && options.columns.is_empty() {
        return Err(input.error("struct index requires columns(...)"));
    }

    if field.is_none() && options.columns.len() < 2 {
        return Err(input.error("a single-column index belongs on that field"));
    }

    Ok(options)
}

/// CRUD-struct shaping from `#[crud(insert(optional|skip|internal), update(skip))]`.
#[derive(Default)]
struct CrudSpec {
    insert_optional: bool,
    insert_skip: bool,
    insert_internal: bool,
    update_skip: bool,
}

impl CrudSpec {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut spec = Self::default();
        for attr in attrs.iter()
            .filter(
                |attr| attr.path()
                    .is_ident("crud")
            ) {
            attr.parse_args_with(|input: ParseStream| {
                while !input.is_empty() {
                    let key: Ident = input.parse()?;
                    let content;
                    parenthesized!(content in input);
                    let value: Ident = content.parse()?;
                    match (
                        key.to_string()
                            .as_str(),
                        value.to_string()
                            .as_str(),
                    ) {
                        ("insert", "optional") => {
                            set_once_flag(&mut spec.insert_optional, &key, "insert(optional)")?;
                        }
                        ("insert", "skip") => {
                            set_once_flag(&mut spec.insert_skip, &key, "insert(skip)")?;
                        }
                        ("insert", "internal") => {
                            set_once_flag(&mut spec.insert_internal, &key, "insert(internal)")?;
                        }
                        ("update", "skip") => {
                            set_once_flag(&mut spec.update_skip, &key, "update(skip)")?;
                        }
                        _ => {
                            return Err(syn::Error::new(
                                key.span(),
                                "expected insert(optional|skip|internal) or update(skip)",
                            ));
                        }
                    }

                    parse_comma(input)?;
                }

                Ok(())
            })?;
        }

        if [spec.insert_optional, spec.insert_skip, spec.insert_internal]
            .into_iter()
            .filter(|enabled| *enabled)
            .count()
            > 1
        {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "insert(optional), insert(skip), and insert(internal) are mutually exclusive",
            ));
        }

        Ok(spec)
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
        match key.to_string()
            .as_str()
        {
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
    match ident.to_string()
        .as_str()
    {
        "no_action" => Ok(ReferentialActionToken::NoAction),
        "restrict" => Ok(ReferentialActionToken::Restrict),
        "cascade" => Ok(ReferentialActionToken::Cascade),
        "set_null" => Ok(ReferentialActionToken::SetNull),
        "set_default" => Ok(ReferentialActionToken::SetDefault),
        _ => Err(syn::Error::new(ident.span(), "unknown referential action")),
    }
}

fn validate_schema_objects(
    table: &str,
    constraints: &[ConstraintSpec],
    indexes: &[IndexSpec],
) -> syn::Result<()> {
    let mut names = HashMap::new();
    for (kind, name) in constraints
        .iter()
        .map(|constraint| ("constraint", &constraint.name))
        .chain(
            indexes.iter()
                .map(|index| ("index", &index.name))
        ) {
        if name.len() > MAX_IDENTIFIER_BYTES {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "{kind} name `{name}` exceeds PostgreSQL's {MAX_IDENTIFIER_BYTES}-byte identifier limit"
                ),
            ));
        }

        if let Some(previous) = names.insert(name, kind) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "{table} declares duplicate schema object name `{name}` ({previous} and {kind})"
                ),
            ));
        }
    }

    Ok(())
}

fn stabilize_colliding_index_names(table: &str, indexes: &mut [IndexSpec]) {
    let mut counts = HashMap::new();
    for index in indexes.iter() {
        *counts.entry(index.name.clone())
            .or_insert(0usize) += 1;
    }

    for index in indexes {
        if !index.name_is_inferred || counts.get(&index.name)
            .copied()
            .unwrap_or_default() < 2
        {
            continue;
        }

        let prefix = format!("{table}_{}", index.columns.join("_"));
        index.name = stable_index_name(&prefix, index.predicate.as_deref(), index.unique);
    }
}

fn set_once_flag(value: &mut bool, token: impl Spanned, label: &str) -> syn::Result<()> {
    reject_duplicate(*value, token, label)?;
    *value = true;
    Ok(())
}

fn reject_duplicate(duplicate: bool, token: impl Spanned, label: &str) -> syn::Result<()> {
    if duplicate {
        Err(syn::Error::new(
            token.span(),
            format!("duplicate {label} declaration"),
        ))
    } else {
        Ok(())
    }
}

fn parse_comma(input: ParseStream) -> syn::Result<()> {
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    } else if !input.is_empty() {
        return Err(input.error("expected `,`"));
    }

    Ok(())
}

fn is_option_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().map(|s| s.ident == "Option").unwrap_or(false))
}
