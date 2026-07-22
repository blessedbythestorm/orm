use orm::query::{FilterGroup, FilterOp, QueryOptions, QuerySort, SortOrder};

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
