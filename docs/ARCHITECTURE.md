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
  prefixed `:` captures a `RouteParameters` entry. `RouterBuilder::resource::<T>(scope, schema)` wires
  the full CRUD set (`index`/`show`/`create`/`update` on both PUT and PATCH/`delete`);
  `read_only_resource::<T>(scope, schema)` wires only `index`/`show`. The `schema` — obtained from the
  connection manager's registry at wiring — is captured into each handler, which builds a per-request
  `ResourceContext` from it. `Router::handle` is the single request→response boundary (see *Request
  lifecycle*).
- **`controller`** — `ResourceController` / `ReadOnlyResourceController`, traits an embedder implements
  per resource as a **stateless marker type**. Their default handler methods receive a
  `ResourceContext<'sch, 'req, Adapter>` — the resource's schema paired with the request `Context` — and
  reach record parsing, query parameters, the store and id resolution through it, already bound to the
  schema. Overriding a method customises one action.
- **`context`** — `Context<'sch, 'req, Adapter>`, the per-request bundle (connection manager, uri,
  route params, parsed request), wrapped by a `ResourceContext` before it reaches a handler. Where
  request identifiers are materialised against the schema.
- **`request` / `responder` / `result` / `route_parameters` / `uri_generator` / `error`** — the
  request wrapper, response builders, the routing `Result`/`Error`, captured path params, link
  generation, and the routing error type.

### `json_api`

Pure (de)serialisation types mirroring the spec: `document`, `resource`, `identifier`,
`relationship`, `links`, `primary_content` (the `data` vs `errors` split), and `error`. No behaviour
beyond serde.

### `database`

The engine. Schema-driven and adapter-generic.

- **`schema`** — `Schema<'sch>` and its parts (`PrimaryKey`, `AttributeType`, `ColumnDescriptor`,
  `RelationshipDescriptor`, `RelationshipKind`, `RelatedResource`, `RelationshipKeys`). Owned `IndexMap`
  containers keyed by borrowed `&'sch str`: O(1) lookup with **definition order preserved** (that order
  is observable in generated SQL). The `attribute`/`foreign_key`/`relationship` lookups return the
  matching *descriptor* (`ColumnDescriptor`/`RelationshipDescriptor`), which carries the schema's own
  `&'sch` name alongside its type — so a lookup hands back a self-named, trusted identifier. Built by
  an ergonomic **`SchemaBuilder`** (`schema::builder`) — the public, intended way to define a schema —
  which collects inert `SchemaParts` that the registry validates and mints into `Schema`s
  (`Schema::new` is `pub(crate)`).
- **`registry`** — `Registry<'sch>`: takes `SchemaBuilder`s and **owns** the resulting schemas,
  validating-and-minting them in one fallible `try_build` step (per-schema consistency + cross-schema
  relationship checks; a duplicate or inconsistent set is rejected at construction). A pure schema
  collection — it holds no storage.
- **`connection_manager`** — `ConnectionManager<'sch, Adapter>`: binds a validated `Registry` (moved
  in, pre-built) to a connection pool. The request path's single handle: it lends schemas (through
  `registry()`) and hands out request-scoped connections and `Table`s. Must be `Send + Sync` (asserted
  in `adapters::tests`) so the borrowing request path can run on any worker thread.
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

`Router::handle(&self, database: &'sch ConnectionManager<'sch, Adapter>, request: http::Request<Vec<u8>>)`:

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

The column-name **keys** of `Attributes`/`Row` are `&'sch str`, interned at the two boundaries where
raw identifiers enter — request-in (`from_value`, `Context::require_record`) and `Table`-out
(`materialise_attributes`) — through the schema's self-named lookups. A request-supplied string can
therefore never *be* a column name in generated SQL; it can only ride into a binding as a value.

A handler is `for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch`.

## Error flow

Three error types, each translating outward toward the wire:

`database::Error` → `routing::Error` → `json_api::error::Error`

- `database::Error` is **source-classified**: each variant carries a single HTTP meaning exposed via
  `status()` / `code()` / `title()`, and a consumer-facing `Display` message.
- `From<database::Error> for routing::Error` is **lossless** (status/code/title/detail preserved).
- Redaction of 5xx detail happens at the router boundary, never in the `From`.

See [CONVENTIONS.md](CONVENTIONS.md) for the error-message rules.

## Testing & conformance

Two test layers (authoring rules in [CONVENTIONS.md](CONVENTIONS.md)):

- **In-crate unit tests** — per module, white-box, asserting the exact behaviour of each entity.
- **Black-box conformance suite** (`tests/conformance/`, the `conformance` test target) — drives
  `Router::handle` exactly as an embedder would and asserts *only* at the request→response boundary, so
  it is untouched by internal refactors. Authored **spec-first**: each test encodes a JSON:API v1.1
  rule (quoted verbatim above it), not current behaviour.

**What it pins — the public contract.** The suite tests an *arbitrary public contract*, not this
crate's internals: **the resource set and their schema, their locations (URLs), and each resource's
action mode (read-only | read-write)**. Nothing else is contractual — everything else is judged exactly
as the spec mandates. So any implementation exposing the *same* contract must pass, which makes the
suite a portable conformance harness rather than a mirror of this implementation.

**Tiers.**

- `mandatory` — the spec's MUSTs plus the contract's own obligations; every conforming implementation
  must pass.
- `recommended` — the spec's SHOULDs; a conforming implementation may decline these.

**Guarded optional affordances.** A few invariants are MUST/SHOULD *given support* for a feature whose
support is itself a MAY — `include`, `sort`, client-generated ids, full to-many replacement, and
relationship-member deletion. They live at their true obligation tier but **guard on the feature's
spec-defined non-support signal** (`400` for include/sort, `403` for the write opt-outs): absent
enforcement, that exact response lets the test log and return (skip) instead of failing; any other
response falls through and is asserted. Enforcement is per-affordance — set `YAJAC_ENFORCE_OPTIONAL` to
a comma-separated list of affordance keys (`include`, `sort`, `client-ids`, `full-replacement`,
`relationship-delete`) or `all` to turn a skip into a failure.

**Layout.** `test_support` is the fixture (a `ConnectionManager` + `RouterBuilder` over an abstract
five-resource schema, one read-only); `validations` holds the generic, reusable validators (JSON:API
grammar, full linkage, application URL set); the `mandatory` and `recommended` modules hold the tests.

**Current state — red by design.** The suite runs partially red on `main`: the mandatory failures are
this implementation's known conformance gaps (e.g. relationship/related-URL routing is not yet wired,
so those URLs return `404`, and self-links are emitted relative rather than absolute). Those reds are an
accepted, tracked liability until the implementation is conformed — the work belongs to Phase A (see
*Known rework*).

## Features

Declared in `Cargo.toml`:

- **`sqlite`** *(default)* — pulls `rusqlite`, `base64`, `r2d2`, `r2d2_sqlite`; enables `SqliteAdapter`.
- **`builtin_migrations`** — pulls `include_dir` to embed migration files. Off in dev/test.

## Known rework

The near-term plan reshapes parts of this document; treat the following as in-flux:

- The key-bearing signatures (`Attributes`, `ForeignKeys`, `Relationships`, `QueryParameters`) now key
  on schema-borrowed `&'sch str` under a "parse, don't validate" layer. The remaining move is on the
  **value** side: `Attribute` will carry `Cow<'req, str>` so request-borrowed and DB-owned values share
  one type (`Attributes<'sch, 'req>`), with no separate `Row` type.
- The **controller model** will grow: `ResourceContext` becomes a user-extensible per-request
  controller (`new(schema, context)` + user fields), and `Context` is renamed `RoutingContext`.
- **Relationship endpoints** (`/:type/:id/relationships/:rel`) are not yet implemented; only resource
  endpoints exist. Wiring them (and the related-resource URLs) is what turns much of the conformance
  suite's mandatory tier green.
