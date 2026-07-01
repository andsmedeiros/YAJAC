# YAJAC — Architecture

**Y**et **A**nother **J**SON:**A**PI **C**rate. A server-agnostic Rust implementation of the
[JSON:API v1.1](https://jsonapi.org/format/) specification over a pluggable, schema-driven data
layer. Edition 2024.

The crate produces JSON:API responses from HTTP requests; it does **not** own a socket. The routing
layer takes an `http::Request<Vec<u8>>` and returns an `http::Response<Option<Document>>`, leaving
transport (a web framework, a Tauri command, a test harness) to the embedder.

## Module map

The crate root (`src/lib.rs`) exposes five top-level modules, layered from the wire inward:

| Module          | Role                                                                                     |
| --------------- | ---------------------------------------------------------------------------------------- |
| `routing`       | HTTP surface: route matching, controllers, per-request context, the response boundary.   |
| `json_api`      | JSON:API v1.1 document model — the serialised wire types.                                 |
| `database`      | Schema-driven data layer: schema, registry, store, records, query building, adapters.    |
| `http_wrappers` | Serde-friendly newtypes over the `http` crate (`StatusCode`, `Uri`).                     |
| `core`          | Cross-cutting glue: the document factory (`to_document`) and a shared error type.         |

### `routing`

The embedder-facing layer.

- **`router`** — `Router` / `RouterBuilder`. Routes are `(Method, path segments, handler)`; a segment
  prefixed `:` captures a `RouteParameters` entry. `RouterBuilder::resource::<T>(scope)` wires the full
  CRUD set (`index`/`show`/`create`/`update` on both PUT and PATCH/`delete`);
  `read_only_resource::<T>` wires only `index`/`show`. `Router::handle` is the single request→response
  boundary (see *Request lifecycle*).
- **`controller`** — `ResourceController` / `ReadOnlyResourceController` traits an embedder implements
  per resource.
- **`context`** — `Context<'sch, 'req, Adapter>`, the per-request bundle handed to a handler
  (registry, uri, route params, parsed request). Where request identifiers are materialised against
  the schema.
- **`request` / `responder` / `result` / `route_parameters` / `uri_generator` / `error`** — the
  request wrapper, response builders, the routing `Result`/`Error`, captured path params, link
  generation, and the routing error type.

### `json_api`

Pure (de)serialisation types mirroring the spec: `document`, `resource`, `identifier`,
`relationship`, `links`, `primary_content` (the `data` vs `errors` split), and `error`. No behaviour
beyond serde.

### `database`

The engine. Schema-driven and adapter-generic.

- **`schema`** — `TableSchema<'sch>` and its parts (`PrimaryKey`, `AttributeType`, `Relationship`,
  `RelatedResource`, `RelationshipKeys`). Currently flat borrowed tuple-slices with linear lookups
  (see *Known rework*).
- **`registry`** — `Registry<'sch, Adapter>`: validates the schema set at construction, owns the
  connection pool, hands out request-scoped `Table`s. Must be `Send + Sync` (asserted in
  `adapters::tests`) so the borrowing request path can run on any worker thread.
- **`store`** — record/collection create/update/delete, including relationship persistence.
- **`record` / `attributes` / `relationships` / `composite`** — materialised rows and their
  field/relationship data.
- **`query_parameters`** — parses JSON:API query params (`include`, `fields`, `filter`, `sort`)
  against a schema.
- **`query_builder` / `connection` / `pool` / `table`** — adapter-facing interfaces (traits).
- **`data_loader`** — relationship/include resolution.
- **`migrator`** — migration machinery (feature-gated; see *Features*).
- **`adapters`** — the extension seam (below).

### The adapter seam

`database::adapters::Adapter` is the trait a backend implements:

```rust
pub trait Adapter {
    type Connection: ConnectionInterface;
    type Pool: PoolInterface<Connection = Self::Connection>;
    type QueryBuilder<'sch>: QueryBuilderInterface<'sch>;
    // type Migrator: MigratorInterface;   // not yet wired
    type Table<'sch, 'req>: TableInterface<'sch, 'req, Self::Connection, Self::QueryBuilder<'sch>>
    where Self::Connection: 'req;
}
```

`SqliteAdapter` (rusqlite + r2d2) is the only implementation today, behind the default-on `sqlite`
feature. `type Migrator` is intentionally commented out — migrations are not yet part of the seam.

## Request lifecycle

`Router::handle(&self, database: &'sch Registry<'sch, Adapter>, request: http::Request<Vec<u8>>)`:

1. Extract `uri`, `method`, and non-empty path segments.
2. Find the first `Route` matching method + arity, capturing `:param` segments into `RouteParameters`.
3. Deserialise the body (`serde_json::from_slice`) into a `Request`; build a `Context`.
4. Invoke the matched handler → `routing::Result`.
5. **Response boundary (`.or_else`)** — on error: read the status; if it is 5xx, log it with full
   `Debug` detail (`error!`), and in **non-debug** builds redact it to a generic
   `InternalServerError` (dev builds keep the detail). Serialise via `to_document` and respond.
6. No route match → 404 `ResourceNotFound`. Error-document construction itself failing → logged, bare
   500.

Redaction and logging live **only** here, at the single boundary, so every 5xx source is caught with
full detail before anything is hidden from the consumer.

## Lifetimes: `'sch` and `'req`

Two lifetimes thread the whole codebase and carry a **provenance** meaning, not merely a scope:

- **`'sch`** — schema-scoped: the registry, schemas, and every **trusted**, framework-blessed string
  (it may in practice be `'static`; the exact region is irrelevant). Trusted strings are the only ones
  permitted into SQL **text**.
- **`'req`** — request-scoped: everything derived from the incoming request, i.e. **untrusted** data,
  which may only ever reach query **bindings**, never SQL text.

A handler is `for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch`.

## Error flow

Three error types, each translating outward toward the wire:

`database::Error` → `routing::Error` → `json_api::error::Error`

- `database::Error` is **source-classified**: each variant carries a single HTTP meaning exposed via
  `status()` / `code()` / `title()`, and a consumer-facing `Display` message.
- `From<database::Error> for routing::Error` is **lossless** (status/code/title/detail preserved).
- Redaction of 5xx detail happens at the router boundary, never in the `From`.

See [CONVENTIONS.md](CONVENTIONS.md) for the error-message rules.

## Features

Declared in `Cargo.toml`:

- **`sqlite`** *(default)* — pulls `rusqlite`, `base64`, `r2d2`, `r2d2_sqlite`; enables `SqliteAdapter`.
- **`builtin_migrations`** — pulls `include_dir` to embed migration files. Off in dev/test.

## Known rework

The near-term plan reshapes parts of this document; treat the following as in-flux:

- The **schema model** (`schema.rs`) will move from flat tuple-slices with O(n) lookups to a richer,
  indexed model built by an ergonomic builder.
- The **key-bearing signatures** (`Attributes`, `ForeignKeys`, `Relationships`, `QueryParameters`)
  will move from owned `String`s to schema-borrowed identifiers under a "parse, don't validate"
  validation layer.
- **Relationship endpoints** (`/:type/:id/relationships/:rel`) are not yet implemented; only resource
  endpoints exist.
