# orm

`orm` is a convention-driven PostgreSQL toolkit for Rust. Rust structs and enums
are the source of truth for database schema, row decoding, CRUD inputs, API
validation, and generated TypeScript bindings.

The crate is built on `tokio-postgres` and `deadpool-postgres`. It does not hide
SQL: generated queries and migration SQL are ordinary SQL strings that can be
reviewed, tested, and supplemented with handwritten queries.

## Install and wire up a new Axum project

`orm` is currently consumed from Git. The derive macros are re-exported by
`orm`; do not depend on the private `macros` crate directly.

```toml
[dependencies]
orm = { git = "https://github.com/blessedbythestorm/orm.git", branch = "main" }

# Crates referenced by the generated code. Keep these as direct dependencies.
anyhow = "1"
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

# Needed by the API examples, not by the database-only parts.
axum = "0.8"
```

The generated implementations refer to dependencies such as `serde`,
`tokio_postgres`, `deadpool_postgres`, and `inventory` by their normal crate
names. That is why the crates used by your selected macros must be direct
dependencies of the consuming package.

### Create the application shell

Start with an ordinary Axum application. The library target keeps the model
definitions available to both the HTTP server and the ORM CLI; replace
`account_api` below with your package's Rust crate name.

```sh
cargo new account-api
cd account-api
```

Use this layout:

```text
src/
├── bin/
│   └── orm-cli.rs
├── lib.rs
└── main.rs
```

Put the model definitions from the next section in `src/lib.rs`. A minimal
`src/main.rs` can create one PostgreSQL pool, put it in Axum state, and call the
generated CRUD trait from a handler:

```rust
use account_api::{Account, AccountCrud};
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

`State<Pool>` is the binding point between Axum and `orm`: the generated CRUD
implementation is for `deadpool_postgres::Pool`, and each call checks out a
client from that pool. Do not create a new PostgreSQL connection inside every
handler. For a handwritten query, check out a client from the same state and
use `orm::QueryExt` on it.

The CLI binary must link the library containing the macro declarations so
`inventory` can see their schema and endpoint metadata:

```rust
// src/bin/orm-cli.rs
use account_api as _; // intentionally keeps the model crate linked

fn main() -> std::process::ExitCode {
    orm::cli::main(concat!(env!("CARGO_MANIFEST_DIR"), "/generated"))
}
```

After adding the model code, create and apply its schema before starting the
server:

```sh
export DATABASE_URL=postgres://localhost/account_api

cargo run --bin orm-cli -- migrate generate create_accounts
cargo run --bin orm-cli -- migrate apply
cargo run

# In another terminal:
curl http://127.0.0.1:3000/accounts
```

The same pool can be used with `Valid<Json<T>>` or `Valid<Query<T>>` in an API
handler; the validation and route metadata example appears below.

## Define the domain types

The following `src/lib.rs` example is a small domain model.
`#[enum_type]`, `#[json_type]`, and `#[table_type]` register the schema and
generate the serialization, PostgreSQL, TypeScript, and CRUD code needed by the
example.

```rust
use chrono::{DateTime, Utc};
use orm::{enum_type, json_type, table_type};
use uuid::Uuid;

#[enum_type(
    schema = "public",
    name = "account_status",
    export_to = "types/database/accounts.ts"
)]
pub enum AccountStatus {
    Active,
    Suspended,
}

#[json_type(export_to = "types/database/accounts.ts")]
pub struct AccountProfile {
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[table_type(
    schema = "public",
    name = "accounts",
    export_to = "types/database/accounts.ts"
)]
pub struct Account {
    // `create_account` omits primary-key columns, so the database must generate it.
    #[pg(primary, default(sql("gen_random_uuid()")))]
    #[crud(insert(skip))]
    pub id: Uuid,
    pub email: String,
    pub status: AccountStatus,
    pub profile: AccountProfile,
    // This column is also generated by PostgreSQL and is not an insert/update field.
    #[pg(default(sql("now()")))]
    #[crud(insert(skip), update(skip))]
    pub created_at: DateTime<Utc>,
}
```

From this one table definition, the macro generates `AccountInsert`,
`AccountUpdate`, `AccountCrud`, a `FromRow` implementation, a `Validate`
implementation, and a TypeScript type export. `AccountCrud` is implemented for
`deadpool_postgres::Pool` and provides `get_accounts`, `get_account`,
`create_account`, `update_account`, and `delete_account`.

The generated table CRUD follows a deliberate convention: the table must have
an `id: uuid::Uuid` column. The generated `get_*`, `update_*`, and `delete_*`
methods use `WHERE id = $1`. For a different key shape, use the lower-level
`FromRow` and `QueryExt` APIs or write a repository by hand.

The `gen_random_uuid()` and `now()` expressions above must be available in the
target PostgreSQL database. `default(sql(...))` is copied into migration SQL;
it does not execute or validate the expression during compilation.

## Tables, schema metadata, and CRUD

`#[table_type]` requires `schema`, `name`, and `export_to`. Field attributes
describe the PostgreSQL schema and the generated input types:

```rust
#[table_type(
    schema = "public",
    name = "projects",
    export_to = "types/database/projects.ts"
)]
pub struct Project {
    #[pg(primary, default(sql("gen_random_uuid()")))]
    #[crud(insert(skip))]
    pub id: uuid::Uuid,

    #[pg(unique)]
    pub slug: String,

    #[pg(foreign(public.accounts.id, on_delete(cascade)))]
    pub account_id: uuid::Uuid,

    #[crud(insert(skip), update(skip))]
    #[pg(default(sql("now()")))]
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

Supported PostgreSQL scalar mappings include `bool`, `i16`, `i32`, `i64`,
`f32`, `f64`, `String`, `Vec<u8>`, `uuid::Uuid`, `chrono::NaiveDate`,
`chrono::NaiveDateTime`, `chrono::DateTime<chrono::Utc>`, and
`serde_json::Value`. `Option<T>` makes the column nullable. `#[enum_type]`
and `#[json_type]` add the corresponding mappings for custom PostgreSQL enum
and JSONB types.

The field controls have these effects:

- `#[pg(primary)]` marks a primary-key column. Under the generated CRUD
  convention it is treated as database-generated and omitted from create and
  update SQL, so give it a database default. Add `#[crud(insert(skip))]` when
  the primary key should also be omitted from the generated `NameInsert`
  struct.
- `#[pg(unique)]` adds a `UNIQUE` constraint to generated schema SQL.
- `#[pg(default(...))]` accepts a literal such as `default("active")` or raw
  SQL such as `default(sql("now()"))`.
- `#[pg(foreign(schema.table.column, on_update(...), on_delete(...)))]` adds a
  foreign key and its referential actions.
- `#[crud(insert(optional))]` makes a generated insert field an `Option<T>`.
- `#[crud(insert(skip))]` removes a field from the generated insert type and
  generated create/update field lists; use it for a database-generated value.
- `#[crud(update(skip))]` removes a field from generated updates.

All table fields must implement both `tokio-postgres` row/value conversion and
`orm::schema::SqlType`. The built-in mappings and the custom-type macros cover
the common cases; implement those traits yourself for a custom database type.

## Typed rows and query options

`FromRow` is generated for tables and views. `QueryExt` adds typed versions of
`query`, `query_one`, and `query_opt` to both a `tokio_postgres::Client` and a
`deadpool_postgres` client:

```rust
use orm::QueryExt;

let client = pool.get().await?;
let account: Option<Account> = client
    .query_opt_typed(
        "SELECT id, email, status, profile, created_at
         FROM public.accounts WHERE id = $1",
        &[&account_id],
    )
    .await?;
```

The selected column names must match the Rust field names because the generated
`FromRow` implementation uses `row.try_get("field_name")`.

`QueryOptions` builds a parameterized `WHERE` clause and a sort/limit/offset
suffix for generated CRUD and views:

```rust
use orm::query::{FilterGroup, FilterOp, QueryOptions, QuerySort, SortOrder};

let options = QueryOptions::new()
    .filter_group(
        FilterGroup::or()
            .filter("email", FilterOp::ILike, "example")
            .filter("status", FilterOp::Eq, AccountStatus::Suspended),
    )
    .filter("email", FilterOp::Ne, "blocked@example.com")
    .sort(QuerySort::new("created_at", SortOrder::Desc))
    .limit(25)
    .offset(50);

let accounts = pool.get_accounts(options).await?;
```

Separate `.filter(...)` calls are joined with `AND`; an `or()` or `and()`
`FilterGroup` is parenthesized. `Like` and `ILike` automatically wrap string
values in `%` wildcards. `IsNull` and `IsNotNull` do not consume a parameter.
`Search` can be converted into an `OR` group for a query across a list of
columns.

Values are bound through PostgreSQL parameters. Field names, sort names, view
filters, view orderings, and table names are SQL text, however, and are not
escaped or parameterized. Never pass user-provided identifiers to
`QueryOptions`, `QuerySort`, or a raw `filter`/`order_by` attribute; map user
input to a closed list of trusted column names first. Built-in filter values are
strings, `i32`, `bool`, `uuid::Uuid`, `Option<T>`, and enums generated by
`#[enum_type]`; use a handwritten query for other PostgreSQL value types.

## PostgreSQL enums and JSONB values

`#[enum_type]` generates a Rust enum with Serde and PostgreSQL conversions, a
`SqlType` implementation, migration metadata, a TypeScript string union, and a
`FilterValue` implementation:

```rust
#[enum_type(
    schema = "public",
    name = "project_visibility",
    export_to = "types/database/projects.ts"
)]
pub enum ProjectVisibility {
    Public,
    #[postgres(name = "team_only")]
    TeamOnly,
}
```

Variants use their snake-case Rust name as the PostgreSQL/JSON value unless
`#[postgres(name = "...")]` overrides it. The generated migration creates the
PostgreSQL enum and adds new values when the Rust enum grows.

`#[json_type]` is for typed JSON/JSONB structs:

```rust
#[json_type(export_to = "types/database/projects.ts")]
pub struct ProjectSettings {
    pub archived: bool,
    pub labels: Vec<String>,
}
```

It derives Serde, converts to and from PostgreSQL `json`/`jsonb`, maps to
`jsonb` in the schema registry, and exports a TypeScript object type. Use
`Option<ProjectSettings>` for a nullable JSONB column.

## Read-only views

`#[view_type]` describes a read-only projection. Each field names its source
column. The first field supplies the base table; additional tables are joined
through foreign keys registered by the table macros:

```rust
#[view_type(
    schema = "public",
    name = "account_cards",
    export_to = "types/database/accounts.ts",
    filter = "accounts.status = 'active'",
    order_by = "accounts.created_at DESC"
)]
pub struct AccountCard {
    #[pg(view(public.accounts.id))]
    pub id: uuid::Uuid,
    #[pg(view(public.accounts.email))]
    pub email: String,
    #[pg(view(public.profiles.display_name))]
    pub display_name: String,
}
```

The macro generates `FromRow`, a TypeScript type, and an `AccountCardView`
trait implemented for `deadpool_postgres::Pool`. With the view name above its
query method is `get_account_cards(QueryOptions)`. The view is also registered
for migration generation. `filter` and `order_by` are raw SQL and should only
contain trusted, application-owned text.

## API types, validation, and endpoint metadata

`#[api_type]` creates a Serde request/response type, an `orm::Validate`
implementation, a TypeScript type, and a client-side validator schema:

```rust
use axum::{extract::Json, http::StatusCode};
use orm::{api_type, endpoint, Valid};

#[api_type(export_to = "types/api/accounts.ts")]
pub struct CreateAccount {
    #[api(validate(email))]
    pub email: String,
    #[api(validate(length(min(12), max(128))))]
    pub password: String,
    #[api(validate(range(min(1), max(100))))]
    pub seats: Option<u32>,
}

#[endpoint(POST, "/accounts", "accounts.create")]
async fn create_account(
    Valid(Json(request)): Valid<Json<CreateAccount>>,
) -> Result<Json<Account>, StatusCode> {
    let _ = request;
    todo!()
}
```

The runtime validation rules are `email`, `required`, `length(min(...),
max(...), or equal(...))`, `range(min(...), or max(...))`, and
`regex(r"...")`. Optional fields are checked only when they are present,
unless they also have `required`. `Valid<Json<T>>` and `Valid<Query<T>>`
validate the Axum extractor and return a `400` JSON response containing a
summary and field errors when validation fails.

`#[endpoint]` only registers metadata; it does not add a route to an Axum
router. Register the function with Axum as usual. Its metadata records the HTTP
method, path, request/query types, and a `Json<T>` response type so the
TypeScript client generator can produce a matching method. Endpoint names may
contain dots, which become nested client objects; path parameters such as
`/accounts/{id}` become string arguments.

## TypeScript generation

The built-in generator currently targets TypeScript. Every `#[*_type]` macro
submits metadata to an `inventory` registry. The `src/bin/orm-cli.rs` binary
from the setup section exposes that registry to the generator; its intentional
`use account_api as _` keeps the model library linked into the CLI.

Run it from the application package:

```sh
cargo run --bin orm-cli -- generate --lang ts
cargo run --bin orm-cli -- generate --lang ts --out ./frontend/src/lib
```

The command writes the paths from `export_to`, plus the generated validator
schemas at `schema/schemas.ts`, the endpoint client at `service/client.ts`, and
the `Result` runtime at `lib/result.ts`. The default client imports use the
`$lib` prefix; call `orm::lang::ts::generate_client` directly with another
prefix when your frontend uses a different layout.

An endpoint named `accounts.create` can then be called from TypeScript like
this:

```ts
import { createApi } from "$lib/service/client";

const api = createApi({ baseUrl: "/api" });
const result = await api.accounts.create({
  email: "ana@example.com",
  password: "a sufficiently long password",
  seats: 5,
});

if (!result.err) {
  console.log(result.value.id);
}
```

The generated client validates `#[api_type]` request bodies with Valibot before
sending them and returns a `Result` instead of throwing for transport, HTTP, or
validation failures. `orm::export::export_all_types` and the
`ExportBackend` trait are available when building a custom generator.

## Migrations

The macros register tables, enums, and views in a schema registry. The migration
commands diff that registry against a snapshot and write reviewable `.up.sql`,
`.down.sql`, and `meta/*.json` files:

```sh
# Create migrations/0001_create_accounts.{up,down}.sql and meta/0001.json.
cargo run --bin orm-cli -- migrate generate create_accounts

# DATABASE_URL is used unless --database-url is supplied.
DATABASE_URL=postgres://localhost/app \
  cargo run --bin orm-cli -- migrate apply

cargo run --bin orm-cli -- migrate status
cargo run --bin orm-cli -- migrate diff
cargo run --bin orm-cli -- migrate diff --write reconcile_live_database
```

Useful commands are:

- `migrate generate <name>` creates a migration from the latest snapshot.
- `migrate apply` runs pending migrations transactionally and records them in
  `_orm_migrations`.
- `migrate revert` runs the latest down migration when it is applied and removes
  its files.
- `migrate baseline <name>` introspects an existing database and records its
  current schema without executing migration SQL. Use it only for an empty
  migrations directory.
- `migrate diff` compares a live database with the Rust registry; `--write`
  turns the reconciliation into a migration.
- `migrate status` lists migrations and, when a database URL is available,
  marks them applied or pending.

Review generated SQL before applying it. Rename detection is interactive by
default because a drop/add pair may actually be a data-preserving rename;
`--no-input` treats ambiguous changes as drop/add. PostgreSQL enum values cannot
be removed by the generated down migration, so a revert may contain a comment
requiring a manual type recreation.

## Using the building blocks directly

The generated surfaces are optional. For custom repositories and tooling, the
crate also exposes:

```rust
use orm::schema::{assemble_desired_schema, diff, render, DatabaseSchema, NoRenames};

let baseline = DatabaseSchema::default();
let desired = assemble_desired_schema();
let changes = diff(&baseline, &desired, &mut NoRenames);
let migration_sql = render(&changes);
```

`orm::schema::introspect` reads PostgreSQL catalog data into the same schema
model, while `orm::export::export_all_types` and the language backends can be
used independently of the CLI.
