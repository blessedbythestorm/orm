# orm

`orm` is a PostgreSQL ORM and schema/code-generation toolkit for Rust. It
generates row decoding, CRUD code, PostgreSQL conversions, API validation,
TypeScript types, HTTP client metadata, and migrations from Rust definitions.

It uses `tokio-postgres` and `deadpool-postgres`. SQL remains visible: generated
queries and migration files are ordinary SQL and can be reviewed or replaced
with handwritten queries.

## Install and create an Axum project

`orm` is currently consumed from Git. Use `orm` only; the `macros` crate is an
implementation detail re-exported by it.

```toml
[dependencies]
orm = { git = "https://github.com/blessedbythestorm/orm.git", branch = "main" }

# Direct dependencies used by macro expansions and the server.
anyhow = "1"
axum = "0.8"
bytes = "1"
chrono = { version = "0.4", features = ["serde"] }
deadpool-postgres = "0.14"
inventory = "0.3"
postgres-types = "0.2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "net", "time"] }
tokio-postgres = { version = "0.7", features = [
  "with-chrono-0_4",
  "with-serde_json-1",
  "with-uuid-1",
] }
uuid = { version = "1", features = ["serde", "v4", "v7"] }
```

The generated code refers to crates such as `serde`, `inventory`,
`tokio_postgres`, and `deadpool_postgres` by name. Keep the crates used by your
macros as direct dependencies.

Create the project and add a library target:

```sh
cargo new account-api
cd account-api
```

```text
src/
├── bin/
│   └── orm-cli.rs  # migration and code-generation binary
├── lib.rs          # shared models and macro declarations
└── main.rs         # Axum server binary
```

There are two binaries. `src/main.rs` is the server and runs with `cargo run`.
`src/bin/orm-cli.rs` runs migrations and TypeScript generation with
`cargo run --bin orm-cli -- ...`. Keep them separate: `orm::cli::main` parses
CLI arguments and exits; it is not the Axum server entrypoint. Both binaries
must link `lib.rs` so `inventory` can see the same model and endpoint metadata.

Put the macro declarations from the next section in `src/lib.rs`. The server
can then share the generated `AccountCrud` implementation through Axum state:

```rust
// src/main.rs
use account_api::{Account, AccountCrud}; // replace with your package name
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use deadpool_postgres::{Config, Runtime};
use orm::query::QueryOptions;
use tokio::net::TcpListener;
use tokio_postgres::NoTls;

async fn get_accounts(
    State(pool): State<deadpool_postgres::Pool>,
) -> Result<Json<Vec<Account>>, (StatusCode, String)> {
    pool.get_accounts(QueryOptions::new().limit(50))
        .await
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn build_pool() -> anyhow::Result<deadpool_postgres::Pool> {
    let mut config = Config::new();
    config.url = Some(std::env::var("DATABASE_URL")?);
    Ok(config.create_pool(Some(Runtime::Tokio1), NoTls)?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pool = build_pool()?;
    let app = Router::new()
        .route("/accounts", get(get_accounts))
        .with_state(pool);

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

`State<Pool>` is the Axum/ORM binding. Generated CRUD methods acquire a client
from the pool; handlers should not open a new PostgreSQL connection per request.

The CLI binary must import the library crate so its inventory submissions are
linked:

```rust
// src/bin/orm-cli.rs
use account_api as _; // replace with your package name

fn main() -> std::process::ExitCode {
    orm::cli::main(concat!(env!("CARGO_MANIFEST_DIR"), "/generated"))
}
```

After adding the models, create the schema before starting the server:

```sh
export DATABASE_URL=postgres://localhost/account_api
cargo run --bin orm-cli -- migrate generate create_accounts
cargo run --bin orm-cli -- migrate apply
cargo run
```

## Macros

All macros below are re-exported from `orm` and are attribute macros.

| Macro | Represents | Main generated code |
| --- | --- | --- |
| `table_type` | A PostgreSQL table | Rust row type, insert/update types, CRUD, schema metadata |
| `enum_type` | A PostgreSQL enum | Serde/Postgres conversions, filters, TypeScript union, schema metadata |
| `json_type` | A typed `json`/`jsonb` value | Serde/Postgres conversions, TypeScript type, schema metadata |
| `view_type` | A read-only PostgreSQL view | Row type, view query trait, TypeScript type, schema metadata |
| `api_type` | An HTTP request/response type | Serde, validation, TypeScript type, validator schema |
| `endpoint` | HTTP route metadata | Request/query/response metadata for the generated client |

### `#[table_type]`: PostgreSQL tables

Use it on a named-field struct to represent a physical PostgreSQL table:

```rust
use uuid::Uuid;

#[table_type(
    schema = "public",
    name = "accounts",
    export_to = "types/database/accounts.ts"
)]
pub struct Account {
    #[pg(primary, default(sql("gen_random_uuid()")))]
    #[crud(insert(skip))]
    pub id: Uuid,
    #[pg(unique)]
    pub email: String,
}
```

`schema`, `name`, and `export_to` are required. The macro generates:

- `AccountInsert`, `AccountUpdate`, and `AccountCrud`;
- `orm::FromRow`, Serde, and `orm::Validate` implementations;
- TypeScript export metadata and migration schema metadata;
- `get_accounts`, `get_account`, `create_account`, `update_account`, and
  `delete_account` on `deadpool_postgres::Pool`.

Generated CRUD assumes an `id: uuid::Uuid` column and uses `WHERE id = $1`.
For another key shape, use `FromRow`/`QueryExt` or write a repository manually.
The database must provide defaults for generated columns such as the primary
key and `created_at`.

Useful field attributes are:

```rust
#[pg(unique)]
#[pg(default("active"))]                         // literal default
#[pg(default(sql("now()")))]                     // raw SQL default
#[pg(foreign(public.users.id, on_delete(cascade)))]
#[crud(insert(optional))]
#[crud(insert(skip), update(skip))]
```

`Option<T>` maps to a nullable column. Built-in `SqlType` mappings include
`bool`, integer/float types, `String`, `Vec<u8>`, `uuid::Uuid`, common `chrono`
types, and `serde_json::Value`. Custom enum and JSON types are covered below.

### `#[enum_type]`: PostgreSQL enums

Use it on an enum to represent a PostgreSQL enum type:

```rust
#[enum_type(
    schema = "public",
    name = "account_status",
    export_to = "types/database/accounts.ts"
)]
pub enum AccountStatus {
    Active,
    #[postgres(name = "suspended_account")]
    Suspended,
}
```

`name` and `export_to` are required; `schema` defaults to `public`. Variants
use snake-case names as PostgreSQL and JSON values unless `#[postgres(name)]`
overrides one. The macro implements Serde, `postgres_types::ToSql`/
`FromSql`, `orm::schema::SqlType`, and `query::FilterValue`, and registers the
enum for TypeScript and migrations.

### `#[json_type]`: typed JSONB values

Use it on a named-field struct stored in a `json` or `jsonb` column:

```rust
#[json_type(export_to = "types/database/accounts.ts")]
pub struct AccountProfile {
    pub display_name: String,
    pub avatar_url: Option<String>,
}
```

`export_to` is required. The macro derives Serde, implements PostgreSQL JSON
conversion and `SqlType` (`jsonb`), and registers a TypeScript object type.
Use `Option<AccountProfile>` for a nullable JSONB column.

### `#[view_type]`: read-only views

Use it on a projection struct. Every field names its source column with
`#[pg(view(...))]`:

```rust
#[view_type(
    schema = "public",
    name = "account_cards",
    export_to = "types/database/accounts.ts",
    filter = "accounts.email IS NOT NULL",
    order_by = "accounts.email ASC"
)]
pub struct AccountCard {
    #[pg(view(public.accounts.id))]
    pub id: uuid::Uuid,
    #[pg(view(public.accounts.email))]
    pub email: String,
}
```

`schema`, `name`, and `export_to` are required. The first source column supplies
the base table. Sources from other tables are joined through foreign keys
declared with `#[pg(foreign(...))]`. The macro generates `FromRow`, a
TypeScript type, and `AccountCardView::get_account_cards(QueryOptions)` for
`deadpool_postgres::Pool`. `filter` and `order_by` are raw SQL.

### `#[api_type]`: validated API types

Use it on a request/response struct or enum. `export_to` is required:

```rust
#[api_type(export_to = "types/api/accounts.ts")]
pub struct CreateAccount {
    #[api(validate(email))]
    pub email: String,
    #[api(validate(length(min(12), max(128))))]
    pub password: String,
    #[api(validate(range(min(1), max(100))))]
    pub seats: Option<u32>,
}
```

The macro derives Serde, implements `orm::Validate`, exports a TypeScript
type, and registers a Valibot schema. Runtime rules are `email`, `required`,
`length(min(...), max(...), equal(...))`, `range(min(...), max(...))`, and
`regex(r"...")`. `orm::Valid<Json<T>>` and `orm::Valid<Query<T>>` run these
checks as Axum extractors and return `400` field-error responses.

### `#[endpoint]`: HTTP client metadata

Use it on an Axum handler to register its method, path, request/query types,
and JSON response type:

```rust
use axum::{extract::Json, http::StatusCode};
use orm::{endpoint, Valid};

#[endpoint(POST, "/accounts", "accounts.create")]
async fn create_account(
    Valid(Json(request)): Valid<Json<CreateAccount>>,
) -> Result<Json<Account>, StatusCode> {
    let _ = request;
    todo!()
}
```

The optional third argument is the client method name; without it, a name is
derived from the method and path. `Valid<Json<T>>` and `Valid<Query<T>>` are
recognized automatically, as are `Json<T>` responses nested in `Result`.
`#[endpoint]` does not register the route with Axum—add the handler to your
`Router` yourself.

## Queries and typed rows

Tables and views implement `FromRow`. `QueryExt` adds typed query methods to
both `tokio_postgres::Client` and `deadpool_postgres::Client`:

```rust
use orm::QueryExt;

let client = pool.get().await?;
let account: Option<Account> = client
    .query_opt_typed(
        "SELECT id, email FROM public.accounts WHERE id = $1",
        &[&account_id],
    )
    .await?;
```

Selected column names must match the Rust fields because generated `FromRow`
uses `row.try_get("field_name")`.

`QueryOptions` supplies filters, grouped `AND`/`OR` conditions, sorting,
limits, and offsets to generated CRUD and view queries:

```rust
use orm::query::{FilterGroup, FilterOp, QueryOptions, QuerySort, SortOrder};

let options = QueryOptions::new()
    .filter_group(
        FilterGroup::or()
            .filter("email", FilterOp::ILike, "example")
            .filter("status", FilterOp::Eq, AccountStatus::Suspended),
    )
    .sort(QuerySort::new("email", SortOrder::Asc))
    .limit(25)
    .offset(50);

let accounts = pool.get_accounts(options).await?;
```

Values are parameterized. Field names, sort names, and raw view/table SQL are
not; whitelist any identifier derived from user input. Built-in filter values
are strings, `i32`, `bool`, `Uuid`, `Option<T>`, and `#[enum_type]` enums.

## TypeScript generation

The `orm-cli` binary from the setup section runs the built-in TypeScript
generator:

```sh
cargo run --bin orm-cli -- generate --lang ts
cargo run --bin orm-cli -- generate --lang ts --out ./frontend/src/lib
```

It writes each type to its `export_to` path, plus:

- `schema/schemas.ts` for Valibot schemas;
- `service/client.ts` for `#[endpoint]` handlers;
- `lib/result.ts` for the generated `Result` runtime.

An endpoint named `accounts.create` is used like this:

```ts
const api = createApi({ baseUrl: "/api" });
const result = await api.accounts.create({
  email: "ana@example.com",
  password: "a sufficiently long password",
  seats: 5,
});
```

Use `orm::export::export_all_types` and implement `ExportBackend` for a custom
language backend. The current CLI language is TypeScript.

## Migrations

Tables, enums, and views register schema metadata. The CLI diffs that metadata
against snapshots and writes `.up.sql`, `.down.sql`, and `meta/*.json` files:

```sh
cargo run --bin orm-cli -- migrate generate create_accounts
DATABASE_URL=postgres://localhost/account_api \
  cargo run --bin orm-cli -- migrate apply
cargo run --bin orm-cli -- migrate status
cargo run --bin orm-cli -- migrate diff
cargo run --bin orm-cli -- migrate diff --write reconcile_database
```

`migrate baseline <name>` adopts an existing database without running SQL.
`migrate revert` runs the latest down migration and removes its files. Review
generated SQL before applying it; ambiguous renames are interactive by default,
and enum values cannot be removed by the generated down migration.

## Lower-level schema APIs

For custom tooling, use the schema model directly:

```rust
use orm::schema::{assemble_desired_schema, diff, render, DatabaseSchema, NoRenames};

let baseline = DatabaseSchema::default();
let desired = assemble_desired_schema();
let sql = render(&diff(&baseline, &desired, &mut NoRenames));
```
