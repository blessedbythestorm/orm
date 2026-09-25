use orm::query::{FilterGroup, FilterOp, InsertValues, QueryOptions, QuerySort, SortOrder, UpdateValues};

#[test]
fn no_filters_means_no_where() {
    let (sql, next) = QueryOptions::new().build_where_clause(1);
    assert_eq!(sql, "");
    assert_eq!(next, 1);
}

#[test]
fn single_filter_binds_one_param() {
    let (sql, next) = QueryOptions::new()
        .filter("handle", FilterOp::Eq, "ana")
        .build_where_clause(1);

    assert_eq!(sql, " WHERE handle = $1");
    assert_eq!(next, 2);
}

#[test]
fn separate_filters_are_anded_and_numbered_from_the_offset() {
    let (sql, next) = QueryOptions::new()
        .filter("published", FilterOp::Eq, true)
        .filter("category", FilterOp::Eq, "design")
        .build_where_clause(3);

    assert_eq!(sql, " WHERE published = $3 AND category = $4");
    assert_eq!(next, 5);
}

#[test]
fn or_group_is_parenthesized() {
    let (sql, next) = QueryOptions::new()
        .filter_group(
            FilterGroup::or()
                .filter("mentor_id", FilterOp::Eq, "a")
                .filter("student_id", FilterOp::Eq, "b"),
        )
        .build_where_clause(1);

    assert_eq!(sql, " WHERE (mentor_id = $1 OR student_id = $2)");
    assert_eq!(next, 3);
}

#[test]
fn null_check_consumes_no_param() {
    let (sql, next) = QueryOptions::new()
        .filter("deleted_at", FilterOp::IsNull, "")
        .build_where_clause(1);

    assert_eq!(sql, " WHERE deleted_at IS NULL");
    assert_eq!(next, 1);
}

#[test]
fn suffix_renders_order_limit_offset() {
    let suffix = QueryOptions::new()
        .sort(QuerySort::new("created_at", SortOrder::Desc))
        .limit(10)
        .offset(20)
        .to_sql_suffix();

    assert_eq!(suffix, " ORDER BY created_at DESC LIMIT 10 OFFSET 20");
}

#[test]
fn secondary_sort_preserves_primary_order_and_ignores_invalid_columns() {
    let suffix = QueryOptions::new()
        .sort(QuerySort::new("created_at", SortOrder::Desc))
        .then_sort(QuerySort::new("id", SortOrder::Desc))
        .then_sort(QuerySort::new("unsafe; drop table", SortOrder::Asc))
        .limit(10)
        .to_sql_suffix();

    assert_eq!(suffix, " ORDER BY created_at DESC, id DESC LIMIT 10");
}

#[test]
fn secondary_sort_can_start_an_ordering() {
    let suffix = QueryOptions::new()
        .then_sort(QuerySort::new("id", SortOrder::Asc))
        .to_sql_suffix();

    assert_eq!(suffix, " ORDER BY id ASC");
}

#[test]
fn share_lock_renders_after_limit() {
    let suffix = QueryOptions::new()
        .limit(1)
        .for_share()
        .to_sql_suffix();

    assert_eq!(suffix, " LIMIT 1 FOR SHARE");
}

#[test]
fn like_ops_wrap_the_value_with_wildcards() {
    assert_eq!(FilterOp::Like.wrap_value("ana"), "%ana%");
    assert_eq!(FilterOp::ILike.wrap_value("ana"), "%ana%");
    assert_eq!(FilterOp::Eq.wrap_value("ana"), "ana");
}

#[test]
fn null_ops_need_no_value() {
    assert!(!FilterOp::IsNull.needs_value());
    assert!(!FilterOp::IsNotNull.needs_value());
    assert!(FilterOp::Eq.needs_value());
}

#[test]
fn set_filters_use_one_parameterized_postgres_array() {
    let options = QueryOptions::new()
        .filter("status", FilterOp::In, vec!["open".to_owned(), "pending".to_owned()]);
    let (sql, next) = options.build_where_clause(1);

    assert_eq!(sql, " WHERE status = ANY($1)");
    assert_eq!(next, 2);
    assert_eq!(options.filter_params().len(), 1);
}

#[test]
fn numeric_values_are_supported() {
    let (sql, next) = QueryOptions::new()
        .filter("total_cents", FilterOp::Gte, 1000_i64)
        .filter("quantity", FilterOp::Gt, 0.5_f64)
        .build_where_clause(1);

    assert_eq!(sql, " WHERE total_cents >= $1 AND quantity > $2");
    assert_eq!(next, 3);
}

#[test]
fn from_params_defaults_and_caps_pagination() {
    use orm::query::{Pagination, Search, Sort};

    let options = QueryOptions::from_params(
        Pagination { limit: Some(500), offset: None },
        Sort { sort_by: None, sort_order: None },
        Search { query: None, fields: None },
    );

    assert_eq!(options.limit, Some(100));
    assert_eq!(options.offset, Some(0));
    assert_eq!(options.to_sql_suffix(), " LIMIT 100 OFFSET 0");
}

#[test]
fn from_params_rejects_a_sort_injection() {
    use orm::query::{Pagination, Search, Sort};

    let options = QueryOptions::from_params(
        Pagination { limit: None, offset: None },
        Sort { sort_by: Some("name; DROP TABLE users--".into()), sort_order: None },
        Search { query: None, fields: None },
    );

    assert!(!options.to_sql_suffix().contains("ORDER BY"));
}

#[test]
fn from_params_searches_only_identifier_fields() {
    use orm::query::{Pagination, Search, Sort};

    let options = QueryOptions::from_params(
        Pagination { limit: None, offset: None },
        Sort { sort_by: None, sort_order: None },
        Search { query: Some("ana".into()), fields: Some("name, evil()".into()) },
    );

    let (sql, _) = options.build_where_clause(1);
    assert_eq!(sql, " WHERE name ILIKE $1");
}

#[test]
fn from_params_sorts_by_a_valid_field() {
    use orm::query::{Pagination, Search, Sort};

    let options = QueryOptions::from_params(
        Pagination { limit: None, offset: None },
        Sort { sort_by: Some("created_at".into()), sort_order: Some(SortOrder::Desc) },
        Search { query: None, fields: None },
    );

    assert!(options.to_sql_suffix().contains(" ORDER BY created_at DESC"));
}

#[test]
fn insert_values_accept_only_known_columns() {
    let values = InsertValues::new()
        .value("name", "Press 1".to_string())
        .value("active", true);
    let (columns, placeholders) = values
        .build(&["name", "active"])
        .unwrap();

    assert_eq!(columns, "name, active");
    assert_eq!(placeholders, "$1, $2");
    assert_eq!(values.params().len(), 2);
    assert!(InsertValues::new()
        .value("unknown", true)
        .build(&["active"])
        .is_err());
}

#[test]
fn update_values_build_bound_arithmetic_and_database_values() {
    let values = UpdateValues::new()
        .assign("status", "posted".to_string())
        .add("version", 1_i64)
        .subtract("balance", 4.5_f64)
        .database_now("posted_at")
        .null("voided_at");
    let (sql, next) = values
        .build(3, &["status", "version", "balance", "posted_at", "voided_at"])
        .unwrap();

    assert_eq!(
        sql,
        "status = $3, version = version + $4, balance = balance - $5, posted_at = now(), voided_at = NULL",
    );
    assert_eq!(next, 6);
    assert_eq!(values.params().len(), 3);
}

#[test]
fn write_values_reject_duplicate_and_unsafe_columns() {
    assert!(UpdateValues::new()
        .assign("status", "open".to_string())
        .assign("status", "closed".to_string())
        .build(1, &["status"])
        .is_err());
    assert!(UpdateValues::new()
        .assign("status = 'closed'", true)
        .build(1, &["status"])
        .is_err());
}
