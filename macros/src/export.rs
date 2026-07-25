//! Macro-time mapping of Rust types into the language-neutral export model
//! ([`orm::export`]). Each `#[*_type]` macro registers an `ExportType`; the
//! actual language rendering happens at export time via an `ExportBackend`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Expr, ExprLit, GenericArgument, Lit, Meta, PathArguments, Type};

/// One struct field as the export model needs it.
pub struct Field {
    pub name: String,
    pub ty: Type,
    /// Render optional even if the Rust type isn't `Option` (e.g. a db-defaulted
    /// insert column).
    pub forced_optional: bool,
}

/// Register a struct/object export type.
pub fn struct_export(name: &str, path: &str, docs: &[String], fields: &[Field]) -> TokenStream {
    let field_toks = fields.iter().map(|f| {
        let (ty, is_option) = field_type(&f.ty);
        let optional = is_option || f.forced_optional;
        let fname = &f.name;
        quote! { ::orm::export::Field { name: #fname, ty: #ty, optional: #optional } }
    });
    submit(name, path, docs, quote! { ::orm::export::Shape::Struct(&[ #(#field_toks),* ]) })
}

/// Register a string-union export type for an enum.
pub fn enum_export(name: &str, path: &str, docs: &[String], variants: &[String]) -> TokenStream {
    submit(name, path, docs, quote! { ::orm::export::Shape::Enum(&[ #(#variants),* ]) })
}

fn submit(name: &str, path: &str, docs: &[String], shape: TokenStream) -> TokenStream {
    quote! {
        inventory::submit! {
            ::orm::export::ExportType {
                name: #name,
                path: #path,
                docs: &[ #(#docs),* ],
                shape: #shape,
            }
        }
    }
}

/// The neutral [`FieldType`](orm::export::FieldType) const-expression for a Rust
/// type, plus whether it was wrapped in `Option`. `Option`/`Vec` are unwrapped.
pub fn field_type(ty: &Type) -> (TokenStream, bool) {
    if let Some(inner) = generic_inner(ty, "Option") {
        return (field_type(&inner).0, true);
    }
    if let Some(inner) = generic_inner(ty, "Vec") {
        let (inner_ty, _) = field_type(&inner);
        return (quote! { &::orm::export::FieldType::Array(#inner_ty) }, false);
    }

    // A map's value type is what crosses the boundary; the key is a string by
    // construction, since that is all serde can write as a JSON object key.
    for map in ["BTreeMap", "HashMap"] {
        if let Some(value) = generic_value(ty, map) {
            let (value_ty, _) = field_type(&value);
            return (quote! { &::orm::export::FieldType::Map(#value_ty) }, false);
        }
    }

    let ident = last_ident(ty);
    let variant = match ident.as_deref() {
        Some("String" | "str") => quote! { &::orm::export::FieldType::String },
        Some("bool") => quote! { &::orm::export::FieldType::Bool },
        Some(
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "usize" | "f32" | "f64",
        ) => quote! { &::orm::export::FieldType::Number },
        Some("Uuid") => quote! { &::orm::export::FieldType::Uuid },
        Some("DateTime") => quote! { &::orm::export::FieldType::Timestamp },
        Some("Value") => quote! { &::orm::export::FieldType::Json },
        Some(other) => quote! { &::orm::export::FieldType::Named(#other) },
        None => quote! { &::orm::export::FieldType::Json },
    };
    (variant, false)
}

/// The second generic argument of `Wrapper<K, V>`, if `ty` is one.
fn generic_value(ty: &Type, wrapper: &str) -> Option<Type> {
    let Type::Path(path) = ty else {
        return None;
    };

    let segment = path.path.segments.last()?;

    if segment.ident != wrapper {
        return None;
    }

    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };

    args.args
        .iter()
        .filter_map(|arg| match arg {
            syn::GenericArgument::Type(ty) => Some(ty.clone()),
            _ => None,
        })
        .nth(1)
}

/// The named fields of a struct as [`Field`]s (none forced optional).
pub fn fields_from(fields: &syn::Fields) -> Vec<Field> {
    match fields {
        syn::Fields::Named(named) => named
            .named
            .iter()
            .map(|f| Field {
                name: f.ident.as_ref().expect("named field").to_string(),
                ty: f.ty.clone(),
                forced_optional: false,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `///` doc lines with their leading space trimmed, for the backend to format.
pub fn doc_lines(attrs: &[Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(doc_line)
        .map(|line| line.strip_prefix(' ').map(str::to_string).unwrap_or(line))
        .collect()
}

fn doc_line(attr: &Attribute) -> Option<String> {
    if !attr.path().is_ident("doc") {
        return None;
    }
    let Meta::NameValue(nv) = &attr.meta else { return None };
    let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = &nv.value else { return None };
    Some(s.value())
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
