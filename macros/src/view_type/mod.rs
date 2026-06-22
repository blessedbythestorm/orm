mod crud;
mod from_row;
mod parse;
mod registry;
mod schema;
mod struct_def;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, parse2};

use parse::ViewDef;

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input: ItemStruct = match parse2(item) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error(),
    };

    let view_attr: syn::Attribute = syn::parse_quote!(#[view_type(#attr)]);
    input.attrs.push(view_attr);

    let view = ViewDef::parse(&input);

    let struct_def = struct_def::generate(&view, &input);
    let from_row = from_row::generate(&view);
    let crud = crud::generate(&view);
    let registry = registry::generate(&view);
    let schema = schema::generate(&view);

    quote! {
        #struct_def
        #from_row
        #crud
        #registry
        #schema
    }
}
