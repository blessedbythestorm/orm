use orm::export::{ExportBackend, Field, FieldType, relative_import};
use orm::lang::ts::TypeScript;

#[test]
fn relative_import_walks_between_directories() {
    assert_eq!(relative_import("api/x.ts", "database/y.ts"), "../database/y");
    assert_eq!(relative_import("api/x.ts", "api/y.ts"), "./y");
    assert_eq!(relative_import("api/sub/x.ts", "database/y.ts"), "../../database/y");
}

#[test]
fn type_expr_maps_scalars_and_composites() {
    let ts = TypeScript;

    assert_eq!(ts.type_expr(&FieldType::String), "string");
    assert_eq!(ts.type_expr(&FieldType::Number), "number");
    assert_eq!(ts.type_expr(&FieldType::Bool), "boolean");
    assert_eq!(ts.type_expr(&FieldType::Uuid), "string");
    assert_eq!(ts.type_expr(&FieldType::Timestamp), "string");
    assert_eq!(ts.type_expr(&FieldType::Json), "unknown");
    assert_eq!(ts.type_expr(&FieldType::Named("Booking")), "Booking");
    assert_eq!(ts.type_expr(&FieldType::Array(&FieldType::String)), "Array<string>");
}

#[test]
fn struct_decl_renders_fields_and_optionals() {
    let fields = [
        Field { name: "id", ty: &FieldType::Uuid, optional: false },
        Field { name: "note", ty: &FieldType::String, optional: true },
    ];

    assert_eq!(
        TypeScript.struct_decl("Doc", "", &fields),
        "export type Doc = { id: string, note?: string, };"
    );
}

#[test]
fn enum_decl_renders_a_string_union() {
    assert_eq!(
        TypeScript.enum_decl("Status", "", &["live", "ended"]),
        "export type Status = \"live\" | \"ended\";"
    );
}

#[test]
fn doc_block_wraps_lines_or_is_empty() {
    assert_eq!(TypeScript.doc(&[]), "");
    assert_eq!(TypeScript.doc(&["A note."]), "/**\n * A note.\n */\n");
}

#[test]
fn import_lists_names_from_a_module() {
    assert_eq!(
        TypeScript.import("../database/live", &["LiveSession", "SessionStatus"]),
        "import type { LiveSession, SessionStatus } from \"../database/live\";"
    );
}
