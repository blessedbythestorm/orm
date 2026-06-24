//! orm: a hand-rolled Postgres ORM for Rust.
//!
//! - `orm::table_type` / `enum_type` / `json_type` / `api_type` — derive macros
//!   that generate `FromRow`, postgres `ToSql`/`FromSql`, TypeScript bindings, and
//!   strongly-typed CRUD traits from your struct/enum definitions.
//! - `orm::FromRow` / `QueryExt` — runtime row-deserialization traits.
//! - `orm::query::*` — `QueryOptions`, `FilterOp`, `Sort`, `Pagination`, `Search`.
//! - `orm::registry::export_all_types()` — drains the inventory of TS-exportable
//!   types and writes them to the output dir.

// Lets the proc macros emit absolute `::orm::*` paths that resolve both for
// external consumers and for orm's own types defined in this crate.
extern crate self as orm;

pub mod cli;
pub mod export;
pub mod lang;
pub mod migrate;
pub mod query;
pub mod registry;
pub mod schema;
pub mod style;
pub mod traits;
pub mod validate;
pub mod validator;

pub use export::{ExportBackend, ExportType, export_all_types};
pub use macros::{api_type, endpoint, enum_type, json_type, table_type, view_type};
pub use traits::{FromRow, QueryExt};
pub use validate::{Valid, Validate, ValidationError, ValidationErrors};

