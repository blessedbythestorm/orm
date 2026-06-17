mod parse;
mod postgres;
mod registry;
mod schema;
mod struct_def;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, parse2};

use parse::JsonDef;

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input: ItemStruct = match parse2(item) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error(),
    };

    let json_attr: syn::Attribute = syn::parse_quote!(#[json_type(#attr)]);
    input.attrs.push(json_attr);

    let def = JsonDef::parse(&input);

    let struct_def = struct_def::generate(&def, &input);
    let postgres = postgres::generate(&def);
    let registry = registry::generate(&def);
    let schema = schema::generate(&def);

    quote! {
        #struct_def
        #postgres
        #registry
        #schema
    }
}
