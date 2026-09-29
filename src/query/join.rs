use anyhow::{Result, bail};
use tokio_postgres::types::ToSql;

use super::builder::is_identifier;
use super::{FilterGroup, FilterOp, FilterValue, QueryModel, QueryOptions};

/// Structured ON conditions: column comparisons and bound value predicates.
/// All comparisons and filter groups are ANDed together.
#[derive(Debug, Default)]
pub struct JoinOn {
    columns: Vec<(String, FilterOp, String)>,
    options: QueryOptions,
}

impl JoinOn {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn eq(left: impl Into<String>, right: impl Into<String>) -> Self {
        Self::new()
            .and_on(left, FilterOp::Eq, right)
    }

    /// Adds a column comparison. Supports Eq, Ne, Gt, Gte, Lt, and Lte.
    pub fn and_on(
        mut self,
        left: impl Into<String>,
        op: FilterOp,
        right: impl Into<String>,
    ) -> Self {
        self.columns.push((left.into(), op, right.into()));
        self
    }

    pub fn where_<V: FilterValue>(
        mut self,
        field: impl Into<String>,
        op: FilterOp,
        value: V,
    ) -> Self {
        self.options = self.options.filter(field, op, value);
        self
    }

    pub fn where_group(mut self, group: FilterGroup) -> Self {
        self.options = self.options.filter_group(group);
        self
    }

    pub fn where_null(mut self, field: impl Into<String>) -> Self {
        self.options = self.options.is_null(field);
        self
    }

    pub fn where_not_null(mut self, field: impl Into<String>) -> Self {
        self.options = self.options.is_not_null(field);
        self
    }

    fn sql(&self, scope: &Scope, offset: usize) -> Result<(String, usize)> {
        let mut parts = Vec::new();
        for (left, op, right) in &self.columns {
            if !matches!(op, FilterOp::Eq | FilterOp::Ne | FilterOp::Gt | FilterOp::Gte | FilterOp::Lt | FilterOp::Lte) {
                bail!("unsupported column comparison in JOIN ON");
            }

            parts.push(format!("{} {} {}", scope.column(left)?, op.as_sql(), scope.column(right)?));
        }

        let options = scope.options(&self.options)?;
        let (predicate, next) = options.build_where_clause(offset);
        if let Some(predicate) = predicate.strip_prefix(" WHERE ") {
            parts.push(predicate.to_string());
        }

        if parts.is_empty() {
            bail!("JOIN requires a nonempty ON condition");
        }

        Ok((parts.join(" AND "), next))
    }
}

pub(crate) struct Join {
    relation: &'static str,
    columns: &'static [&'static str],
    alias: String,
    left: bool,
    on: JoinOn,
}

impl Join {
    pub(crate) fn new<M: QueryModel>(alias: impl Into<String>, left: bool, on: JoinOn) -> Self {
        Self {
            relation: M::RELATION,
            columns: M::COLUMNS,
            alias: alias.into(),
            left,
            on,
        }
    }
}

#[derive(Default)]
pub(crate) struct ReadQuery {
    pub(crate) alias: Option<String>,
    pub(crate) joins: Vec<Join>,
    pub(crate) projection: Option<Vec<(String, String)>>,
}

impl ReadQuery {
    pub(crate) fn sql<M: QueryModel>(&self, options: &QueryOptions, count: bool) -> Result<String> {
        let base = self.alias.as_deref()
            .unwrap_or_else(
                || M::RELATION.rsplit('.')
                    .next()
                    .unwrap()
            );

        let mut scope = Scope::default();
        scope.add(base, M::COLUMNS)?;
        let mut from = format!("{} AS \"{}\"", M::RELATION, base);
        let mut next = 1;
        for join in &self.joins {
            scope.add(&join.alias, join.columns)?;
            let (on, offset) = join.on.sql(&scope, next)?;
            next = offset;
            from.push_str(&format!(" {} JOIN {} AS \"{}\" ON {}", if join.left { "LEFT" } else { "INNER" }, join.relation, join.alias, on));
        }

        let mut options = options.clone();
        if count {
            options.limit = None;
            options.offset = None;
            options.sort_by = None;
            options.secondary_sorts.clear();
            options.row_lock = None;
        }

        let options = scope.options(&options)?;
        let (predicate, _) = options.build_where_clause(next);
        let columns = if count {
            "COUNT(*) AS count".to_string()
        } else if let Some(projection) = &self.projection {
            if projection.is_empty() {
                bail!("projection requires at least one column");
            }

            let mut seen = Vec::new();
            let mut columns = Vec::new();
            for (field, alias) in projection {
                validate_alias(alias)?;
                if seen.contains(&alias) {
                    bail!("duplicate projection alias `{alias}`");
                }

                seen.push(alias);
                columns.push(format!("{} AS \"{}\"", scope.column(field)?, alias));
            }

            columns.join(", ")
        } else {
            M::COLUMNS.iter()
                .map(|column| Ok(format!("{} AS \"{}\"", scope.column(column)?, column)))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        };

        let mut suffix = options.to_sql_suffix_with(|field| Some(field.to_string()));
        if options.row_lock.is_some() {
            suffix.push_str(&format!(" OF \"{base}\""));
        }

        Ok(format!("SELECT {columns} FROM {from}{predicate}{suffix}"))
    }

    pub(crate) fn params<'a>(&'a self, options: &'a QueryOptions) -> Vec<&'a (dyn ToSql + Sync)> {
        let mut params = Vec::new();
        for join in &self.joins {
            params.extend(join.on.options.filter_params());
        }

        params.extend(options.filter_params());
        params
    }
}

#[derive(Default)]
struct Scope<'a> {
    relations: Vec<(&'a str, &'static [&'static str])>,
}

impl<'a> Scope<'a> {
    fn add(&mut self, alias: &'a str, columns: &'static [&'static str]) -> Result<()> {
        validate_alias(alias)?;
        if self.relations.iter()
            .any(|(existing, _)| *existing == alias)
        {
            bail!("duplicate relation alias `{alias}`");
        }

        self.relations.push((alias, columns));
        Ok(())
    }

    fn column(&self, field: &str) -> Result<String> {
        let (alias, column) = field.split_once('.')
            .unwrap_or((self.relations[0].0, field));

        let Some((_, columns)) = self.relations.iter()
            .find(|(candidate, _)| *candidate == alias) else {
                bail!("unknown relation alias `{alias}`");
            };

        if !is_identifier(column) || !columns.contains(&column) {
            bail!("unknown query column `{field}`");
        }

        Ok(format!("\"{alias}\".\"{column}\""))
    }

    fn options(&self, options: &QueryOptions) -> Result<QueryOptions> {
        let mut options = options.clone();
        for filter in options.groups.iter_mut()
            .flat_map(|group| &mut group.filters)
        {
            if filter.op.needs_value() != filter.value.is_some() {
                bail!("invalid value for filter on `{}`", filter.field);
            }

            filter.field = self.column(&filter.field)?;
        }

        if let Some(field) = &mut options.sort_by {
            *field = self.column(field)?;
        }

        for sort in &mut options.secondary_sorts {
            sort.field = self.column(&sort.field)?;
        }

        Ok(options)
    }
}

fn validate_alias(alias: &str) -> Result<()> {
    if !is_identifier(alias) || alias.len() > 63 {
        bail!("invalid SQL alias `{alias}`");
    }

    Ok(())
}
