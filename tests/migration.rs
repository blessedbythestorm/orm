use orm::schema::{
    Column, Constraint, ConstraintKind, DatabaseSchema, EnumType, ForeignKey, Index, NoRenames,
    ReferentialAction, Table, diff, invert, render,
};

fn col(name: &str, sql_type: &str) -> Column {
    Column {
        name: name.into(),
        sql_type: sql_type.into(),
        nullable: false,
        primary_key: false,
        unique: false,
        default: None,
        foreign_key: None,
    }
}

fn table(name: &str, columns: Vec<Column>) -> Table {
    Table { schema: "public".into(), name: name.into(), columns, ..Default::default() }
}

fn unique(name: &str, columns: &[&str]) -> Constraint {
    Constraint {
        name: name.into(),
        kind: ConstraintKind::Unique { columns: columns.iter().map(|c| c.to_string()).collect() },
    }
}

fn check(name: &str, expression: &str) -> Constraint {
    Constraint { name: name.into(), kind: ConstraintKind::Check { expression: expression.into() } }
}

fn schema(tables: Vec<Table>) -> DatabaseSchema {
    let tables = tables.into_iter().map(|t| (t.qualified_name(), t)).collect();
    DatabaseSchema { tables, ..Default::default() }
}

fn up(baseline: &DatabaseSchema, desired: &DatabaseSchema) -> String {
    render(&diff(baseline, desired, &mut NoRenames))
}

#[test]
fn no_changes_when_schemas_match() {
    let s = schema(vec![table("widgets", vec![col("id", "uuid")])]);
    assert!(diff(&s, &s, &mut NoRenames).is_empty());
}

#[test]
fn create_table_renders_columns_and_primary_key() {
    let desired = schema(vec![table(
        "widgets",
        vec![Column { primary_key: true, ..col("id", "uuid") }, col("name", "text")],
    )]);
    let sql = up(&DatabaseSchema::default(), &desired);

    assert!(sql.contains("CREATE TABLE public.widgets ("), "{sql}");
    assert!(sql.contains("id uuid NOT NULL"), "{sql}");
    assert!(sql.contains("name text NOT NULL"), "{sql}");
    assert!(sql.contains("PRIMARY KEY (id)"), "{sql}");
}

#[test]
fn create_table_renders_a_foreign_key() {
    let user_id = Column {
        foreign_key: Some(ForeignKey {
            schema: "auth".into(),
            table: "users".into(),
            column: "id".into(),
            on_update: ReferentialAction::NoAction,
            on_delete: ReferentialAction::Cascade,
        }),
        ..col("user_id", "uuid")
    };
    let sql = up(&DatabaseSchema::default(), &schema(vec![table("sessions", vec![user_id])]));

    assert!(
        sql.contains("user_id uuid NOT NULL REFERENCES auth.users (id) ON UPDATE NO ACTION ON DELETE CASCADE"),
        "{sql}"
    );
}

#[test]
fn adding_a_column_alters_the_table() {
    let baseline = schema(vec![table("widgets", vec![col("id", "uuid")])]);
    let desired = schema(vec![table("widgets", vec![col("id", "uuid"), col("name", "text")])]);

    assert_eq!(up(&baseline, &desired).trim(), "ALTER TABLE public.widgets ADD COLUMN name text NOT NULL;");
}

#[test]
fn dropping_a_column_alters_the_table() {
    let baseline = schema(vec![table("widgets", vec![col("id", "uuid"), col("name", "text")])]);
    let desired = schema(vec![table("widgets", vec![col("id", "uuid")])]);

    assert_eq!(up(&baseline, &desired).trim(), "ALTER TABLE public.widgets DROP COLUMN name;");
}

#[test]
fn creating_an_enum_renders_create_type() {
    let mut desired = DatabaseSchema::default();
    desired.enums.insert(
        "public.status".into(),
        EnumType { name: "public.status".into(), values: vec!["live".into(), "ended".into()] },
    );

    let sql = up(&DatabaseSchema::default(), &desired);
    assert!(sql.contains("CREATE TYPE public.status AS ENUM ('live', 'ended');"), "{sql}");
}

#[test]
fn invert_turns_create_table_into_drop() {
    let baseline = DatabaseSchema::default();
    let desired = schema(vec![table("widgets", vec![col("id", "uuid")])]);
    let changes = diff(&baseline, &desired, &mut NoRenames);

    let down = render(&invert(&changes, &baseline));
    assert!(down.contains("DROP TABLE public.widgets CASCADE;"), "{down}");
}

#[test]
fn invert_turns_add_column_into_drop_column() {
    let baseline = schema(vec![table("widgets", vec![col("id", "uuid")])]);
    let desired = schema(vec![table("widgets", vec![col("id", "uuid"), col("name", "text")])]);
    let changes = diff(&baseline, &desired, &mut NoRenames);

    let down = render(&invert(&changes, &baseline));
    assert_eq!(down.trim(), "ALTER TABLE public.widgets DROP COLUMN name;");
}

fn status_enum(values: &[&str]) -> EnumType {
    EnumType { name: "public.status".into(), values: values.iter().map(|v| v.to_string()).collect() }
}

fn schema_with_enum(values: &[&str], tables: Vec<Table>) -> DatabaseSchema {
    let mut database = schema(tables);
    let status = status_enum(values);
    database.enums.insert(status.name.clone(), status);
    database
}

fn status_col() -> Column {
    col("status", "public.status")
}

#[test]
fn extending_an_enum_recreates_the_type() {
    let baseline = schema_with_enum(&["live", "ended"], vec![table("widgets", vec![status_col()])]);
    let desired = schema_with_enum(&["live", "ended", "paused"], vec![table("widgets", vec![status_col()])]);

    let sql = up(&baseline, &desired);
    assert!(sql.contains("ALTER TYPE public.status RENAME TO status_old;"), "{sql}");
    assert!(sql.contains("CREATE TYPE public.status AS ENUM ('live', 'ended', 'paused');"), "{sql}");
    assert!(
        sql.contains("ALTER TABLE public.widgets ALTER COLUMN status TYPE public.status USING status::text::public.status;"),
        "{sql}"
    );
    assert!(sql.contains("DROP TYPE public.status_old;"), "{sql}");
}

#[test]
fn removing_a_value_falls_back_to_the_column_default() {
    let with_default = Column { default: Some("'live'".into()), ..status_col() };
    let baseline = schema_with_enum(&["live", "ended"], vec![table("widgets", vec![with_default.clone()])]);
    let desired = schema_with_enum(&["live"], vec![table("widgets", vec![with_default])]);

    let sql = up(&baseline, &desired);
    assert!(sql.contains("ALTER TABLE public.widgets ALTER COLUMN status DROP DEFAULT;"), "{sql}");
    assert!(
        sql.contains("USING (CASE WHEN status::text IN ('live') THEN status::text ELSE 'live' END)::public.status;"),
        "{sql}"
    );
    assert!(sql.contains("ALTER TABLE public.widgets ALTER COLUMN status SET DEFAULT 'live';"), "{sql}");
}

#[test]
fn removing_a_value_falls_back_to_null_when_nullable() {
    let nullable = Column { nullable: true, ..status_col() };
    let baseline = schema_with_enum(&["live", "ended"], vec![table("widgets", vec![nullable.clone()])]);
    let desired = schema_with_enum(&["live"], vec![table("widgets", vec![nullable])]);

    let sql = up(&baseline, &desired);
    assert!(sql.contains("ELSE NULL END)::public.status;"), "{sql}");
}

#[test]
fn removing_a_value_falls_back_to_the_first_value_when_not_nullable() {
    let baseline = schema_with_enum(&["live", "ended"], vec![table("widgets", vec![status_col()])]);
    let desired = schema_with_enum(&["ended"], vec![table("widgets", vec![status_col()])]);

    let sql = up(&baseline, &desired);
    assert!(
        sql.contains("USING (CASE WHEN status::text IN ('ended') THEN status::text ELSE 'ended' END)::public.status;"),
        "{sql}"
    );
}

#[test]
fn reordering_values_recreates_the_type() {
    let baseline = schema_with_enum(&["live", "ended"], vec![]);
    let desired = schema_with_enum(&["ended", "live"], vec![]);

    let sql = up(&baseline, &desired);
    assert!(sql.contains("CREATE TYPE public.status AS ENUM ('ended', 'live');"), "{sql}");
}

#[test]
fn a_dropped_table_is_dropped_before_the_enum_replace_and_excluded_from_it() {
    let baseline = schema_with_enum(
        &["live", "ended"],
        vec![table("widgets", vec![status_col()]), table("gadgets", vec![status_col()])],
    );
    let desired = schema_with_enum(&["live", "ended", "paused"], vec![table("widgets", vec![status_col()])]);

    let sql = up(&baseline, &desired);
    let drop_position = sql.find("DROP TABLE public.gadgets CASCADE;").expect("gadgets dropped");
    let replace_position = sql.find("ALTER TYPE public.status RENAME TO").expect("status replaced");
    assert!(drop_position < replace_position, "{sql}");
    assert!(!sql.contains("ALTER TABLE public.gadgets ALTER COLUMN"), "{sql}");
}

#[test]
fn moving_a_column_off_a_dying_enum_drops_the_type_last() {
    let mut baseline = schema(vec![table("widgets", vec![col("status", "public.old_status")])]);
    baseline.enums.insert(
        "public.old_status".into(),
        EnumType { name: "public.old_status".into(), values: vec!["live".into()] },
    );
    let mut desired = schema(vec![table("widgets", vec![col("status", "public.new_status")])]);
    desired.enums.insert(
        "public.new_status".into(),
        EnumType { name: "public.new_status".into(), values: vec!["live".into()] },
    );

    let sql = up(&baseline, &desired);
    let create_position = sql.find("CREATE TYPE public.new_status").expect("new enum created");
    let cast_position = sql
        .find("ALTER COLUMN status TYPE public.new_status USING status::text::public.new_status")
        .expect("column re-pointed with a cast");
    let drop_position = sql.find("DROP TYPE IF EXISTS public.old_status;").expect("old enum dropped");
    assert!(create_position < cast_position, "{sql}");
    assert!(cast_position < drop_position, "{sql}");
}

#[test]
fn invert_replace_enum_restores_the_old_values() {
    let baseline = schema_with_enum(&["live", "ended"], vec![table("widgets", vec![status_col()])]);
    let desired = schema_with_enum(&["live", "ended", "paused"], vec![table("widgets", vec![status_col()])]);
    let changes = diff(&baseline, &desired, &mut NoRenames);

    let down = render(&invert(&changes, &baseline));
    assert!(down.contains("CREATE TYPE public.status AS ENUM ('live', 'ended');"), "{down}");
    assert!(down.contains("DROP TYPE public.status_old;"), "{down}");
}

#[test]
fn create_table_inlines_table_constraints() {
    let mut widgets = table("widgets", vec![col("supplier_id", "uuid"), col("code", "text")]);
    widgets.constraints = vec![
        unique("widgets_supplier_id_code_key", &["supplier_id", "code"]),
        check("widgets_positive_check", "weight_kg > 0"),
    ];

    let sql = up(&DatabaseSchema::default(), &schema(vec![widgets]));

    assert!(
        sql.contains("CONSTRAINT widgets_supplier_id_code_key UNIQUE (supplier_id, code)"),
        "{sql}"
    );
    assert!(sql.contains("CONSTRAINT widgets_positive_check CHECK (weight_kg > 0)"), "{sql}");
}

#[test]
fn adding_a_table_constraint_alters_the_table() {
    let baseline = schema(vec![table("widgets", vec![col("code", "text")])]);
    let mut widgets = table("widgets", vec![col("code", "text")]);
    widgets.constraints = vec![check("widgets_positive_check", "weight_kg > 0")];

    assert_eq!(
        up(&baseline, &schema(vec![widgets])).trim(),
        "ALTER TABLE public.widgets ADD CONSTRAINT widgets_positive_check CHECK (weight_kg > 0);"
    );
}

#[test]
fn changing_a_constraint_expression_drops_and_re_adds_it() {
    let mut before = table("widgets", vec![col("code", "text")]);
    before.constraints = vec![check("widgets_weight_check", "weight_kg > 0")];
    let mut after = table("widgets", vec![col("code", "text")]);
    after.constraints = vec![check("widgets_weight_check", "weight_kg >= 0")];

    let sql = up(&schema(vec![before]), &schema(vec![after]));
    let drop = sql.find("DROP CONSTRAINT IF EXISTS widgets_weight_check").expect("dropped");
    let add = sql.find("ADD CONSTRAINT widgets_weight_check CHECK (weight_kg >= 0)").expect("re-added");

    assert!(drop < add, "{sql}");
}

#[test]
fn dropping_a_constraint_is_reversible() {
    let mut before = table("widgets", vec![col("code", "text")]);
    before.constraints = vec![unique("widgets_code_key2", &["code"])];
    let baseline = schema(vec![before]);
    let desired = schema(vec![table("widgets", vec![col("code", "text")])]);

    let down = render(&invert(&diff(&baseline, &desired, &mut NoRenames), &baseline));
    assert!(down.contains("ADD CONSTRAINT widgets_code_key2 UNIQUE (code);"), "{down}");
}

#[test]
fn a_partial_unique_index_renders_its_predicate() {
    let mut widgets = table("widgets", vec![col("coil_id", "uuid")]);
    widgets.indexes = vec![Index {
        name: "widgets_coil_id_partial_idx".into(),
        columns: vec!["coil_id".into()],
        unique: true,
        predicate: Some("voided_at IS NULL".into()),
    }];

    let sql = up(&DatabaseSchema::default(), &schema(vec![widgets]));

    assert!(
        sql.contains(
            "CREATE UNIQUE INDEX widgets_coil_id_partial_idx ON public.widgets (coil_id) \
             WHERE voided_at IS NULL;"
        ),
        "{sql}"
    );
}

#[test]
fn dropping_a_column_level_unique_is_no_longer_silent() {
    let baseline = schema(vec![table("inks", vec![Column { unique: true, ..col("vendor_code", "text") }])]);
    let desired = schema(vec![table("inks", vec![col("vendor_code", "text")])]);

    assert_eq!(
        up(&baseline, &desired).trim(),
        "ALTER TABLE public.inks DROP CONSTRAINT IF EXISTS inks_vendor_code_key;"
    );
}
