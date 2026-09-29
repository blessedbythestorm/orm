use orm::schema::{ConstraintKind, assemble_desired_schema};
use orm::{enum_type, table_type};
use uuid::Uuid;

#[enum_type(schema = "public", name = "schema_attribute_state", export_to = "types/schema_attributes.ts")]
pub enum SchemaAttributeState {
    Draft,
    #[postgres(name = "ready-for-use")]
    Ready,
}

#[table_type(schema = "public", name = "schema_attribute_probes", export_to = "types/schema_attributes.ts")]
#[pg(validate(is_null(ready_at) || state == SchemaAttributeState::Ready))]
#[pg(validate(
    present_iff(ready_at, state == SchemaAttributeState::Ready),
    name = "schema_attribute_probes_ready_at_check",
))]
#[pg(index(
    columns(owner_id, created_at),
    name = "schema_attribute_probes_owner_created_idx",
    where(state == SchemaAttributeState::Ready),
))]
pub struct SchemaAttributeProbe {
    #[pg(primary, default_value(gen_random_uuid()))]
    pub id: Uuid,
    pub owner_id: Uuid,
    #[pg(default_value(SchemaAttributeState::Draft))]
    pub state: SchemaAttributeState,
    pub ready_at: Option<String>,
    pub created_at: String,
}

#[test]
fn structured_schema_attributes_resolve_registered_enum_labels() {
    let schema = assemble_desired_schema();
    let table = schema.tables.get("public.schema_attribute_probes")
        .unwrap();

    assert_eq!(table.column("state").unwrap().default.as_deref(), Some("'draft'"));
    let constraint = table.constraint("schema_attribute_probes_ready_at_check")
        .unwrap();

    assert_eq!(
        constraint.kind,
        ConstraintKind::Check {
            expression: "(ready_at IS NOT NULL) = (state = 'ready-for-use')".into(),
        },
    );

    let index = table.index("schema_attribute_probes_owner_created_idx")
        .unwrap();

    assert_eq!(index.columns, ["owner_id", "created_at"]);
    assert_eq!(index.predicate.as_deref(), Some("state = 'ready-for-use'"));

    let inferred = table.constraints.iter()
        .find(|constraint| constraint.name != "schema_attribute_probes_ready_at_check")
        .unwrap();

    let ConstraintKind::Check { expression } = &inferred.kind else {
        panic!("expected a check constraint")
    };

    assert_eq!(expression, "ready_at IS NULL OR state = 'ready-for-use'");
    assert_eq!(
        inferred.name,
        stable_check_name(
            "schema_attribute_probes_ready_at_state",
            "ready_at IS NULL OR state = 'ready-for-use'",
        ),
    );
}

fn stable_check_name(prefix: &str, predicate: &str) -> String {
    let digest = predicate.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });

    format!("{prefix}_{digest:016x}_check")
}
