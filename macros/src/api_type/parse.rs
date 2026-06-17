use syn::{Attribute, Item, Lit};

pub struct ApiDef {
    pub name: syn::Ident,
    pub export_to: String,
}

impl ApiDef {
    pub fn parse(input: &Item) -> Self {
        let (name, attrs) = match input {
            Item::Struct(s) => (s.ident.clone(), &s.attrs),
            Item::Enum(e) => (e.ident.clone(), &e.attrs),
            _ => panic!("api_type only supports structs and enums"),
        };

        let export_to = parse_export_to(attrs);

        Self { name, export_to }
    }
}

fn parse_export_to(attrs: &[Attribute]) -> String {
    for attr in attrs {
        if !attr.path().is_ident("api_type") {
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

    panic!("api_type requires export_to attribute");
}
