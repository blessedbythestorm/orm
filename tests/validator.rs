use orm::lang::ts::Valibot;
use orm::validator::{BaseType, Field, Rule, ValidatorBackend, ValidatorSchema};

fn render(name: &'static str, fields: &'static [Field]) -> String {
    Valibot.schema(&ValidatorSchema { name, fields })
}

#[test]
fn pipes_a_base_type_with_its_rules() {
    let out = render(
        "Signup",
        &[
            Field { name: "email", base: BaseType::String, rules: &[Rule::Email], optional: false, array: false },
            Field {
                name: "age",
                base: BaseType::Number,
                rules: &[Rule::Min(0), Rule::Max(120)],
                optional: true,
                array: false,
            },
        ],
    );

    assert_eq!(
        out,
        "export const SignupSchema = v.object({ email: v.pipe(v.string(), v.email()), \
         age: v.optional(v.pipe(v.number(), v.minValue(0), v.maxValue(120))) });"
    );
}

#[test]
fn uuid_and_timestamp_carry_intrinsic_rules() {
    let out = render(
        "Row",
        &[
            Field { name: "id", base: BaseType::Uuid, rules: &[], optional: false, array: false },
            Field { name: "at", base: BaseType::Timestamp, rules: &[], optional: false, array: false },
        ],
    );

    assert_eq!(
        out,
        "export const RowSchema = v.object({ id: v.pipe(v.string(), v.uuid()), \
         at: v.pipe(v.string(), v.isoTimestamp()) });"
    );
}

#[test]
fn array_wraps_the_base_type() {
    let out = render(
        "Post",
        &[Field { name: "tags", base: BaseType::String, rules: &[], optional: false, array: true }],
    );

    assert_eq!(out, "export const PostSchema = v.object({ tags: v.array(v.string()) });");
}

#[test]
fn regex_rule_escapes_into_a_js_regexp() {
    let out = render(
        "Contact",
        &[Field {
            name: "phone",
            base: BaseType::String,
            rules: &[Rule::Regex(r"^\+\d+$")],
            optional: false,
            array: false,
        }],
    );

    assert_eq!(
        out,
        "export const ContactSchema = v.object({ phone: v.pipe(v.string(), v.regex(new RegExp(\"^\\\\+\\\\d+$\"))) });"
    );
}

inventory::submit! {
    orm::export::ExportType {
        name: "TestStatus",
        path: "types/test.ts",
        docs: &[],
        shape: orm::export::Shape::Enum(&["live", "ended"]),
    }
}

#[test]
fn named_enum_renders_as_a_picklist_of_its_wire_values() {
    let out = render(
        "SetStatus",
        &[Field { name: "status", base: BaseType::Named("TestStatus"), rules: &[], optional: false, array: false }],
    );

    assert!(out.contains("status: v.picklist([\"live\", \"ended\"])"), "{out}");
}

#[test]
fn unregistered_named_type_stays_unknown() {
    let out = render(
        "SetThing",
        &[Field { name: "thing", base: BaseType::Named("NotRegistered"), rules: &[], optional: true, array: false }],
    );

    assert!(out.contains("thing: v.optional(v.unknown())"), "{out}");
}
