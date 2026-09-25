use orm::{table_type, view_type};
use uuid::Uuid;

#[table_type(schema = "orm_drift_regression", name = "widgets", export_to = "types/drift.ts")]
#[table(
    check("widgets_block_check" = "false"),
    index(name = "widgets_id_present_idx", id, where = "id IS NOT NULL"),
)]
pub struct Widget {
    #[pg(primary)]
    pub id: Uuid,
}

#[view_type(schema = "orm_drift_regression", name = "widget_view", export_to = "types/drift.ts")]
pub struct WidgetView {
    #[pg(view(orm_drift_regression.widgets.id))]
    pub id: Uuid,
}

#[test]
fn live_drift_rejects_changed_checks_and_view_columns() {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run the live drift test");
        return;
    };

    let directory = std::env::temp_dir().join(format!("orm-drift-regression-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("create migration directory");
    orm::migrate::generate(&directory, "drift_regression", false).expect("generate migration");
    orm::migrate::apply(&directory, Some(url.clone())).expect("apply migration");
    orm::migrate::diff_live(&directory, Some(url.clone()), None, true, false)
        .expect("unchanged schema verifies");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect");
        let connection_task = tokio::spawn(connection);

        client.batch_execute(
            "ALTER TABLE orm_drift_regression.widgets DROP CONSTRAINT widgets_block_check;
             ALTER TABLE orm_drift_regression.widgets ADD CONSTRAINT widgets_block_check CHECK (NULL::boolean);
             INSERT INTO orm_drift_regression.widgets VALUES (gen_random_uuid());",
        )
        .await
        .expect("change CHECK contract");

        connection_task.abort();
    });

    let changed_check = orm::migrate::diff_live(&directory, Some(url.clone()), None, true, false)
        .expect_err("a CHECK that now accepts rows must be drift");
    assert!(changed_check.to_string().contains("database drifted"), "{changed_check:#}");

    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect");
        let connection_task = tokio::spawn(connection);

        client.batch_execute(
            "DELETE FROM orm_drift_regression.widgets;
             ALTER TABLE orm_drift_regression.widgets DROP CONSTRAINT widgets_block_check;
             ALTER TABLE orm_drift_regression.widgets ADD CONSTRAINT widgets_block_check CHECK (false);
             DROP INDEX orm_drift_regression.widgets_id_present_idx;
             CREATE INDEX widgets_id_present_idx ON orm_drift_regression.widgets (id) WHERE id IS NULL;",
        )
        .await
        .expect("change index predicate");

        connection_task.abort();
    });

    let changed_index = orm::migrate::diff_live(&directory, Some(url.clone()), None, true, false)
        .expect_err("a changed partial-index predicate must be drift");
    assert!(changed_index.to_string().contains("database drifted"), "{changed_index:#}");

    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect");
        let connection_task = tokio::spawn(connection);

        client.batch_execute(
            "DROP INDEX orm_drift_regression.widgets_id_present_idx;
             CREATE INDEX widgets_id_present_idx ON orm_drift_regression.widgets (id) WHERE id IS NOT NULL;
             ALTER VIEW orm_drift_regression.widget_view RENAME COLUMN id TO wrong_id;",
        )
        .await
        .expect("rename view output");
        let error = client
            .query("SELECT id FROM orm_drift_regression.widget_view", &[])
            .await
            .expect_err("the expected view column is absent");
        assert_eq!(error.as_db_error().expect("database error").code().code(), "42703");

        connection_task.abort();
    });

    let changed_view = orm::migrate::diff_live(&directory, Some(url.clone()), None, true, false)
        .expect_err("a renamed view output must be drift");
    assert!(changed_view.to_string().contains("database drifted"), "{changed_view:#}");

    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect");
        let connection_task = tokio::spawn(connection);

        client.batch_execute("DROP SCHEMA orm_drift_regression CASCADE; DROP TABLE public._orm_migrations;")
            .await
            .expect("clean fixture");

        connection_task.abort();
    });
    std::fs::remove_dir_all(directory).expect("remove migration directory");
}
