mod builder;

pub use builder::*;

use crate::api_type;

#[api_type(export_to = "types/api/query.ts")]
pub struct Pagination {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[api_type(export_to = "types/api/query.ts")]
pub struct Sort {
    pub sort_by: Option<String>,
    pub sort_order: Option<SortOrder>,
}

#[api_type(export_to = "types/api/query.ts")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_str(&self) -> &'static str {
        match self {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        }
    }
}

#[api_type(export_to = "types/api/query.ts")]
pub struct Search {
    pub query: Option<String>,
    pub fields: Option<String>,
}

impl Search {
    /// The `fields` value split into individual, trimmed column names.
    pub fn field_list(&self) -> Vec<String> {
        self.fields
            .as_deref()
            .map(|fields| {
                fields
                    .split(',')
                    .map(|field| field.trim().to_string())
                    .filter(|field| !field.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}
