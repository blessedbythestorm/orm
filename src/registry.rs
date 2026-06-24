//! Metadata the macros register via `inventory`, drained by the code generators
//! in [`crate::lang`].

use crate::export::FieldType;

/// A request/response/query type referenced by an endpoint, in the neutral model.
pub struct TypeRef {
    pub ty: &'static FieldType,
}

/// One HTTP endpoint, registered by `#[endpoint(METHOD, "/path")]`.
pub struct EndpointMeta {
    pub method: &'static str,
    pub path: &'static str,
    pub name: &'static str,
    pub request: Option<TypeRef>,
    pub query: Option<TypeRef>,
    pub response: Option<TypeRef>,
}

inventory::collect!(EndpointMeta);
