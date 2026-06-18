//! orm: a hand-rolled Postgres ORM for Rust.
//!
//! - `orm::table_type` / `enum_type` / `json_type` / `api_type` — derive macros
//!   that generate `FromRow`, postgres `ToSql`/`FromSql`, ts-rs bindings, and
//!   strongly-typed CRUD traits from your struct/enum definitions.
//! - `orm::FromRow` / `QueryExt` — runtime row-deserialization traits.
//! - `orm::query::*` — `QueryOptions`, `FilterOp`, `Sort`, `Pagination`, `Search`.
//! - `orm::registry::export_all_types()` — drains the inventory of TS-exportable
//!   types and writes them to `TS_RS_EXPORT_DIR`.

// Lets the proc macros emit absolute `::orm::*` paths that resolve both for
// external consumers and for orm's own types defined in this crate.
extern crate self as orm;

pub mod cli;
pub mod client;
pub mod migrate;
pub mod query;
pub mod registry;
pub mod schema;
pub mod style;
pub mod traits;

pub use macros::{api_type, endpoint, enum_type, json_type, table_type};
pub use registry::{TypeExport, export_all_types};
pub use traits::{FromRow, QueryExt};
