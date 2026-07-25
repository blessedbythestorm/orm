//! Metadata the macros register via `inventory`, drained by the code generators
//! in [`crate::lang`].

use crate::export::FieldType;

/// A request/response/query type referenced by an endpoint, in the neutral model.
pub struct TypeRef {
    pub ty: &'static FieldType,
}

/// One HTTP endpoint, registered by `#[endpoint(METHOD, "/path")]`. A handler
/// may take several `Query<T>` extractors (e.g. `Pagination`, `Sort`,
/// `Search`); the client types their intersection as one query argument.
pub struct EndpointMeta {
    pub method: &'static str,
    pub path: &'static str,
    pub name: &'static str,
    pub request: Option<TypeRef>,
    pub queries: &'static [TypeRef],
    pub response: Option<TypeRef>,
    /// The handler takes a `Multipart` body: a file, plus any number of text
    /// fields. Generated clients send `FormData` and set no content type, so the
    /// browser can supply the boundary.
    pub upload: bool,
}

inventory::collect!(EndpointMeta);
