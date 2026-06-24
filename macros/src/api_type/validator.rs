//! Emits a neutral [`ValidatorSchema`](orm::validator) from an `#[api_type]`
//! struct's `#[validate(...)]` rules. Rendering to a library (valibot, zod, ...)
//! happens at export time via a validator backend.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Fields, GenericArgument, Item, LitInt, LitStr, Meta, PathArguments, Token, Type,
    punctuated::Punctuated,
};

pub fn generate(input: &Item) -> TokenStream {
    let Item::Struct(item) = input else {
        return quote! {};
    };
    let Fields::Named(fields) = &item.fields else {
        return quote! {};
    };

    let name = item.ident.to_string();
    let field_defs = fields.named.iter().map(|field| {
        let field_name = field.ident.as_ref().expect("named field").to_string();
        let (base, optional, array) = base_type(&field.ty);
        let rules = rules(&field.attrs);

        quote! {
            ::orm::validator::Field {
                name: #field_name,
                base: #base,
                rules: &[ #(#rules),* ],
                optional: #optional,
                array: #array,
            }
        }
    });

    quote! {
        inventory::submit! {
            ::orm::validator::ValidatorSchema {
                name: #name,
                fields: &[ #(#field_defs),* ],
            }
        }
    }
}

/// `(BaseType expr, optional, array)` — `Option`/`Vec` set the flags and unwrap.
fn base_type(ty: &Type) -> (TokenStream, bool, bool) {
    if let Some(inner) = generic_inner(ty, "Option") {
        let (base, _, array) = base_type(&inner);
        return (base, true, array);
    }
    if let Some(inner) = generic_inner(ty, "Vec") {
        let (base, optional, _) = base_type(&inner);
        return (base, optional, true);
    }

    let base = match last_ident(ty).as_deref() {
        Some("String" | "str") => quote! { ::orm::validator::BaseType::String },
        Some("bool") => quote! { ::orm::validator::BaseType::Bool },
        Some(
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "usize" | "f32" | "f64",
        ) => quote! { ::orm::validator::BaseType::Number },
        Some("Uuid") => quote! { ::orm::validator::BaseType::Uuid },
        Some("DateTime") => quote! { ::orm::validator::BaseType::Timestamp },
        _ => quote! { ::orm::validator::BaseType::Unknown },
    };
    (base, false, false)
}

/// Every `Rule` expr from a field's `#[api(validate(...))]` attributes.
fn rules(attrs: &[Attribute]) -> Vec<TokenStream> {
    let mut rules = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("api") {
            continue;
        }
        let Ok(items) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
            continue;
        };

        for item in items {
            let Meta::List(validate) = item else { continue };
            if !validate.path.is_ident("validate") {
                continue;
            }
            let Ok(metas) = validate.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
                continue;
            };

            for meta in metas {
                match meta {
                    Meta::Path(p) if p.is_ident("email") => rules.push(quote! { ::orm::validator::Rule::Email }),
                    Meta::Path(p) if p.is_ident("url") => rules.push(quote! { ::orm::validator::Rule::Url }),
                    Meta::List(list) => list_rules(&list, &mut rules),
                    _ => {}
                }
            }
        }
    }
    rules
}

fn list_rules(list: &syn::MetaList, rules: &mut Vec<TokenStream>) {
    let key = list.path.get_ident().map(ToString::to_string).unwrap_or_default();

    if key == "regex" {
        if let Ok(lit) = list.parse_args::<LitStr>() {
            let pattern = lit.value();
            rules.push(quote! { ::orm::validator::Rule::Regex(#pattern) });
        }
        return;
    }
    if key != "length" && key != "range" {
        return;
    }

    let Ok(args) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return;
    };

    for arg in args {
        let Meta::List(inner) = &arg else { continue };
        let bound = inner.path.get_ident().map(ToString::to_string).unwrap_or_default();
        let Ok(n) = inner.parse_args::<LitInt>() else { continue };

        let rule = match (key.as_str(), bound.as_str()) {
            ("length", "min") => quote! { ::orm::validator::Rule::MinLength(#n) },
            ("length", "max") => quote! { ::orm::validator::Rule::MaxLength(#n) },
            ("length", "equal") => quote! { ::orm::validator::Rule::ExactLength(#n) },
            ("range", "min") => quote! { ::orm::validator::Rule::Min(#n) },
            ("range", "max") => quote! { ::orm::validator::Rule::Max(#n) },
            _ => continue,
        };
        rules.push(rule);
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
