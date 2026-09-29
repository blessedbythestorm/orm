# orm

`orm` is a PostgreSQL ORM and schema/code-generation toolkit for Rust. It
generates row decoding, CRUD code, PostgreSQL conversions, API validation,
TypeScript types, HTTP client metadata, and migrations from Rust definitions.

It uses `tokio-postgres` and `deadpool-postgres`. SQL remains visible: generated
queries and migration files are ordinary SQL and can be reviewed or replaced
with handwritten queries.

## Install and create an Axum project

`orm` is currently consumed from Git. Use `orm` only; the `macros` crate is an
implementation detail re-exported by it. Pin a revision in applications so a
schema or query change reaches them only when the pin moves:

```toml
[dependencies]
orm = { git = "https://github.com/blessedbythestorm/orm.git", rev = "<commit>" }

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
    #[pg(primary, default_value(gen_random_uuid()))]
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
  `delete_account` on pools, pooled clients, PostgreSQL clients, and transactions;
- a typed upsert for every declared unique key, such as
  `upsert_account_by_email(&AccountInsert)` for `#[pg(unique)]` and
  `upsert_membership_by_account_id_and_user_id(...)` for a composite
  `#[pg(unique(columns(account_id, user_id)))]` constraint.

Generated upserts use PostgreSQL `ON CONFLICT`, update all mutable columns from
the proposed row, and return the inserted or updated record. The same CRUD
methods are available on a pooled `deadpool_postgres::Object` and a native
`tokio_postgres::Transaction`, allowing callers to compose them atomically.
Each key also has a selective `_with` variant that accepts an update value and
changes only its populated fields, for example
`upsert_account_by_email_with(&insert, &update)`.

Generated CRUD respects the column marked `#[pg(primary)]`, but its
single-record method signatures currently require a UUID key. For another key
type, use the fluent query API with `.where_("key_column", FilterOp::Eq, key)`.
The database must provide defaults for generated columns such as the primary
key and `created_at`.

Useful field attributes are:

```rust
#[pg(unique)]
#[pg(default_value("active"))]                   // literal default
#[pg(default_value(now()))]                       // database generator
#[pg(index)]                                      // single-column index
#[pg(foreign(public.users.id, on_delete(cascade)))]
#[crud(insert(optional))]
#[crud(insert(skip), update(skip))]
```

Put compound indexes, compound unique constraints, and cross-column database
rules on the struct. Predicates use a bounded expression syntax, so malformed
columns and unsupported SQL fail at compile time:

```rust
#[pg(unique(columns(account_id, user_id)))]
#[pg(index(columns(account_id, created_at)))]
#[pg(index(columns(account_id, user_id), unique, where(is_null(revoked_at))))]
#[pg(validate(present_iff(revoked_at, status == MembershipStatus::Revoked)))]
pub struct Membership {
    // ...
}
```

Index names are optional. The default uses the table and key columns. When
multiple inferred indexes would receive the same name, the colliding names add
a fixed digest of their canonical predicate and uniqueness. This keeps partial
indexes on the same columns distinct, preserves existing non-colliding names,
and respects PostgreSQL's 63-byte identifier limit. Use `name = "..."` only
when an external contract requires a specific database identifier.

`default_value` accepts scalar literals, `now()`, `gen_random_uuid()`, and a
registered enum variant. Strings are always quoted values; a string such as
`"now()"` never becomes executable SQL. `pg(validate(...))` creates a database
CHECK constraint, while `api(validate(...))` validates application inputs.

#### Database rules with `pg(validate(...))`

A rule is a Rust-syntax expression over the model's own columns, checked at
compile time and rendered to SQL. On a field it constrains that field; on the
struct it can relate several:

```rust
#[pg(validate(implies(one_of(operation, ["dispatch", "void"]), balance_exact == 0)))]
#[pg(validate(required_if(actor_id, operation != "opening")))]
#[pg(validate(num_nonnulls(coil_id, ink_id) == 1))]
pub struct Movement {
    #[pg(validate(delta_exact != 0))]
    pub delta_exact: NumericText,
    // ...
}
```

| Form | Meaning |
| --- | --- |
| `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `\|\|`, `!` | Comparison and boolean logic |
| `+`, `-`, `*`, `/`, unary `-` | Arithmetic |
| Literals, `"text"`, `Enum::Variant` | Numbers, booleans, strings and registered enum values |
| `is_null(a)`, `is_not_null(a)` | Explicit NULL tests |
| `one_of(a, [x, y])`, `between(a, low, high)`, `matches(a, "regex")` | Membership, ranges, POSIX pattern |
| `exactly_one(a, b, ...)`, `num_nonnulls(a, b, ...)` | Exactly one present / how many are present |
| `implies(condition, rule)` | `rule` must hold whenever `condition` does |
| `present_iff(a, condition)` | `a` is set exactly when `condition` holds |
| `required_if(a, condition)` | `a` must be set when `condition` holds |
| `length`, `char_length`, `trim`, `btrim`, `abs` | Scalar functions |

Unknown columns and functions fail compilation. So does a rule that could
evaluate to SQL UNKNOWN because it reads a nullable column without deciding
what NULL means; PostgreSQL would silently accept such a row. Test nullable
columns with `is_null`/`is_not_null` or use a presence rule.

Rule names are inferred and stable, so a declaration and its migration stay in
step:

- a field rule is `<table>_<field>_check`;
- `present_iff`/`required_if` on a column is `<table>_<column>_presence_check`;
- any other struct rule is `<table>_<first two referenced columns>` followed by
  a fixed 16-hex-digit digest of the rendered predicate and `_check`, shortened
  to PostgreSQL's 63-byte identifier limit.

Changing a struct rule's predicate therefore changes its name, and the
generated migration drops the old constraint and adds the new one. Add
`name = "..."` inside `validate(...)` only when something outside the schema
refers to the constraint by name.

`Option<T>` maps to a nullable column. Built-in `SqlType` mappings include
`bool`, integer/float types, `String`, `Vec<u8>`, `uuid::Uuid`, common `chrono`
types, `serde_json::Value`, and `NumericText`. `NumericText` carries finite
PostgreSQL `numeric` values as decimal strings, including inside `IN` and
`NOT IN` filters, without converting through floating point.

Generated update DTOs distinguish all three nullable-field states: omission
leaves the column unchanged, JSON `null` clears it, and a value replaces it.
In Rust, a nullable update field is `Option<Option<T>>`; use `None`,
`Some(None)`, and `Some(Some(value))` respectively. Exported TypeScript keeps
the equivalent `field?: T | null` contract.

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
TypeScript type, and `AccountCardView::get_account_cards(QueryOptions)` on the
same clients as table CRUD. `filter` and `order_by` are raw schema SQL. Runtime
joins can instead be built on SELECT without declaring a database view.

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

### Fluent queries

Import `orm::query::QueryBuilderExt` to build queries on a pool, pooled client,
PostgreSQL client, or transaction. `#[table_type]` generates `QueryModel` and
`TableModel` metadata; `#[view_type]` generates read-only `QueryModel` metadata.
The shared runtime builders use `QueryOptions`, `FilterGroup`, `InsertValues`,
and `UpdateValues`. All generated CRUD, view reads, counts, and upserts delegate
to these builders. The macros only provide model metadata and map typed DTOs
to values; they no longer maintain a separate SQL execution implementation.

```rust
use orm::query::{FilterOp, QueryBuilderExt, SortOrder};

let accounts = transaction
    .update::<Account>()
    .set("email", new_email)
    .where_("id", FilterOp::Eq, account_id)
    .returning()
    .await?;

let matching = transaction
    .select::<Account>()
    .where_("email", FilterOp::ILike, "example.com")
    .order_by("email", SortOrder::Asc)
    .limit(25)
    .fetch_all()
    .await?;

let deleted = transaction
    .delete::<Account>()
    .where_("id", FilterOp::Eq, account_id)
    .execute()
    .await?;
```

Builders do no database work until a terminal method is awaited. Rust reserves
`where`, so the predicate method is `where_`.

| Operation | Terminal | Result |
| --- | --- | --- |
| SELECT | `fetch_all()` | `Vec<Model>` |
| SELECT | `fetch_optional()` | `Option<Model>`; error on more than one selected row |
| SELECT | `fetch_one()` | `Model`; error unless exactly one row is selected |
| SELECT | `count()` | `i64` matching rows before pagination, without sorting or locks |
| UPDATE / DELETE | `execute()` | `u64` affected rows, without a RETURNING clause |
| UPDATE / DELETE | `returning()` | `Vec<Model>` from the same modifying statement |
| INSERT | `execute()` / `returning()` | Affected count / returned rows, including zero for DO NOTHING |
| INSERT / UPDATE | `returning_one()` | Exactly one returned row, otherwise an error |

`fetch_one` and `fetch_optional` do not silently add a limit. To request the
first match, explicitly combine an ordering with `limit(1)`. A write returning
no rows is successful; the application decides whether that means not found,
a stale version, or an ordinary no-op.

`returning_one()` does not undo a write if it affects multiple rows. Use a
unique predicate for single-row updates, as generated primary-key CRUD does,
and keep operations in an explicit transaction when rollback is required.

SELECT, UPDATE, and DELETE support `where_`, `where_group(FilterGroup)`,
`where_null`, and `where_not_null`. Separate predicates/groups are ANDed; `FilterGroup::or()`
provides a parenthesized OR. `where_` preserves optional-filter behavior:
`None` omits a predicate, so use `where_null` to match SQL NULL. Required scope,
identity, and version values should be concrete values, not optional filters.

SELECT also supports `order_by`, `then_order_by`, `limit`, `offset`, `for_update`,
and `for_share`. Views support SELECT only. SELECT locks the base relation
(`FOR UPDATE/SHARE OF <base alias>`), including when LEFT JOINs are present.
Row locks need an explicit transaction to outlive the statement; a pool query
releases them immediately.

UPDATE supports `set`, `add`, `subtract`, `set_null`, and `database_now`.
Arithmetic and timestamp expressions execute inside the UPDATE, preserving
atomic conditional updates. Exact PostgreSQL numeric values use `NumericText`.
`set("note", None::<String>)` explicitly writes NULL; omitting a `set` leaves
the column unchanged. An update must have at least one assignment.

For existing query composition, `.options(QueryOptions)` replaces accumulated
options, and `.values(UpdateValues)` on UPDATE replaces accumulated assignments.
Subsequent builder calls extend those supplied values. Writes reject sorting,
pagination, and row locks passed through `options` before any SQL is executed.

UPDATE and DELETE require a nonempty effective predicate. An omitted optional
filter or empty group does not satisfy this requirement. To deliberately affect
the whole table, use `.all_rows()`; combining that with a predicate is rejected.

```rust
let deleted = transaction
    .delete::<Account>()
    .all_rows()
    .execute()
    .await?;
```

Fluent queries validate filter, sort, and write column names against model
metadata before acquiring a connection or executing SQL. Values are bound
parameters. Column/value compatibility is still checked at runtime; string
column names do not provide compile-time field typing or application-level
authorization. PostgreSQL errors remain in the `anyhow` source chain.

### Joins and projections

SELECT supports `.inner_join::<Model>(alias, on)` and
`.left_join::<Model>(alias, on)` with explicit `JoinOn` conditions. The joined
model can be a table or a view. Aliases support self-joins, multiple references
to the same table, and joins through previously joined relations; no foreign
key inference is required.

For example, assuming `Order` and `Customer` are declared table models:

```rust
use orm::query::{FilterOp, JoinOn, QueryBuilderExt};

#[derive(orm::FromRow)]
struct OrderCustomer {
    order_id: uuid::Uuid,
    customer_name: Option<String>,
}

let rows = transaction
    .select::<Order>()
    .alias("orders")
    .left_join::<Customer>(
        "customer",
        JoinOn::eq("orders.customer_id", "customer.id")
    )
    .where_("orders.id", FilterOp::Eq, order_id)
    .project::<OrderCustomer>(&[
        ("orders.id", "order_id"),
        ("customer.name", "customer_name"),
    ])
    .fetch_all()
    .await?;
```

`#[derive(orm::FromRow)]` decodes named struct fields from matching result
aliases. A projection is a query result, not a registered table or view, and
does not generate a migration. Use `Option<T>` for columns that can be NULL
after an unmatched LEFT JOIN. Without `project`, SELECT returns just the base
model's columns, with output aliases that prevent overlapping `id` fields from
being confused.

The base alias defaults to the final component of its declared relation name.
Unqualified fields always refer to the base model. Use `alias.column` for
joined fields in ON, WHERE, projections, and ordering. Aliases and columns are
validated before execution; aliases must be unique plain identifiers of at
most 63 bytes. An ON condition can refer only to the base, earlier joins, and
the relation being joined. Unknown/future aliases and duplicate output aliases
are errors.

`JoinOn::eq(left, right)` compares two columns. `.and_on(left, op, right)` adds
another comparison (`Eq`, `Ne`, `Gt`, `Gte`, `Lt`, or `Lte`). ON also supports
bound `.where_(field, op, value)`, `.where_group(FilterGroup)`, `.where_null`,
and `.where_not_null`. All components are ANDed; filter groups provide grouped
ORs. Empty ON conditions are rejected. Values stay bound across every join and
the outer WHERE clause.

Put a joined-row eligibility filter in ON when unmatched LEFT JOIN base rows
should remain. Putting it in WHERE can exclude those rows. One-to-many joins
return duplicate base rows as SQL does; limits apply to joined rows and
`count()` counts their multiplicity. RIGHT/FULL/CROSS joins, arbitrary SQL
expressions, aggregate projections, and joined UPDATE/DELETE are not exposed.

### Inserts, upserts, and generated CRUD

`insert::<Model>().value(field, value)` or `.values(InsertValues)` creates one
row; no values means `DEFAULT VALUES`. Choose `.execute()`, `.returning()`, or
`.returning_one()` explicitly. For conflicts:

```rust
let account = transaction
    .insert::<Account>()
    .value("email", email)
    .on_conflict(&["email"])
    .do_update_excluded(&["email"])
    .returning_one()
    .await?;
```

An explicit conflict target is followed by `do_nothing()`,
`do_update_excluded(&[columns])`, or `do_update(UpdateValues)`. Bound update
values follow the insert parameters; arithmetic updates refer to the existing
target row. Conflict targets must correspond to database uniqueness rules.
The runtime validates column membership and rejects empty or duplicate column
lists and empty explicit update sets.

Generated `create_*`, `update_*` and upsert methods first run the insert or
update DTO's `api(validate(...))` rules and return the validation errors before
any SQL executes. The dynamic `insert_<table>_fields`, `update_<tables>_where`
and fluent builders take values rather than DTOs and do not run those rules;
the table's `pg(validate(...))` CHECK constraints apply to every write.

Generated CRUD preserves its DTO rules: optional insert fields can defer to
database defaults; skipped fields are excluded; nullable patches distinguish
omission from explicit NULL; primary-key methods use the declared key name.
An empty generated update still errors. A missing single-record delete still
errors. Standard upserts update the same eligible columns from EXCLUDED;
selective upserts retain unchanged fields and return the existing row even
when the patch is empty. Counts retain filter-only semantics. These methods
are convenience adapters over the query API, so existing call sites can stay.

### Existing query interfaces

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
            .filter("email", FilterOp::EqInsensitive, "support@company.org"),
    )
    .sort(QuerySort::new("email", SortOrder::Asc))
    .limit(25)
    .offset(50);

let accounts = pool.get_accounts(options).await?;
```

Generated table CRUD also provides `count_<table>s(QueryOptions)`. The same
CRUD trait is implemented for `tokio_postgres::Transaction`, so a service can
compose generated reads and writes atomically without dropping down to raw SQL:

```rust
let mut client = pool.get().await?;
let transaction = client.transaction().await?;
let account = transaction.create_account(&payload).await?;
transaction.commit().await?;
```

Use `.is_null("deleted_at")` and `.is_not_null("confirmed_at")` for NULL
predicates without a dummy filter value. `.then_sort(...)` appends deterministic
secondary ordering. `.for_update()` and `.for_share()` add row locks; use them
on a transaction when the lock must live beyond the SELECT statement.

For server-managed or conditionally populated fields, generated CRUD exposes
`insert_<table>_fields(InsertValues)` and
`update_<tables>_where(QueryOptions, UpdateValues)`. Values remain bound and
column names are checked against the model at runtime. `UpdateValues` supports
assignment, addition, subtraction, NULL, and database time. These builders are
dynamic rather than compile-time column/value typed.

Filtered update and bulk delete accept predicates only. Passing sorting,
pagination, or row-lock controls returns an error before SQL executes; those
controls are never silently discarded. Both operations also reject an empty
predicate. If a bounded write is needed, select and lock explicit identities in
a transaction, then update those identities.

`FilterOp::In` and `FilterOp::NotIn` accept vectors and bind them as a single
PostgreSQL array (`= ANY($n)` / `<> ALL($n)`). Values are parameterized. Field
names, sort names, and raw view/table SQL are
not; whitelist any identifier derived from user input. Built-in filter values
are strings, `i32`, `bool`, `Uuid`, `Option<T>`, and `#[enum_type]` enums.
`FilterOp::EqInsensitive` performs case-insensitive equality without adding the
wildcards used by `ILike`.

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

Tables, enums, and views register schema metadata. Migration generation
introspects the live database selected by `DATABASE_URL`, diffs it against the
Rust metadata, and writes `.up.sql`, `.down.sql`, and `meta/*.json` files:

```sh
cargo run --bin orm-cli -- migrate generate create_accounts
DATABASE_URL=postgres://localhost/account_api \
  cargo run --bin orm-cli -- migrate apply
cargo run --bin orm-cli -- migrate status
cargo run --bin orm-cli -- migrate diff
cargo run --bin orm-cli -- migrate diff --write reconcile_database
```

To generate against another environment without replacing the local
`DATABASE_URL`, name the variable containing that connection URL:

```sh
PROD_DATABASE_URL=postgres://localhost:15432/app \
  cargo run --bin orm-cli -- migrate generate add_orders \
    --database-url-env PROD_DATABASE_URL
```

This is useful with an SSH or platform database proxy. The URL remains in the
environment rather than appearing in the command line.

`migrate baseline <name>` adopts an existing database without running SQL.
`migrate revert` runs the latest down migration and removes its files. Review
generated SQL before applying it; ambiguous renames are interactive by default,
and enum values cannot be removed by the generated down migration.

Live drift checks round-trip declared view, check-constraint, and partial-index
expressions through temporary PostgreSQL objects in rolled-back transactions.
The stored definitions are compared with the live catalog definitions. This
accepts PostgreSQL's formatting of equivalent declarations while preserving
view output names and check-constraint null behavior. If a declaration cannot
be normalized, verification fails instead of reporting that the schema matches.
This check needs permission to create temporary objects and a writable
transaction; it does not persist schema changes.

## Lower-level schema APIs

For custom tooling, use the schema model directly:

```rust
use orm::schema::{assemble_desired_schema, diff, render, DatabaseSchema, NoRenames};

let baseline = DatabaseSchema::default();
let desired = assemble_desired_schema();
let sql = render(&diff(&baseline, &desired, &mut NoRenames));
```
