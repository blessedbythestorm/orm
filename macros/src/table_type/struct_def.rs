use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemStruct;

use super::parse::TableDef;

pub fn generate(table: &TableDef, input: &ItemStruct) -> TokenStream {
    let vis = &input.vis;
    let name = &table.name;
    let export_path = table.export_path();

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("table_type")).collect();

    let helper_attrs = ["pg", "crud", "api"];
    let fields: Vec<_> = input
        .fields
        .iter()
        .map(|f| {
            let mut field = f.clone();
            field.attrs.retain(|a| !helper_attrs.iter().any(|helper| a.path().is_ident(helper)));
            field
        })
        .collect();

    let validate_impl = crate::api_type::validate::generate(name, &input.generics, &input.fields);

    let doc = crate::export::doc_lines(&input.attrs);
    let ts_fields: Vec<crate::export::Field> = table
        .fields
        .iter()
        .map(|f| crate::export::Field { name: f.name_str.clone(), ty: f.ty.clone(), forced_optional: false })
        .collect();
    let ts_export = crate::export::struct_export(&name.to_string(), export_path, &doc, &ts_fields);

    quote! {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        #(#user_attrs)*
        #vis struct #name {
            #(#fields),*
        }

        #validate_impl
        #ts_export
    }
}
