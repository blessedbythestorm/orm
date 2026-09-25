//! DB-backed round-trip, gated on `ORM_TEST_DATABASE_URL`. When that env var is
//! unset the test returns early, so `cargo test` stays green without a database;
//! set it to a throwaway Postgres to actually exercise apply + introspect.

use orm::schema::{Column, DatabaseSchema, NoRenames, Table, diff, introspect, render};
use orm::table_type;
use orm::query::{FilterOp, InsertValues, QueryOptions, UpdateValues};
use tokio_postgres::NoTls;
use uuid::Uuid;

const SCRATCH: &str = "orm_roundtrip_test";

#[table_type(schema = "orm_error_test", name = "widgets", export_to = "types/error_widget.ts")]
pub struct ErrorWidget {
    #[pg(primary)]
    pub id: Uuid,
    #[pg(unique)]
    pub code: String,
}

#[tokio::test]
async fn generated_create_keeps_constraint_error_source() {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run the constraint error test");
        return;
    };

    let (mut client, connection) = tokio_postgres::connect(&url, NoTls).await.expect("connect");
    let conn = tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute("DROP SCHEMA IF EXISTS orm_error_test CASCADE; CREATE SCHEMA orm_error_test; CREATE TABLE orm_error_test.widgets (id uuid PRIMARY KEY, code text NOT NULL UNIQUE);")
        .await
        .expect("create error fixture");

    let transaction = client.transaction().await.expect("begin");
    transaction
        .create_error_widget(&ErrorWidgetInsert { id: Some(Uuid::new_v4()), code: "same".into() })
        .await
        .expect("first create");
    let shared = transaction
        .get_error_widgets(
            QueryOptions::new()
                .filter("code", FilterOp::Eq, "same")
                .limit(1)
                .for_share(),
        )
        .await
        .expect("shared lock query");
    assert_eq!(shared.len(), 1);

    let dynamic_id = Uuid::new_v4();
    transaction
        .insert_error_widget_fields(
            InsertValues::new()
                .value("id", dynamic_id)
                .value("code", "dynamic".to_string()),
        )
        .await
        .expect("dynamic insert");
    let changed = transaction
        .update_error_widgets_where(
            QueryOptions::new()
                .filter("id", FilterOp::Eq, dynamic_id),
            UpdateValues::new()
                .assign("code", "changed".to_string()),
        )
        .await
        .expect("filtered update");
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].code, "changed");

    let (observer, observer_connection) = tokio_postgres::connect(&url, NoTls).await.expect("observer connect");
    let observer_task = tokio::spawn(async move {
        let _ = observer_connection.await;
    });

    orm::query::advisory_xact_lock_key(&transaction, "orm-error-widget")
        .await
        .expect("text advisory lock");
    let text_lock_available: bool = observer
        .query_one("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))", &[&"orm-error-widget"])
        .await
        .expect("probe text advisory lock")
        .get(0);
    assert!(!text_lock_available);

    orm::query::advisory_xact_lock_id(&transaction, 726184311)
        .await
        .expect("numeric advisory lock");
    let numeric_lock_available: bool = observer
        .query_one("SELECT pg_try_advisory_xact_lock($1)", &[&726184311_i64])
        .await
        .expect("probe numeric advisory lock")
        .get(0);
    assert!(!numeric_lock_available);

    let error = transaction
        .create_error_widget(&ErrorWidgetInsert { id: Some(Uuid::new_v4()), code: "same".into() })
        .await
        .expect_err("duplicate code");
    let postgres = error
        .downcast_ref::<tokio_postgres::Error>()
        .expect("PostgreSQL error remains in the anyhow chain");
    let database = postgres.as_db_error().expect("constraint violation");

    assert_eq!(database.code().code(), "23505");
    assert_eq!(database.constraint(), Some("widgets_code_key"));

    transaction.rollback().await.expect("rollback");
    client.batch_execute("DROP SCHEMA orm_error_test CASCADE;").await.expect("cleanup");
    conn.abort();
    observer_task.abort();
}

#[tokio::test]
async fn create_table_round_trips_through_introspect() {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run the db round-trip test");
        return;
    };

    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.expect("connect");
    let conn = tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS {SCRATCH} CASCADE; CREATE SCHEMA {SCRATCH};"))
        .await
        .expect("create scratch schema");

    let desired = desired_schema();
    let migration = render(&diff(&DatabaseSchema::default(), &desired, &mut NoRenames));
    client.batch_execute(&migration).await.expect("apply migration");

    let live = introspect(&client, &[SCRATCH.to_string()]).await.expect("introspect");
    let drift = diff(&live, &desired, &mut NoRenames);

    client.batch_execute(&format!("DROP SCHEMA IF EXISTS {SCRATCH} CASCADE;")).await.expect("cleanup");
    conn.abort();

    assert!(drift.is_empty(), "introspected schema drifted from desired: {drift:#?}");
}

#[tokio::test]
async fn introspect_includes_view_names_and_definitions() {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run the view introspection test");
        return;
    };

    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.expect("connect");
    let conn = tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute("DROP SCHEMA IF EXISTS orm_view_test CASCADE; CREATE SCHEMA orm_view_test; CREATE VIEW orm_view_test.one AS SELECT 1::int4 AS value;")
        .await
        .expect("create view fixture");

    let live = introspect(&client, &["orm_view_test".to_string()]).await.expect("introspect");
    let view = live.views.get("orm_view_test.one").expect("view found");
    assert_eq!(view.schema, "orm_view_test");
    assert_eq!(view.name, "one");
    assert!(view.definition.contains("SELECT"));

    client.batch_execute("DROP SCHEMA orm_view_test CASCADE;").await.expect("cleanup");
    conn.abort();
}

fn desired_schema() -> DatabaseSchema {
    let table = Table {
        schema: SCRATCH.to_string(),
        name: "widget".to_string(),
        columns: vec![
            column("id", "uuid", false, true),
            column("label", "text", false, false),
            column("count", "int4", true, false),
        ],
        ..Default::default()
    };

    DatabaseSchema { tables: [(table.qualified_name(), table)].into_iter().collect(), ..Default::default() }
}

fn column(name: &str, sql_type: &str, nullable: bool, primary_key: bool) -> Column {
    Column {
        name: name.into(),
        sql_type: sql_type.into(),
        nullable,
        primary_key,
        unique: false,
        default: None,
        foreign_key: None,
    }
}
