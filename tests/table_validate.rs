//! The generated Insert/Update structs must enforce the table's
//! `#[api(validate(...))]` rules and register exported schemas, exactly like
//! `#[api_type]` request types do.

use chrono::{DateTime, Utc};
use orm::table_type;
use orm::validate::Validate;
use uuid::Uuid;

#[table_type(schema = "public", name = "gadgets", export_to = "types/gadgets.ts")]
pub struct Gadget {
    #[pg(primary, default(sql("gen_random_uuid()")))]
    pub id: Uuid,
    #[api(validate(length(min(3), max(30))))]
    pub name: String,
    #[api(validate(email))]
    pub contact: Option<String>,
    #[pg(default(sql("now()")))]
    #[crud(insert(optional), update(skip))]
    pub created_at: DateTime<Utc>,
}

#[test]
fn insert_enforces_the_table_rules() {
    let bad = GadgetInsert { id: None, name: "ab".into(), contact: None, created_at: None };
    assert!(bad.validate().is_err());

    let good = GadgetInsert { id: None, name: "abc".into(), contact: None, created_at: None };
    assert!(good.validate().is_ok());
}

#[test]
fn insert_checks_optional_fields_only_when_present() {
    let bad = GadgetInsert {
        id: None,
        name: "abc".into(),
        contact: Some("not-an-email".into()),
        created_at: None,
    };
    assert!(bad.validate().is_err());
}

#[test]
fn update_checks_only_provided_fields() {
    let empty = GadgetUpdate { name: None, contact: None };
    assert!(empty.validate().is_ok());

    let bad = GadgetUpdate { name: Some("ab".into()), contact: None };
    assert!(bad.validate().is_err());
}

#[test]
fn insert_and_update_schemas_are_registered() {
    let names: Vec<&str> = inventory::iter::<orm::validator::ValidatorSchema>
        .into_iter()
        .map(|schema| schema.name)
        .collect();

    assert!(names.contains(&"GadgetInsert"), "{names:?}");
    assert!(names.contains(&"GadgetUpdate"), "{names:?}");
}
