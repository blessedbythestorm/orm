use std::fmt::Write;
use std::sync::Arc;

use tokio_postgres::types::ToSql;

use super::{Pagination, Search, Sort, SortOrder};

/// The largest page a caller can request through [`QueryOptions::from_params`].
const MAX_LIMIT: u32 = 100;

/// The page size used when the caller doesn't ask for one.
const DEFAULT_LIMIT: u32 = 50;

/// True when `field` is a plain column identifier, safe to interpolate into
/// SQL. Field names arriving from query parameters (sort, search fields) must
/// pass this before they reach `ORDER BY` or `WHERE`.
fn is_identifier(field: &str) -> bool {
    let mut chars = field.chars();

    let starts_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');

    starts_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub trait FilterValue {
    fn into_filter_value(self, op: FilterOp) -> Option<Arc<dyn ToSql + Send + Sync>>;
}

impl FilterValue for String {
    fn into_filter_value(self, op: FilterOp) -> Option<Arc<dyn ToSql + Send + Sync>> {
        Some(Arc::new(op.wrap_value(&self)))
    }
}

impl FilterValue for &str {
    fn into_filter_value(self, op: FilterOp) -> Option<Arc<dyn ToSql + Send + Sync>> {
        Some(Arc::new(op.wrap_value(self)))
    }
}

impl<T: FilterValue> FilterValue for Option<T> {
    fn into_filter_value(self, op: FilterOp) -> Option<Arc<dyn ToSql + Send + Sync>> {
        self.and_then(|value| value.into_filter_value(op))
    }
}

/// Values bound as-is (the filter op only matters for text matching).
macro_rules! impl_direct_filter_value {
    ($($ty:ty),+ $(,)?) => {$(
        impl FilterValue for $ty {
            fn into_filter_value(self, _op: FilterOp) -> Option<Arc<dyn ToSql + Send + Sync>> {
                Some(Arc::new(self))
            }
        }
    )+};
}

impl_direct_filter_value!(i32, bool, uuid::Uuid);

#[derive(Debug, Clone, Copy, Default)]
pub enum LogicalOp {
    #[default]
    And,
    Or,
}

impl LogicalOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogicalOp::And => " AND ",
            LogicalOp::Or => " OR ",
        }
    }
}

#[derive(Default, Debug)]
pub struct QueryOptions {
    pub groups: Vec<FilterGroup>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub sort_by: Option<String>,
    pub sort_order: Option<SortOrder>,
}

#[derive(Debug, Default)]
pub struct FilterGroup {
    pub filters: Vec<Filter>,
    pub op: LogicalOp,
}

impl FilterGroup {
    pub fn and() -> Self {
        Self { filters: Vec::new(), op: LogicalOp::And }
    }

    pub fn or() -> Self {
        Self { filters: Vec::new(), op: LogicalOp::Or }
    }

    pub fn filter<T: FilterValue>(mut self, field: impl Into<String>, op: FilterOp, value: T) -> Self {
        if let Some(converted) = value.into_filter_value(op) {
            self.filters.push(Filter { field: field.into(), op, value: converted });
        }
        self
    }
}

impl From<Search> for FilterGroup {
    fn from(search: Search) -> Self {
        let mut group = FilterGroup::or();
        let Some(query) = search.query.clone() else {
            return group;
        };
        for field in search.field_list() {
            if !is_identifier(&field) {
                continue;
            }
            if let Some(converted) = query.clone().into_filter_value(FilterOp::ILike) {
                group.filters.push(Filter { field, op: FilterOp::ILike, value: converted });
            }
        }
        group
    }
}

#[derive(Debug)]
pub struct Filter {
    pub field: String,
    pub op: FilterOp,
    pub value: Arc<dyn ToSql + Send + Sync>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    ILike,
    IsNull,
    IsNotNull,
}

impl FilterOp {
    pub fn as_sql(&self) -> &'static str {
        match self {
            FilterOp::Eq => "=",
            FilterOp::Ne => "!=",
            FilterOp::Gt => ">",
            FilterOp::Gte => ">=",
            FilterOp::Lt => "<",
            FilterOp::Lte => "<=",
            FilterOp::Like => "LIKE",
            FilterOp::ILike => "ILIKE",
            FilterOp::IsNull => "IS NULL",
            FilterOp::IsNotNull => "IS NOT NULL",
        }
    }

    pub fn wrap_value(&self, value: &str) -> String {
        match self {
            FilterOp::Like | FilterOp::ILike => format!("%{}%", value),
            _ => value.to_string(),
        }
    }

    pub fn needs_value(&self) -> bool {
        !matches!(self, FilterOp::IsNull | FilterOp::IsNotNull)
    }
}

#[derive(Debug)]
pub struct QuerySort {
    pub field: String,
    pub order: SortOrder,
}

impl QuerySort {
    pub fn new(field: impl Into<String>, order: SortOrder) -> Self {
        Self { field: field.into(), order }
    }
}

impl QueryOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds options straight from the standard list query parameters:
    /// pagination (defaulted and capped), an optional sort, and an optional
    /// free-text search. Sort and search field names are ignored unless they
    /// are plain identifiers — they end up interpolated into SQL, so nothing
    /// else may pass.
    pub fn from_params(pagination: Pagination, sort: Sort, search: Search) -> Self {
        let mut options = Self::new()
            .limit(pagination.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT))
            .offset(pagination.offset.unwrap_or(0))
            .filter_group(search.into());

        if let Some(field) = sort.sort_by.filter(|field| is_identifier(field)) {
            options = options.sort(QuerySort::new(field, sort.sort_order.unwrap_or_default()));
        }

        options
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn sort(mut self, sort: QuerySort) -> Self {
        self.sort_by = Some(sort.field);
        self.sort_order = Some(sort.order);
        self
    }

    pub fn filter<T: FilterValue>(mut self, field: impl Into<String>, op: FilterOp, value: T) -> Self {
        if let Some(converted) = value.into_filter_value(op) {
            self.groups.push(FilterGroup {
                filters: vec![Filter { field: field.into(), op, value: converted }],
                op: LogicalOp::And,
            });
        }
        self
    }

    pub fn filter_group(mut self, group: FilterGroup) -> Self {
        if !group.filters.is_empty() {
            self.groups.push(group);
        }
        self
    }

    pub fn build_where_clause(&self, param_offset: usize) -> (String, usize) {
        let non_empty_groups: Vec<_> = self.groups.iter().filter(|g| !g.filters.is_empty()).collect();

        if non_empty_groups.is_empty() {
            return (String::new(), param_offset);
        }

        let mut param_idx = param_offset;
        let mut group_conditions = Vec::new();

        for group in &non_empty_groups {
            let conditions: Vec<_> = group
                .filters
                .iter()
                .map(|f| {
                    if f.op.needs_value() {
                        let s = format!("{} {} ${}", f.field, f.op.as_sql(), param_idx);
                        param_idx += 1;
                        s
                    } else {
                        format!("{} {}", f.field, f.op.as_sql())
                    }
                })
                .collect();

            if conditions.len() == 1 {
                group_conditions.push(conditions.into_iter().next().unwrap());
            } else {
                group_conditions.push(format!("({})", conditions.join(group.op.as_str())));
            }
        }

        (format!(" WHERE {}", group_conditions.join(" AND ")), param_idx)
    }

    pub fn filter_params(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.groups
            .iter()
            .flat_map(|g| g.filters.iter())
            .filter(|f| f.op.needs_value())
            .map(|f| f.value.as_ref() as &(dyn ToSql + Sync))
            .collect()
    }

    pub fn to_sql_suffix(&self) -> String {
        let mut sql = String::new();
        if let Some(field) = self.sort_by.as_deref().filter(|field| is_identifier(field)) {
            let _ = write!(sql, " ORDER BY {} {}", field, self.sort_order.unwrap_or_default().as_str());
        }
        if let Some(limit) = self.limit {
            let _ = write!(sql, " LIMIT {}", limit);
        }
        if let Some(offset) = self.offset {
            let _ = write!(sql, " OFFSET {}", offset);
        }
        sql
    }
}
