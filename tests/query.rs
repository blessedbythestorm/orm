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
