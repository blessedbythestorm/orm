use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub fn expand(input: TokenStream) -> TokenStream {
    let input = match syn::parse2::<DeriveInput>(input) {
        Ok(input) => input,
        Err(error) => return error.to_compile_error(),
    };

    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input, "FromRow requires a named-field struct")
            .to_compile_error();
    };

    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(&input, "FromRow requires named fields")
            .to_compile_error();
    };

    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let assignments = fields.named.iter()
        .map(|field| {
            let name = field.ident.as_ref()
                .unwrap();

            let column = name.to_string()
                .trim_start_matches("r#")
                .to_string();

            quote! { #name: row.try_get(#column)? }
        });

    quote! {
        impl #impl_generics ::orm::FromRow for #name #type_generics #where_clause {
            fn from_row(row: &::orm::__private::Row) -> ::std::result::Result<Self, ::orm::__private::PostgresError> {
                Ok(Self { #(#assignments),* })
            }
        }
    }
}
