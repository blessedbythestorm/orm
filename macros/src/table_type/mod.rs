mod crud;
mod from_row;
mod insert;
mod parse;
mod registry;
mod schema;
mod struct_def;
mod update;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, parse2};

use parse::TableDef;

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input: ItemStruct = match parse2(item) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error(),
    };

    let table_attr: syn::Attribute = syn::parse_quote!(#[table_type(#attr)]);
    input.attrs.push(table_attr);

    let table = TableDef::parse(&input);

    let struct_def = struct_def::generate(&table, &input);
    let from_row = from_row::generate(&table);
    let insert = insert::generate(&table);
    let update = update::generate(&table);
    let crud = crud::generate(&table);
    let registry = registry::generate(&table);
    let schema = schema::generate(&table);

    quote! {
        #struct_def
        #from_row
        #insert
        #update
        #crud
        #registry
        #schema
    }
}
