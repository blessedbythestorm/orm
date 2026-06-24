//! DB-backed round-trip, gated on `ORM_TEST_DATABASE_URL`. When that env var is
//! unset the test returns early, so `cargo test` stays green without a database;
//! set it to a throwaway Postgres to actually exercise apply + introspect.

use orm::schema::{Column, DatabaseSchema, NoRenames, Table, diff, introspect, render};
use tokio_postgres::NoTls;

const SCRATCH: &str = "orm_roundtrip_test";

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

fn desired_schema() -> DatabaseSchema {
    let table = Table {
        schema: SCRATCH.to_string(),
        name: "widget".to_string(),
        columns: vec![
            column("id", "uuid", false, true),
            column("label", "text", false, false),
            column("count", "int4", true, false),
        ],
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
