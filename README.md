# orm

A hand-rolled Postgres ORM for Rust, plus the derive macros that drive it.

- `orm::table_type` / `enum_type` / `json_type` / `api_type` — derive macros that
  generate `FromRow`, postgres `ToSql`/`FromSql`, [ts-rs] bindings, and
  strongly-typed CRUD traits from your struct/enum definitions.
- `orm::FromRow` / `QueryExt` — runtime row-deserialization traits.
- `orm::query::*` — `QueryOptions`, `FilterOp`, `Sort`, `Pagination`, `Search`.
- `orm::registry::export_all_types()` — drains the inventory of TS-exportable
  types and writes them to `TS_RS_EXPORT_DIR`.
- `orm::cli::main(default_out)` — a CLI entrypoint with `export` and a
  Drizzle-style `migrate` (generate / apply / diff / revert / baseline / status).

## Use as a dependency

```toml
[dependencies]
orm = { git = "https://github.com/blessedbythestorm/orm.git", branch = "main" }
```

The `macros` crate is a private implementation detail re-exported through `orm`;
depend on `orm` only.

[ts-rs]: https://github.com/Aleph-Alpha/ts-rs
