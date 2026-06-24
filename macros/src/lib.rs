mod api_type;
mod endpoint;
mod enum_type;
mod json_type;
mod table_type;
mod export;
mod view_type;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn enum_type(attr: TokenStream, item: TokenStream) -> TokenStream {
    enum_type::expand(attr.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn json_type(attr: TokenStream, item: TokenStream) -> TokenStream {
    json_type::expand(attr.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn table_type(attr: TokenStream, item: TokenStream) -> TokenStream {
    table_type::expand(attr.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn view_type(attr: TokenStream, item: TokenStream) -> TokenStream {
    view_type::expand(attr.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn api_type(attr: TokenStream, item: TokenStream) -> TokenStream {
    api_type::expand(attr.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn endpoint(attr: TokenStream, item: TokenStream) -> TokenStream {
    endpoint::expand(attr.into(), item.into()).into()
}
