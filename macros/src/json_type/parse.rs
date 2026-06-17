use syn::{Attribute, ItemStruct, Lit};

pub struct JsonDef {
    pub name: syn::Ident,
    pub export_to: String,
}

impl JsonDef {
    pub fn parse(input: &ItemStruct) -> Self {
        let name = input.ident.clone();
        let export_to = parse_export_to(&input.attrs);

        Self { name, export_to }
    }
}

fn parse_export_to(attrs: &[Attribute]) -> String {
    for attr in attrs {
        if !attr.path().is_ident("json_type") {
            continue;
        }

        let mut export_to = None;

        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("export_to")
                && let Ok(Lit::Str(s)) = meta.value()?.parse::<Lit>()
            {
                export_to = Some(s.value());
            }
            Ok(())
        });

        if let Some(path) = export_to {
            return path;
        }
    }

    panic!("json_type requires export_to attribute");
}
