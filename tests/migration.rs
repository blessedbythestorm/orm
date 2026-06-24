use orm::schema::{
    Column, DatabaseSchema, EnumType, ForeignKey, NoRenames, ReferentialAction, Table, diff, invert, render,
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
    Table { schema: "public".into(), name: name.into(), columns }
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
    assert!(down.contains("DROP TABLE public.widgets;"), "{down}");
}

#[test]
fn invert_turns_add_column_into_drop_column() {
    let baseline = schema(vec![table("widgets", vec![col("id", "uuid")])]);
    let desired = schema(vec![table("widgets", vec![col("id", "uuid"), col("name", "text")])]);
    let changes = diff(&baseline, &desired, &mut NoRenames);

    let down = render(&invert(&changes, &baseline));
    assert_eq!(down.trim(), "ALTER TABLE public.widgets DROP COLUMN name;");
}
