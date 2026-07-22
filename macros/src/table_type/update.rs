use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use syn::ItemStruct;

use super::parse::TableDef;
use super::validation;

pub fn generate(table: &TableDef, input: &ItemStruct) -> TokenStream {
    let name = format_ident!("{}Update", table.name);
    let export_path = table.export_path();
    let fields = generate_fields(table);

    let ts_fields: Vec<crate::export::Field> = table
        .update_fields()
        .map(|f| crate::export::Field { name: f.name_str.clone(), ty: f.ty.clone(), forced_optional: true })
        .collect();
    let ts_export = crate::export::struct_export(&name.to_string(), export_path, &[], &ts_fields);

    let rule_fields = validation::validation_fields(table.update_fields(), input, |_| true);
    let validation = validation::validation(&name, &rule_fields);

    quote! {
        #ts_export

        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        pub struct #name {
            #(#fields),*
        }

        #validation
    }
}

fn generate_fields(table: &TableDef) -> Vec<TokenStream> {
    table
        .update_fields()
        .map(|f| {
            let name = &f.name;
            let ty = f.as_option_type();

            quote! {
                pub #name: #ty
            }
        })
        .collect()
}
