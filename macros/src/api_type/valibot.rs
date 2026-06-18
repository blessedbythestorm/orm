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
    Attribute, Expr, Fields, GenericArgument, Item, Lit, Meta, MetaNameValue, PathArguments, Token,
    Type, punctuated::Punctuated,
};

/// A `#[validate]` rule that maps to a valibot pipe item.
enum Rule {
    /// Fully known at macro time, e.g. `v.email()` / `v.minLength(2)`.
    Static(String),
    /// `regex(path = <expr>)` — the pattern is read from the `Regex` at runtime.
    Regex(Expr),
}

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
    let mut args = Vec::new();
    for rule in rules {
        match rule {
            Rule::Static(item) => pipe.push(item.clone()),
            Rule::Regex(path) => {
                pipe.push("v.regex(new RegExp({}))".to_string());
                // serde_json gives a JS-safe, correctly-escaped string literal.
                args.push(quote! { ::serde_json::to_string((#path).as_str()).unwrap() });
            }
        }
    }

    let schema = if pipe.is_empty() { base } else { format!("v.pipe({base}, {})", pipe.join(", ")) };
    (schema, args)
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

/// Rules from a field's `#[validate(...)]` attributes.
fn validate_rules(attrs: &[Attribute]) -> Vec<Rule> {
    let mut rules = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("validate") {
            continue;
        }
        let Ok(metas) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
            continue;
        };
        for meta in metas {
            match meta {
                Meta::Path(path) if path.is_ident("email") => rules.push(Rule::Static("v.email()".into())),
                Meta::Path(path) if path.is_ident("url") => rules.push(Rule::Static("v.url()".into())),
                Meta::List(list) => collect_list_rules(&list, &mut rules),
                _ => {}
            }
        }
    }
    rules
}

fn collect_list_rules(list: &syn::MetaList, rules: &mut Vec<Rule>) {
    let key = list.path.get_ident().map(ToString::to_string).unwrap_or_default();

    if key == "regex" {
        if let Ok(pairs) = list.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated) {
            for pair in pairs {
                if pair.path.is_ident("path") {
                    rules.push(Rule::Regex(pair.value));
                }
            }
        }
        return;
    }

    if key != "length" && key != "range" {
        return;
    }
    let Ok(pairs) = list.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated) else {
        return;
    };
    for pair in pairs {
        let field = pair.path.get_ident().map(ToString::to_string).unwrap_or_default();
        let Some(n) = int_value(&pair.value) else { continue };
        match (key.as_str(), field.as_str()) {
            ("length", "min") => rules.push(Rule::Static(format!("v.minLength({n})"))),
            ("length", "max") => rules.push(Rule::Static(format!("v.maxLength({n})"))),
            ("length", "equal") => rules.push(Rule::Static(format!("v.length({n})"))),
            ("range", "min") => rules.push(Rule::Static(format!("v.minValue({n})"))),
            ("range", "max") => rules.push(Rule::Static(format!("v.maxValue({n})"))),
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

fn int_value(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Int(int) => int.base10_parse().ok(),
            _ => None,
        },
        _ => None,
    }
}
