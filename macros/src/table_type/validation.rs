//! Builds the synthetic field lists that let the generated Insert/Update
//! structs run through the same validation codegen as `#[api_type]`: the
//! original fields (keeping their `#[api(validate(...))]` rules), filtered to
//! the struct's field set, with types Option-wrapped where the struct relaxes
//! them. From these both the `Validate` impl and the exported schema are
//! generated, so client and server validation always match.

use proc_macro2::TokenStream;
use syn::{Field, Fields, FieldsNamed, Ident, ItemStruct, punctuated::Punctuated};

use super::parse::FieldDef;

/// The synthetic named fields for a derived struct: each `FieldDef` matched
/// back to its original field (for the `api` attrs), Option-wrapped when the
/// definition says so.
pub fn validation_fields<'a>(
    defs: impl Iterator<Item = &'a FieldDef>,
    input: &ItemStruct,
    wrap: impl Fn(&FieldDef) -> bool,
) -> Fields {
    let mut named: Punctuated<Field, syn::Token![,]> = Punctuated::new();

    for def in defs {
        let Some(original) = original_field(input, &def.name) else {
            continue;
        };

        let mut field = original.clone();
        field.attrs.retain(|attr| attr.path().is_ident("api"));

        if wrap(def) {
            let option_type = def.as_option_type();
            field.ty = syn::parse2(option_type).expect("option-wrapped type");
        }

        named.push(field);
    }

    Fields::Named(FieldsNamed { brace_token: Default::default(), named })
}

/// The `Validate` impl plus the registered validator schema for a derived
/// struct, generated from its synthetic fields.
pub fn validation(name: &Ident, fields: &Fields) -> TokenStream {
    let validate_impl =
        crate::api_type::validate::generate(name, &syn::Generics::default(), fields);

    let schema_source: syn::Item = syn::parse_quote! {
        pub struct #name #fields
    };
    let schema = crate::api_type::validator::generate(&schema_source);

    quote::quote! {
        #validate_impl
        #schema
    }
}

fn original_field<'a>(input: &'a ItemStruct, name: &Ident) -> Option<&'a Field> {
    input
        .fields
        .iter()
        .find(|field| field.ident.as_ref() == Some(name))
}
