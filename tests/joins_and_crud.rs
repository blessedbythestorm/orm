use orm::numeric::NumericText;
use orm::query::{FilterOp, JoinOn, QueryBuilderExt, QueryOptions, QuerySort, SortOrder, UpdateValues};
use orm::{table_type, view_type};
use tokio_postgres::NoTls;
use uuid::Uuid;

#[table_type(schema = "pg_temp", name = "accounts", export_to = "types/join_tests.ts")]
pub struct Account {
    #[pg(primary, default_value(gen_random_uuid()))]
    pub account_id: Uuid,
    #[pg(unique)]
    #[api(validate(length(min(2), max(20))))]
    pub code: String,
    #[pg(default_value(10.00))]
    #[crud(insert(optional))]
    pub balance: NumericText,
    pub parent_id: Option<Uuid>,
    #[api(validate(length(min(3))))]
    pub note: Option<String>,
    #[crud(insert(skip))]
    pub server_value: i32,
}

#[table_type(schema = "pg_temp", name = "entries", export_to = "types/join_tests.ts")]
#[pg(unique(columns(account_id, label)))]
pub struct Entry {
    #[pg(primary, default_value(gen_random_uuid()))]
    pub id: Uuid,
    pub account_id: Uuid,
    pub label: String,
    pub active: bool,
}

#[table_type(schema = "pg_temp", name = "defaults_only", export_to = "types/join_tests.ts")]
pub struct DefaultsOnly {
    #[pg(primary, default_value(gen_random_uuid()))]
    pub id: Uuid,
    #[pg(default_value(11))]
    #[crud(insert(optional))]
    pub amount: i32,
}

#[view_type(schema = "pg_temp", name = "account_cards", export_to = "types/join_tests.ts")]
pub struct AccountCard {
    #[pg(view(pg_temp.accounts.account_id))]
    pub id: Uuid,
    #[pg(view(pg_temp.accounts.code))]
    pub label: String,
}

#[derive(Debug, orm::FromRow)]
struct Summary {
    code: String,
    entry_id: Option<Uuid>,
    entry_label: Option<String>,
    parent_code: Option<String>,
}

#[derive(Debug, orm::FromRow)]
struct Label {
    label: String,
}

fn input(code: &str) -> AccountInsert {
    AccountInsert {
        account_id: None,
        code: code.into(),
        balance: None,
        parent_id: None,
        note: None,
    }
}

fn empty_patch<T: serde::de::DeserializeOwned>() -> T {
    serde_json::from_value(serde_json::json!({}))
        .unwrap()
}

async fn fixture() -> Option<deadpool_postgres::Pool> {
    let Ok(url) = std::env::var("ORM_TEST_DATABASE_URL") else {
        eprintln!("skipping: set ORM_TEST_DATABASE_URL to run join/CRUD tests");
        return None;
    };

    let pool = deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
        url.parse()
            .unwrap(),
        NoTls
    ))
    .max_size(1)
    .build()
    .unwrap();

    pool.get()
        .await
        .unwrap()
        .batch_execute(
            "
        CREATE TEMP TABLE accounts (
            account_id uuid PRIMARY KEY DEFAULT gen_random_uuid(), code text UNIQUE NOT NULL,
            balance numeric NOT NULL DEFAULT 10.00, parent_id uuid, note text,
            server_value integer NOT NULL DEFAULT 42
        );
        CREATE TEMP TABLE entries (
            id uuid PRIMARY KEY DEFAULT gen_random_uuid(), account_id uuid NOT NULL,
            label text NOT NULL, active boolean NOT NULL, UNIQUE(account_id, label)
        );
        CREATE TEMP TABLE defaults_only (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), amount integer NOT NULL DEFAULT 11);
        CREATE TEMP VIEW account_cards AS SELECT account_id AS id, code AS label FROM accounts;
    "
        )
        .await
        .unwrap();

    let ana = pool.create_account(&input("ana"))
        .await
        .unwrap();

    let bob = pool.create_account(&AccountInsert { parent_id: Some(ana.account_id), ..input("bob") })
        .await
        .unwrap();

    pool.create_account(&input("cy"))
        .await
        .unwrap();

    for (account_id, label, active) in [(ana.account_id, "main", true), (ana.account_id, "archived", false), (bob.account_id, "main", true)] {
        pool.create_entry(&EntryInsert {
            id: None,
            account_id,
            label: label.into(),
            active,
        })
        .await
        .unwrap();
    }

    Some(pool)
}

#[tokio::test]
async fn joins_bind_on_and_where_values_and_map_nullable_projections() {
    let Some(pool) = fixture()
        .await else { return; };

    let rows = pool.select::<Account>()
        .alias("a")
        .left_join::<Entry>(
            "e",
            JoinOn::eq("a.account_id", "e.account_id")
                .where_("e.active", FilterOp::Eq, true)
        )
        .left_join::<Account>(
            "p",
            JoinOn::eq("a.parent_id", "p.account_id")
                .where_("p.code", FilterOp::Eq, "ana")
        )
        .where_("a.code", FilterOp::In, vec!["ana".to_string(), "bob".to_string(), "cy".to_string()])
        .order_by("a.code", SortOrder::Asc)
        .project::<Summary>(&[("a.code", "code"), ("e.id", "entry_id"), ("e.label", "entry_label"), ("p.code", "parent_code")])
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].code, "ana");
    assert!(rows[0].entry_id.is_some());
    assert_eq!(rows[0].entry_label.as_deref(), Some("main"));
    assert_eq!(rows[0].parent_code, None);
    assert_eq!(rows[1].code, "bob");
    assert_eq!(rows[1].parent_code.as_deref(), Some("ana"));
    assert_eq!(rows[2].code, "cy");
    assert_eq!(rows[2].entry_id, None);
    assert_eq!(rows[2].entry_label, None);
    assert_eq!(rows[2].parent_code, None);

    let rows = pool.select::<Entry>()
        .alias("e")
        .inner_join::<Account>("a", JoinOn::eq("e.account_id", "a.account_id"))
        .inner_join::<Account>("p", JoinOn::eq("a.parent_id", "p.account_id"))
        .where_("e.active", FilterOp::Eq, true)
        .project::<Label>(&[("p.code", "label")])
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "ana");
}

#[tokio::test]
async fn joins_preserve_sql_multiplicity_pagination_and_base_model_results() {
    let Some(pool) = fixture()
        .await else { return; };

    let rows = pool.select::<Account>()
        .alias("a")
        .inner_join::<Entry>("e", JoinOn::eq("account_id", "e.account_id"))
        .order_by("a.code", SortOrder::Asc)
        .then_order_by("e.label", SortOrder::Asc)
        .limit(1)
        .offset(1)
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].code, "ana");
    let count = pool.select::<Account>()
        .alias("a")
        .left_join::<Entry>("e", JoinOn::eq("account_id", "e.account_id"))
        .limit(0)
        .offset(999)
        .for_update()
        .count()
        .await
        .unwrap();

    assert_eq!(count, 4);
    let cards = pool.select::<AccountCard>()
        .alias("card")
        .inner_join::<Entry>("e", JoinOn::eq("card.id", "e.account_id"))
        .where_("e.active", FilterOp::Eq, true)
        .order_by("card.label", SortOrder::Desc)
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(cards.iter().map(|row| row.label.as_str()).collect::<Vec<_>>(), ["bob", "ana"]);
    let pairs = pool.select::<Account>()
        .alias("a")
        .inner_join::<Account>(
            "b",
            JoinOn::eq("a.balance", "b.balance")
                .and_on("a.account_id", FilterOp::Ne, "b.account_id")
        )
        .count()
        .await
        .unwrap();

    assert_eq!(pairs, 6);
}

#[tokio::test]
async fn left_join_on_filters_preserve_unmatched_rows_and_lock_only_the_base() {
    let Some(pool) = fixture()
        .await else { return; };

    let mut client = pool.get()
        .await
        .unwrap();

    let transaction = client.transaction()
        .await
        .unwrap();

    let all = transaction.select::<Account>()
        .alias("a")
        .left_join::<Entry>(
            "e",
            JoinOn::eq("a.account_id", "e.account_id")
                .where_("e.active", FilterOp::Eq, true)
        )
        .for_update()
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(all.len(), 3);
    let matching = transaction.select::<Account>()
        .alias("a")
        .left_join::<Entry>("e", JoinOn::eq("a.account_id", "e.account_id"))
        .where_("e.active", FilterOp::Eq, true)
        .for_share()
        .fetch_all()
        .await
        .unwrap();

    assert_eq!(matching.len(), 2);
    assert_eq!(transaction.get_account_cards(QueryOptions::new()).await.unwrap().len(), 3);
    transaction.rollback()
        .await
        .unwrap();
}

#[tokio::test]
async fn joins_reject_invalid_scopes_on_conditions_and_projections() {
    let Some(pool) = fixture()
        .await else { return; };

    let mut client = pool.get()
        .await
        .unwrap();

    let transaction = client.transaction()
        .await
        .unwrap();

    for alias in ["a", "unsafe;drop", ""] {
        assert!(transaction.select::<Account>().alias("a")
            .inner_join::<Entry>(alias, JoinOn::eq("a.account_id", "e.account_id"))
            .fetch_all().await.is_err());
    }

    for on in [
        JoinOn::new(),
        JoinOn::new()
            .where_("e.id", FilterOp::Eq, None::<Uuid>),
        JoinOn::eq("a.missing", "e.account_id"),
        JoinOn::eq("later.account_id", "e.account_id"),
        JoinOn::new()
            .and_on("a.account_id", FilterOp::In, "e.account_id"),
        JoinOn::eq("a.account_id; DROP TABLE accounts", "e.account_id"),
    ] {
        assert!(transaction.select::<Account>().alias("a")
            .inner_join::<Entry>("e", on)
            .inner_join::<Account>("later", JoinOn::eq("a.account_id", "later.account_id"))
            .fetch_all().await.is_err());
    }

    for projection in [vec![], vec![("code", "label"), ("note", "label")], vec![("code", "bad;alias")], vec![("missing.code", "label")]] {
        assert!(transaction.select::<Account>().project::<Label>(&projection).fetch_all().await.is_err());
    }

    assert!(transaction.select::<Account>().alias("a")
        .left_join::<Entry>("e", JoinOn::eq("a.account_id", "e.account_id"))
        .where_("a.code", FilterOp::Eq, "cy")
        .project::<Label>(&[("e.label", "label")])
        .fetch_all().await.is_err());

    assert_eq!(transaction.select::<Account>().count().await.unwrap(), 3);
    transaction.rollback()
        .await
        .unwrap();
}

#[tokio::test]
async fn generated_crud_preserves_defaults_skipped_columns_nulls_and_missing_rows() {
    let Some(pool) = fixture()
        .await else { return; };

    let mut client = pool.get()
        .await
        .unwrap();

    let transaction = client.transaction()
        .await
        .unwrap();

    let created = transaction.create_account(&AccountInsert { note: Some("keep".into()), ..input("new") })
        .await
        .unwrap();

    assert_eq!(created.balance.as_str(), "10.00");
    assert_eq!(created.server_value, 42);
    assert_eq!(transaction.get_account(&created.account_id).await.unwrap().code, "new");
    let changed = transaction.update_account(&created.account_id, &AccountUpdate { code: Some("changed".into()), ..empty_patch() })
        .await
        .unwrap();

    assert_eq!(changed.note.as_deref(), Some("keep"));
    let cleared = transaction.update_account(&created.account_id, &AccountUpdate { note: Some(None), ..empty_patch() })
        .await
        .unwrap();

    assert_eq!(cleared.note, None);
    assert!(transaction.update_account(&created.account_id, &empty_patch::<AccountUpdate>()).await.is_err());
    assert_eq!(transaction.count_accounts(QueryOptions::new().limit(0).offset(999).sort(QuerySort::new("missing", SortOrder::Asc)).for_update()).await.unwrap(), 4);
    transaction.delete_account(&created.account_id)
        .await
        .unwrap();

    assert!(transaction.delete_account(&created.account_id).await.unwrap_err().to_string().contains("not found"));
    assert!(transaction.get_account(&created.account_id).await.unwrap_err().downcast_ref::<tokio_postgres::Error>().is_some());
    let default = transaction.create_defaults_only(&DefaultsOnlyInsert { id: None, amount: None })
        .await
        .unwrap();

    assert_eq!(default.amount, 11);
    assert_eq!(transaction.insert::<DefaultsOnly>().returning_one().await.unwrap().amount, 11);
    assert_eq!(transaction.delete_accounts(QueryOptions::new().filter("code", FilterOp::Eq, "cy")).await.unwrap(), 1);
    transaction.rollback()
        .await
        .unwrap();

    assert_eq!(client.count_accounts(QueryOptions::new()).await.unwrap(), 3);
}

#[tokio::test]
async fn generated_upserts_preserve_excluded_and_selective_update_semantics() {
    let Some(pool) = fixture()
        .await else { return; };

    let original = pool.select::<Account>()
        .where_("code", FilterOp::Eq, "ana")
        .fetch_one()
        .await
        .unwrap();

    let changed = pool.upsert_account_by_code(&AccountInsert {
        balance: Some(
            NumericText::new("7.50")
                .unwrap()
        ),
        note: Some("keep".into()),
        ..input("ana")
    })
    .await
    .unwrap();

    assert_eq!(changed.account_id, original.account_id);
    assert_eq!(changed.balance.as_str(), "7.50");
    let unchanged = pool.upsert_account_by_code_with(&input("ana"), &empty_patch::<AccountUpdate>())
        .await
        .unwrap();

    assert_eq!(unchanged.account_id, original.account_id);
    assert_eq!(unchanged.balance.as_str(), "7.50");
    assert_eq!(unchanged.note.as_deref(), Some("keep"));
    let cleared = pool.upsert_account_by_code_with(&input("ana"), &AccountUpdate { code: Some("must-be-excluded".into()), note: Some(None), ..empty_patch() })
        .await
        .unwrap();

    assert_eq!(cleared.code, "ana");
    assert_eq!(cleared.note, None);
    let reset = pool.upsert_account_by_code(&input("ana"))
        .await
        .unwrap();

    assert_eq!(reset.balance.as_str(), "10.00");
    let entry = pool.upsert_entry_by_account_id_and_label(&EntryInsert {
        id: None,
        account_id: original.account_id,
        label: "main".into(),
        active: false,
    })
    .await
    .unwrap();

    assert!(!entry.active);
    let entry_again = pool.upsert_entry_by_account_id_and_label_with(
        &EntryInsert {
            id: None,
            account_id: original.account_id,
            label: "main".into(),
            active: false,
        },
        &EntryUpdate { active: Some(true), ..empty_patch() }
    )
    .await
    .unwrap();

    assert_eq!(entry.id, entry_again.id);
    assert!(entry_again.active);
}

#[tokio::test]
async fn generated_writes_validate_before_querying() {
    let Some(pool) = fixture()
        .await else { return; };

    let invalid_create = pool.create_account(&input("x"))
        .await
        .unwrap_err();

    let create_errors = invalid_create.downcast_ref::<orm::ValidationErrors>()
        .expect("generated create retains ValidationErrors");

    assert!(create_errors.field_errors().contains_key("code"));
    assert!(pool.select::<Account>()
        .where_("code", FilterOp::Eq, "x")
        .fetch_optional()
        .await
        .unwrap()
        .is_none());

    let original = pool.select::<Account>()
        .where_("code", FilterOp::Eq, "ana")
        .fetch_one()
        .await
        .unwrap();

    let invalid_update = pool.update_account(&original.account_id, &AccountUpdate { note: Some(Some("no".into())), ..empty_patch() })
        .await
        .unwrap_err();

    assert!(invalid_update.downcast_ref::<orm::ValidationErrors>()
        .expect("generated update retains ValidationErrors")
        .field_errors()
        .contains_key("note"));

    let unchanged = pool.get_account(&original.account_id)
        .await
        .unwrap();

    assert_eq!(unchanged.note, original.note);

    let cleared = pool.update_account(&original.account_id, &AccountUpdate { note: Some(None), ..empty_patch() })
        .await
        .unwrap();

    assert_eq!(cleared.note, None);

    assert!(pool.upsert_account_by_code(&input("x"))
        .await
        .unwrap_err()
        .downcast_ref::<orm::ValidationErrors>()
        .is_some());

    assert!(pool.upsert_account_by_code_with(
            &input("ana"),
            &AccountUpdate {
                note: Some(Some("no".into())),
                ..empty_patch()
            }
        )
        .await
        .unwrap_err()
        .downcast_ref::<orm::ValidationErrors>()
        .is_some());

    let mut client = pool.get()
        .await
        .unwrap();

    let transaction = client.transaction()
        .await
        .unwrap();

    assert!(transaction.create_account(&input("x"))
        .await
        .unwrap_err()
        .downcast_ref::<orm::ValidationErrors>()
        .is_some());

    let valid = transaction.create_account(&AccountInsert { note: Some("valid note".into()), ..input("valid") })
        .await
        .unwrap();

    assert_eq!(valid.balance.as_str(), "10.00");

    transaction.rollback()
        .await
        .unwrap();
}

#[tokio::test]
async fn fluent_conflicts_bind_updates_after_inserts_and_preserve_error_sources() {
    let Some(pool) = fixture()
        .await else { return; };

    let changed = pool.insert::<Account>()
        .value("code", "ana")
        .on_conflict(&["code"])
        .do_update(
            UpdateValues::new()
                .add(
                    "balance",
                    NumericText::new("2.125")
                        .unwrap()
                )
                .assign("note", "changed")
        )
        .returning_one()
        .await
        .unwrap();

    assert_eq!(changed.balance.as_str(), "12.125");
    assert_eq!(changed.note.as_deref(), Some("changed"));
    assert_eq!(pool.insert::<Account>().value("code", "ana").on_conflict(&["code"]).do_nothing().execute().await.unwrap(), 0);
    assert!(pool.insert::<Account>().value("code", "ana").on_conflict(&["code"]).do_nothing().returning().await.unwrap().is_empty());
    for columns in [vec![], vec!["missing"], vec!["code", "code"]] {
        assert!(pool.insert::<Account>().value("code", "not-inserted").on_conflict(&columns).do_nothing().execute().await.is_err());
    }

    assert!(pool.insert::<Account>().value("code", "not-inserted").on_conflict(&["code"]).do_update(UpdateValues::new()).execute().await.is_err());
    assert!(pool.select::<Account>().where_("code", FilterOp::Eq, "not-inserted").fetch_optional().await.unwrap().is_none());
    let error = pool.create_account(&input("ana"))
        .await
        .unwrap_err();

    assert_eq!(error.downcast_ref::<tokio_postgres::Error>().unwrap().as_db_error().unwrap().constraint(), Some("accounts_code_key"));
}
