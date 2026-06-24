use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemStruct;

use super::parse::ViewDef;

/// Re-emits the projection struct with the derives a read-model needs: row
/// deserialization (`FromRow` is generated separately), serde, and TypeScript export. No
/// `Validate` — a view is query output, never validated input.
pub fn generate(view: &ViewDef, input: &ItemStruct) -> TokenStream {
    let vis = &input.vis;
    let name = &view.name;
    let export_path = view.export_path();

    let user_attrs: Vec<_> = input.attrs.iter().filter(|a| !a.path().is_ident("view_type")).collect();

    // Drop the `#[pg(view(...))]` source markers — they're metadata for the macro,
    // not part of the emitted struct.
    let fields: Vec<_> = input
        .fields
        .iter()
        .map(|f| {
            let mut field = f.clone();
            field.attrs.retain(|a| !a.path().is_ident("pg"));
            field
        })
        .collect();

    let doc = crate::export::doc_lines(&input.attrs);
    let ts_export = crate::export::struct_export(&name.to_string(), export_path, &doc, &crate::export::fields_from(&input.fields));

    quote! {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        #(#user_attrs)*
        #vis struct #name {
            #(#fields),*
        }

        #ts_export
    }
}
