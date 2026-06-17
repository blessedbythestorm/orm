mod enum_def;
mod parse;
mod postgres;
mod registry;
mod schema;
mod serde;
mod typescript;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemEnum, parse2};

use parse::EnumDef;

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input: ItemEnum = match parse2(item) {
        Ok(e) => e,
        Err(e) => return e.to_compile_error(),
    };

    let enum_attr: syn::Attribute = syn::parse_quote!(#[enum_type(#attr)]);
    input.attrs.push(enum_attr);

    let def = EnumDef::parse(&input);

    let enum_def = enum_def::generate(&def, &input);
    let serde = serde::generate(&def);
    let typescript = typescript::generate(&def);
    let postgres = postgres::generate(&def);
    let registry = registry::generate(&def);
    let schema = schema::generate(&def);

    quote! {
        #enum_def
        #serde
        #typescript
        #postgres
        #registry
        #schema
    }
}
