use chrono::{DateTime, Utc};
use orm::numeric::NumericText;
use orm::query::{FilterGroup, FilterOp, QueryBuilderExt, QueryModel, QueryOptions, SortOrder, UpdateValues};
use orm::{table_type, view_type};
use tokio_postgres::NoTls;
use uuid::Uuid;

#[table_type(schema = "pg_temp", name = "fluent_widgets", export_to = "types/fluent.ts")]
pub struct FluentWidget {
    #[pg(primary)]
    pub id: Uuid,
    #[pg(unique)]
    pub code: String,
    pub version: i32,
    pub amount: NumericText,
    pub note: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
}

#[view_type(schema = "pg_temp", name = "fluent_widget_cards", export_to = "types/fluent.ts")]
pub struct FluentWidgetCard {
    #[pg(view(pg_temp.fluent_widgets.id))]
    pub id: Uuid,
    #[pg(view(pg_temp.fluent_widgets.code))]
    pub label: String,
}

const FIXTURE: &str = "
    CREATE TEMP TABLE fluent_widgets (
        id uuid PRIMARY KEY, code text NOT NULL UNIQUE, version integer NOT NULL,
        amount numeric NOT NULL, note text, checked_at timestamptz
    );
    CREATE TEMP VIEW fluent_widget_cards AS SELECT id, code AS label FROM fluent_widgets;
";

async fn fixture() -> Option<deadpool_postgres::Pool> {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run fluent query tests");
        return None;
    };

    let manager = deadpool_postgres::Manager::new(
        url.parse()
            .expect("database config"),
        NoTls
    );

    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("pool");

    pool.get()
        .await
        .expect("client")
        .batch_execute(FIXTURE)
        .await
        .expect("temporary fixture");

    for code in ["alpha", "beta", "gamma"] {
        pool.create_fluent_widget(&FluentWidgetInsert {
            id: Some(Uuid::new_v4()),
            code: code.into(),
            version: 0,
            amount: NumericText::new("10.500000000000000000001")
                .unwrap(),
            note: None,
            checked_at: None,
        })
        .await
        .expect("seed");
    }

    Some(pool)
}

#[test]
fn generated_metadata_uses_database_names_and_view_aliases() {
    assert_eq!(FluentWidget::RELATION, "pg_temp.fluent_widgets");
    assert_eq!(FluentWidget::COLUMNS, &["id", "code", "version", "amount", "note", "checked_at"]);
    assert_eq!(FluentWidgetCard::RELATION, "pg_temp.fluent_widget_cards");
    assert_eq!(FluentWidgetCard::COLUMNS, &["id", "label"]);
}

#[tokio::test]
async fn conditional_updates_bind_values_and_return_exact_rows() {
    let Some(pool) = fixture()
        .await else { return; };

    let note = "quoted ' value; DROP TABLE fluent_widgets; --";
    let rows = pool
        .update::<FluentWidget>()
        .set("note", Some(note.to_string()))
        .add("version", 1_i32)
        .subtract(
            "amount",
            NumericText::new("0.000000000000000000001")
                .unwrap()
        )
        .database_now("checked_at")
        .where_("code", FilterOp::Eq, "alpha")
        .where_("version", FilterOp::Eq, 0_i32)
        .returning()
        .await
        .expect("conditional update");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].note.as_deref(), Some(note));
    assert_eq!(rows[0].version, 1);
    assert_eq!(rows[0].amount.as_str(), "10.500000000000000000000");
    assert!(rows[0].checked_at.is_some());

    let stale = pool
        .update::<FluentWidget>()
        .add("version", 1_i32)
        .where_("id", FilterOp::Eq, rows[0].id)
        .where_("version", FilterOp::Eq, 0_i32)
        .execute()
        .await
        .expect("stale version");

    assert_eq!(stale, 0);

    let cleared = pool
        .update::<FluentWidget>()
        .options(
            QueryOptions::new()
                .filter("id", FilterOp::Eq, rows[0].id)
        )
        .values(
            UpdateValues::new()
                .null("note")
        )
        .set("checked_at", None::<DateTime<Utc>>)
        .returning()
        .await
        .expect("existing option/value builders");

    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].note, None);
    assert_eq!(cleared[0].checked_at, None);
    assert_eq!(cleared[0].version, 1);

    assert_eq!(pool.select::<FluentWidget>().fetch_all().await.unwrap().len(), 3);
}

#[tokio::test]
async fn writes_reject_missing_predicates_and_invalid_options_before_mutation() {
    let Some(pool) = fixture()
        .await else { return; };

    let mut client = pool.get()
        .await
        .unwrap();

    let transaction = client.transaction()
        .await
        .unwrap();

    for options in [
        QueryOptions::new(),
        QueryOptions::new()
            .filter("id", FilterOp::Eq, None::<Uuid>),
        QueryOptions::new()
            .filter_group(FilterGroup::or()),
    ] {
        let error = transaction
            .update::<FluentWidget>()
            .set("version", 9_i32)
            .options(options)
            .execute()
            .await
            .unwrap_err();

        assert!(error.to_string().contains("at least one filter"));
    }

    assert!(transaction.delete::<FluentWidget>().execute().await.is_err());
    assert!(transaction.delete::<FluentWidget>()
        .where_("id", FilterOp::Eq, None::<Uuid>)
        .returning().await.is_err());

    for options in [
        QueryOptions::new()
            .limit(1),
        QueryOptions::new()
            .offset(1),
        QueryOptions::new()
            .sort(orm::query::QuerySort::new("code", SortOrder::Asc)),
        QueryOptions::new()
            .for_update(),
        QueryOptions::new()
            .for_share(),
    ] {
        let error = transaction
            .update::<FluentWidget>()
            .set("version", 9_i32)
            .options(options)
            .where_("code", FilterOp::Eq, "alpha")
            .returning()
            .await
            .unwrap_err();

        assert!(error.to_string().contains("predicates only"));
    }

    let bounded_delete = transaction.delete::<FluentWidget>()
        .options(
            QueryOptions::new()
                .limit(1)
        )
        .where_("code", FilterOp::Eq, "alpha")
        .execute()
        .await
        .unwrap_err();

    assert!(bounded_delete.to_string().contains("predicates only"));

    assert!(transaction.update::<FluentWidget>()
        .where_("code", FilterOp::Eq, "alpha")
        .execute().await.is_err());

    assert!(transaction.update::<FluentWidget>()
        .set("code", "one").set("code", "two")
        .where_("code", FilterOp::Eq, "alpha")
        .execute().await.is_err());

    assert!(transaction.update::<FluentWidget>()
        .set("missing_column", 1_i32)
        .where_("code", FilterOp::Eq, "alpha")
        .execute().await.is_err());

    assert!(transaction.update::<FluentWidget>()
        .set("version", 9_i32)
        .where_("code", FilterOp::Eq, "alpha")
        .all_rows()
        .execute().await.is_err());

    assert!(transaction.delete::<FluentWidget>()
        .all_rows()
        .where_("code", FilterOp::Eq, "alpha")
        .returning().await.is_err());

    for column in ["missing", "code; DROP TABLE fluent_widgets", "code OR true --"] {
        assert!(transaction.update::<FluentWidget>()
            .set("version", 9_i32)
            .where_(column, FilterOp::Eq, "alpha")
            .execute().await.is_err());

        assert!(transaction.delete::<FluentWidget>()
            .where_(column, FilterOp::Eq, "alpha")
            .execute().await.is_err());

        assert!(transaction.select::<FluentWidget>()
            .where_(column, FilterOp::Eq, "alpha")
            .fetch_all().await.is_err());

        assert!(transaction.select::<FluentWidget>()
            .order_by(column, SortOrder::Asc)
            .fetch_all().await.is_err());
    }

    let unchanged = transaction.select::<FluentWidget>()
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(unchanged.len(), 3);
    assert!(unchanged.iter().all(|row| row.version == 0));
    transaction.rollback()
        .await
        .unwrap();
}

#[tokio::test]
async fn reads_compose_predicates_ordering_views_and_explicit_cardinality() {
    let Some(pool) = fixture()
        .await else { return; };

    let rows = pool.select::<FluentWidget>()
        .where_group(
            FilterGroup::or()
                .filter("code", FilterOp::Eq, "gamma")
                .filter("code", FilterOp::Eq, "alpha")
        )
        .where_null("note")
        .where_("amount", FilterOp::In, vec![NumericText::new("10.500000000000000000001").unwrap()])
        .order_by("version", SortOrder::Asc)
        .then_order_by("code", SortOrder::Desc)
        .offset(1)
        .limit(1)
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].code, "alpha");
    assert!(pool.select::<FluentWidget>()
        .where_not_null("note")
        .fetch_optional().await.unwrap().is_none());

    assert!(pool.select::<FluentWidget>()
        .where_("code", FilterOp::Eq, "absent")
        .fetch_one().await.is_err());

    assert!(pool.select::<FluentWidget>().fetch_optional().await.is_err());
    assert!(pool.select::<FluentWidget>().fetch_one().await.is_err());
    let first = pool.select::<FluentWidget>()
        .order_by("code", SortOrder::Asc)
        .limit(1)
        .fetch_optional()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first.code, "alpha");
    let card = pool.select::<FluentWidgetCard>()
        .where_("label", FilterOp::EqInsensitive, "ALPHA")
        .fetch_one()
        .await
        .unwrap();

    assert_eq!(card.id, first.id);
    assert_eq!(card.label, "alpha");
}

#[tokio::test]
async fn affected_counts_and_returning_cover_filtered_and_explicit_full_writes() {
    let Some(pool) = fixture()
        .await else { return; };

    let changed = pool.update::<FluentWidget>()
        .set("note", "temporary")
        .all_rows()
        .execute()
        .await
        .unwrap();

    assert_eq!(changed, 3);
    let changed = pool.update::<FluentWidget>()
        .set_null("note")
        .all_rows()
        .returning()
        .await
        .unwrap();

    assert_eq!(changed.len(), 3);
    assert!(changed.iter().all(|row| row.note.is_none()));
    let removed = pool.delete::<FluentWidget>()
        .where_("code", FilterOp::Eq, "alpha")
        .returning()
        .await
        .unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].code, "alpha");
    assert!(pool.delete::<FluentWidget>()
        .where_("code", FilterOp::Eq, "alpha")
        .returning().await.unwrap().is_empty());

    assert_eq!(pool.delete::<FluentWidget>()
        .where_("code", FilterOp::Eq, "alpha")
        .execute().await.unwrap(), 0);

    assert_eq!(pool.delete::<FluentWidget>().all_rows().execute().await.unwrap(), 2);
    assert!(pool.select::<FluentWidget>().fetch_all().await.unwrap().is_empty());
}

#[tokio::test]
async fn pooled_and_direct_transactions_preserve_scope_and_send_futures() {
    fn require_send<T: Send>(_: T) {}
    let Some(pool) = fixture()
        .await else { return; };

    require_send(
        pool.select::<FluentWidget>()
            .fetch_all()
    );

    require_send(
        pool.update::<FluentWidget>()
            .all_rows()
            .set("version", 1_i32)
            .execute()
    );

    require_send(
        pool.delete::<FluentWidget>()
            .all_rows()
            .returning()
    );

    let mut client = pool.get()
        .await
        .unwrap();

    assert_eq!(client.select::<FluentWidget>().fetch_all().await.unwrap().len(), 3);
    let transaction = client.transaction()
        .await
        .unwrap();

    require_send(
        transaction.update::<FluentWidget>()
            .all_rows()
            .set("version", 1_i32)
            .returning()
    );

    let row = transaction.select::<FluentWidget>()
        .where_("code", FilterOp::Eq, "alpha")
        .for_update()
        .fetch_one()
        .await
        .unwrap();

    assert_eq!(transaction.update::<FluentWidget>()
        .set("version", 1_i32)
        .where_("id", FilterOp::Eq, row.id)
        .execute().await.unwrap(), 1);

    assert_eq!(transaction.select::<FluentWidgetCard>()
        .where_("id", FilterOp::Eq, row.id)
        .fetch_one().await.unwrap().label, "alpha");

    transaction.rollback()
        .await
        .unwrap();

    assert_eq!(client.select::<FluentWidget>()
        .where_("id", FilterOp::Eq, row.id)
        .fetch_one().await.unwrap().version, 0);

    let url = std::env::var("ORM_TEST_DATABASE_URL")
        .unwrap();

    let (mut direct, connection) = tokio_postgres::connect(&url, NoTls)
        .await
        .unwrap();

    let task = tokio::spawn(async move { connection.await
        .unwrap() });

    direct.batch_execute(FIXTURE)
        .await
        .unwrap();

    assert!(direct.select::<FluentWidget>().fetch_all().await.unwrap().is_empty());
    let transaction = direct.transaction()
        .await
        .unwrap();

    transaction.create_fluent_widget(&FluentWidgetInsert {
        id: Some(Uuid::new_v4()),
        code: "direct".into(),
        version: 0,
        amount: NumericText::new("1")
            .unwrap(),
        note: None,
        checked_at: None,
    })
    .await
    .unwrap();

    assert_eq!(transaction.select::<FluentWidget>()
        .where_("code", FilterOp::Eq, "direct")
        .for_share()
        .fetch_one().await.unwrap().version, 0);

    transaction.rollback()
        .await
        .unwrap();

    assert!(direct.select::<FluentWidget>().fetch_all().await.unwrap().is_empty());
    drop(direct);
    task.abort();
}

#[tokio::test]
async fn fluent_writes_keep_postgresql_constraint_error_sources() {
    let Some(pool) = fixture()
        .await else { return; };

    for returning in [false, true] {
        let update = pool.update::<FluentWidget>()
            .set("code", "beta")
            .where_("code", FilterOp::Eq, "alpha");

        let error = if returning {
            update.returning()
                .await
                .unwrap_err()
        } else {
            update.execute()
                .await
                .unwrap_err()
        };

        let postgres = error.downcast_ref::<tokio_postgres::Error>()
            .expect("PostgreSQL source");

        let database = postgres.as_db_error()
            .expect("constraint error");

        assert_eq!(database.code().code(), "23505");
        assert_eq!(database.constraint(), Some("fluent_widgets_code_key"));
    }

    assert_eq!(pool.select::<FluentWidget>().fetch_all().await.unwrap().len(), 3);
}
