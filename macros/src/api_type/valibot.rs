//! Builds a valibot schema for an `#[api_type]` struct from its field types and
//! `#[validate(...)]` rules, registered into the `ValibotSchema` inventory.
//!
//! The schema is produced as a `fn() -> String` evaluated at export time rather
//! than a compile-time literal. That lets `regex` rules emit the *actual*
//! pattern: the macro only sees `path = *REGEX_E164`, but it can emit
//! `(*REGEX_E164).as_str()`, which the exporter reads from the compiled `Regex`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Fields, GenericArgument, Item, LitInt, LitStr, Meta, PathArguments, Token, Type,
    punctuated::Punctuated,
};

/// A `#[api(validate(...))]` rule as a fully-resolved valibot pipe item (every
/// rule — including an inline `regex(r"...")` — is known at macro time).
struct Rule(String);

pub fn generate(input: &Item) -> TokenStream {
    let Item::Struct(item) = input else {
        return quote! {};
    };
    let Fields::Named(fields) = &item.fields else {
        return quote! {};
    };

    let name = item.ident.to_string();

    // Each field contributes a template fragment (with `{}` placeholders for any
    // runtime regex patterns) plus the matching argument expressions.
    let mut field_templates: Vec<String> = Vec::new();
    let mut args: Vec<TokenStream> = Vec::new();
    for field in &fields.named {
        let field_name = field.ident.as_ref().expect("named field").to_string();
        let (schema, field_args) = field_schema(&field.ty, &validate_rules(&field.attrs));
        field_templates.push(format!("{field_name}: {schema}"));
        args.extend(field_args);
    }

    // Built by hand (not `format!`) so the `{}`/`{{`/`}}` survive into the
    // runtime `format!` the build closure performs.
    let template = String::from("v.object({{ ") + &field_templates.join(", ") + " }})";

    quote! {
        inventory::submit! {
            ::orm::registry::ValibotSchema {
                name: #name,
                build: || format!(#template, #(#args),*),
            }
        }
    }
}

/// The valibot expression for a field type plus its runtime argument exprs,
/// applying `#[validate]` rules to the underlying scalar (through `Option`/`Vec`).
fn field_schema(ty: &Type, rules: &[Rule]) -> (String, Vec<TokenStream>) {
    if let Some(inner) = generic_inner(ty, "Option") {
        let (schema, args) = field_schema(&inner, rules);
        return (format!("v.optional({schema})"), args);
    }
    if let Some(inner) = generic_inner(ty, "Vec") {
        let (schema, args) = field_schema(&inner, &[]);
        return (format!("v.array({schema})"), args);
    }

    let (base, mut pipe) = scalar(ty);
    for rule in rules {
        pipe.push(rule.0.clone());
    }

    let schema = if pipe.is_empty() { base } else { format!("v.pipe({base}, {})", pipe.join(", ")) };
    (schema, Vec::new())
}

/// `(base valibot type, intrinsic pipe items)` for a scalar Rust type.
fn scalar(ty: &Type) -> (String, Vec<String>) {
    match last_ident(ty).as_deref() {
        Some("String" | "str") => ("v.string()".into(), vec![]),
        Some("bool") => ("v.boolean()".into(), vec![]),
        Some(
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "usize" | "f32" | "f64",
        ) => ("v.number()".into(), vec![]),
        Some("Uuid") => ("v.string()".into(), vec!["v.uuid()".into()]),
        Some("DateTime") => ("v.string()".into(), vec!["v.isoTimestamp()".into()]),
        // serde_json::Value and anything unrecognized (nested types / enums) stay
        // permissive for now; nested-schema references are a future enhancement.
        _ => ("v.unknown()".into(), vec![]),
    }
}

/// Rules from a field's `#[api(validate(...))]` attributes.
fn validate_rules(attrs: &[Attribute]) -> Vec<Rule> {
    let mut rules = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("api") {
            continue;
        }
        // `#[api(validate(...))]` — unwrap each `validate(...)` list, then map its rules.
        let Ok(items) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
            continue;
        };
        for item in items {
            let Meta::List(validate) = item else { continue };
            if !validate.path.is_ident("validate") {
                continue;
            }
            let Ok(metas) = validate.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                continue;
            };
            for meta in metas {
                match meta {
                    Meta::Path(path) if path.is_ident("email") => rules.push(Rule("v.email()".into())),
                    Meta::Path(path) if path.is_ident("url") => rules.push(Rule("v.url()".into())),
                    Meta::List(list) => collect_list_rules(&list, &mut rules),
                    _ => {}
                }
            }
        }
    }
    rules
}

fn collect_list_rules(list: &syn::MetaList, rules: &mut Vec<Rule>) {
    let key = list.path.get_ident().map(ToString::to_string).unwrap_or_default();

    if key == "regex" {
        // `regex(r"...")` — bake the inline pattern as a JS RegExp. Escape `{`/`}`
        // so the pattern survives the runtime `format!` that assembles the schema.
        if let Ok(lit) = list.parse_args::<LitStr>() {
            let js = lit.value().replace('\\', "\\\\").replace('"', "\\\"");
            let item = format!("v.regex(new RegExp(\"{js}\"))").replace('{', "{{").replace('}', "}}");
            rules.push(Rule(item));
        }
        return;
    }

    if key != "length" && key != "range" {
        return;
    }
    // Call syntax: `length(min(3), max(30))` — each arg is `name(value)`.
    let Ok(args) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return;
    };
    for arg in args {
        let Meta::List(inner) = &arg else { continue };
        let field = inner.path.get_ident().map(ToString::to_string).unwrap_or_default();
        let Ok(value) = inner.parse_args::<LitInt>() else { continue };
        let n = value.base10_digits();
        match (key.as_str(), field.as_str()) {
            ("length", "min") => rules.push(Rule(format!("v.minLength({n})"))),
            ("length", "max") => rules.push(Rule(format!("v.maxLength({n})"))),
            ("length", "equal") => rules.push(Rule(format!("v.length({n})"))),
            ("range", "min") => rules.push(Rule(format!("v.minValue({n})"))),
            ("range", "max") => rules.push(Rule(format!("v.maxValue({n})"))),
            _ => {}
        }
    }
}

fn generic_inner(ty: &Type, wrapper: &str) -> Option<Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else { return None };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
}

fn last_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}
