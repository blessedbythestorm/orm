mod item_def;
mod parse;
mod registry;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Item, parse2};

use parse::ApiDef;

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input: Item = match parse2(item) {
        Ok(i) => i,
        Err(e) => return e.to_compile_error(),
    };

    let api_attr: syn::Attribute = syn::parse_quote!(#[api_type(#attr)]);
    match &mut input {
        Item::Struct(s) => s.attrs.push(api_attr),
        Item::Enum(e) => e.attrs.push(api_attr),
        _ => {
            return syn::Error::new_spanned(&input, "api_type only supports structs and enums")
                .to_compile_error();
        }
    }

    let def = ApiDef::parse(&input);

    let item_def = item_def::generate(&def, &input);
    let registry = registry::generate(&def);

    quote! {
        #item_def
        #registry
    }
}
