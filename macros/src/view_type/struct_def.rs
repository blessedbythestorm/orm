use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemStruct;

use super::parse::ViewDef;

/// Re-emits the projection struct with the derives a read-model needs: row
/// deserialization (`FromRow` is generated separately), serde, and ts-rs. No
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

    quote! {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
        #[ts(export, export_to = #export_path, optional_fields)]
        #(#user_attrs)*
        #vis struct #name {
            #(#fields),*
        }
    }
}
