use heck::ToSnakeCase;
use syn::{Attribute, ItemEnum, Lit};

pub struct EnumDef {
    pub name: syn::Ident,
    pub schema: String,
    pub pg_name: String,
    pub variants: Vec<VariantDef>,
    pub export_to: String,
}

impl EnumDef {
    /// Schema-qualified Postgres type name, e.g. `auth.user_role`.
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.pg_name)
    }
}

pub struct VariantDef {
    pub ident: syn::Ident,
    pub value: String,
}

impl EnumDef {
    pub fn parse(input: &ItemEnum) -> Self {
        let name = input.ident.clone();
        let config = EnumConfig::parse(&input.attrs);

        let variants = input
            .variants
            .iter()
            .map(|v| {
                let ident = v.ident.clone();
                let value = get_postgres_name(&v.attrs).unwrap_or_else(|| ident.to_string().to_snake_case());
                VariantDef { ident, value }
            })
            .collect();

        Self { name, schema: config.schema, pg_name: config.name, variants, export_to: config.export_to }
    }

    pub fn expected_values(&self) -> String {
        self.variants.iter().map(|v| v.value.as_str()).collect::<Vec<_>>().join(", ")
    }
}

struct EnumConfig {
    schema: String,
    name: String,
    export_to: String,
}

impl EnumConfig {
    fn parse(attrs: &[Attribute]) -> Self {
        for attr in attrs {
            if !attr.path().is_ident("enum_type") {
                continue;
            }

            let mut schema = None;
            let mut name = None;
            let mut export_to = None;

            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("schema")
                    && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
                {
                    schema = Some(s.value());
                } else if meta.path.is_ident("name")
                    && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
                {
                    name = Some(s.value());
                } else if meta.path.is_ident("export_to")
                    && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
                {
                    export_to = Some(s.value());
                }
                Ok(())
            });

            if let (Some(name), Some(export_to)) = (name, export_to) {
                return Self { schema: schema.unwrap_or_else(|| "public".to_string()), name, export_to };
            }
        }

        panic!("enum_type requires name and export_to attributes");
    }
}

fn get_postgres_name(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("postgres") {
            return None;
        }

        let mut name = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name")
                && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
            {
                name = Some(s.value());
            }
            Ok(())
        });

        name
    })
}
