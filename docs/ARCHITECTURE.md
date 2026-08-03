# YAJAC — Architecture

**Y**et **A**nother **J**SON:**A**PI **C**rate. A server-agnostic Rust implementation of the
[JSON:API v1.1](https://jsonapi.org/format/) specification over a pluggable, schema-driven data
layer. Edition 2024.

The crate produces JSON:API responses from HTTP requests; it does **not** own a socket. The routing
layer takes an `http::Request<Vec<u8>>` and returns an `http::Response<Option<Document>>`, leaving
transport (a web framework, a Tauri command, a test harness) to the embedder.

## Module map

The crate root (`src/lib.rs`) exposes six top-level modules, layered from the wire inward:

| Module          | Role                                                                                                        |
| --------------- | ---------------------------------------------------------------------------------------------------------- |
| `routing`       | HTTP surface: route matching, controllers, per-request context, the response boundary.                     |
| `json_api`      | JSON:API v1.1 document model — the serialised wire types.                                                   |
| `serialisation` | Builds JSON:API documents from records: the document factory (`to_document`), link generation (`UriGenerator` over a `BaseUri`), and its error type. |
| `database`      | Schema-driven data layer: schema, registry, store, records, query building, adapters.                      |
| `http_wrappers` | Serde-friendly newtypes over the `http` crate (`StatusCode`, `Uri`).                                       |
| `utils`         | Generic, domain-agnostic helpers — e.g. the `indexing` iterator adaptors.                                  |

### `routing`

The embedder-facing layer.

- **`router`** — `Router`, assembled by `Router::try_new(base_uri, |root| …)` which runs a `PrimaryRouteBuilder`.
  `root.resource::<T>(scope, schema)` wires a resource's full CRUD (`index`/`show`/`create`/`update` on
  both PUT and PATCH/`delete`) **and** its relationship endpoints — the `/:id/relationships/:rel` linkage
  route and the `/:id/:rel` related-resource route; `read_only_resource::<T>` mounts the reads and refuses
  writes with `403`. The `schema` — obtained from the connection manager's registry at wiring — is
  captured into each handler, which builds a per-request `ResourceContext` from it. Routes are **eagerly
  built and validated** (a resource mounted twice, or a relationship name the schema doesn't declare, is a
  build-time `RouterError`). The router is **schema-aware**: alongside its route list it holds a
  `MountTable` — each resource's controller factory plus the link *templates* captured at build time (the
  collection base, from which the resource path is `base` + `:id`, and each mounted relationship slot) —
  and the `BaseUri` that roots every link it mints. `Router::handle` is the single request→response
  boundary (see *Request lifecycle*).
- **`builders`** — the route-builder DSL behind `Router::try_new`: `PrimaryRouteBuilder` (mounts resources
  and opens nested scopes), `ResourceRouteBuilder` (a resource's relationship endpoints and custom
  member/collection routes), and `SubordinateRouteBuilder`, with `RelationshipConfig` /
  `RelationshipsConfig` for per-relationship options (read-only, path/keyword relocation via `*_with`
  closures).
- **`controller`** (a directory module — `controller/{mod.rs, tests.rs}`) — `ResourceController`, the trait
  an embedder implements per resource as a **stateless marker type**. `DefaultController` is the
  no-customisation impl, and read-only mounting is a route-builder choice, not a separate trait. Its
  default handler methods receive a `ResourceContext<'sch, 'req, Adapter>` — the resource's schema paired
  with the request `Context` — and reach record parsing, query parameters, the store and id resolution
  through it, already bound to the schema. It also carries `parameters_for_route`, which resolves a mounted
  route's dynamic segments (`:id` from the record, others echoed from the request) for link rendering —
  **infallible**, omitting anything it cannot resolve. Overriding a method customises one action;
  overriding `configuration()` returns a `Configuration` that shapes framework behaviour — today whether
  the resource accepts **client-generated ids** (otherwise `create` refuses a client-supplied id with `403`).
- **`context`** — `Context<'sch, 'req, Adapter>`, the per-request bundle (connection manager, uri,
  route params, parsed request, and — lent by the router for the request — the `BaseUri` and `MountTable`,
  from which it lazily builds the per-request link generator), wrapped by a `ResourceContext` before it
  reaches a handler. Where request identifiers are materialised against the schema.
- **`request` / `responder` / `result` / `route_parameters` / `base_uri` / `mount_table` / `error`** — the
  request wrapper, response builders, the routing `Result`/`Error`, captured path params, the `BaseUri`
  link-rooting knob, the router's mount table, and the routing error type. (Link *generation* itself lives
  in `serialisation`; the router only supplies the base and the captured templates.)

### `json_api`

Pure (de)serialisation types mirroring the spec: `document`, `resource`, `identifier`,
`relationship`, `links`, `primary_content` (the `data` vs `errors` split), and `error`. No behaviour
beyond serde.

### `serialisation`

Turns database `Record`s into `json_api` documents. `to_document` assembles the top-level document
(primary `data` or `errors`, plus `included`); `make_record_resource` projects one record into a
`resource::Resource`. Links are rendered by a **`UriGenerator`** — a per-request view the router builds
and the factories drive — which resolves each record's `self`, relationship, and related links from the
`MountTable`'s templates, rooted at the `BaseUri`; a type or relationship slot that is not mounted yields
no link rather than a broken one. Dynamic segments are resolved through the record's controller
(`ResourceController::parameters_for_route`), leaving the generator as the sole source of link errors. Its
`Error` carries document-serialisation and link-generation faults — both internal (`500`), both folding
into `routing::Error` on the way out.

Both `to_document` and the generator are crate-private: the public serialisation entry point is
`Router::handle`, not this module.

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
- **`store`** — the read/write engine over `Table`: `fetch_record`/`fetch_collection`, record and
  collection `create`/`update`/`delete`, the related-resource fetches (`fetch_related_*`, plus id-only
  `peek_related_*`), and relationship persistence (`{link,relink,unlink}_{record,collection}`). Writes
  self-wrap a **re-entrant transaction** (depth 0 → `BEGIN`, deeper → `SAVEPOINT`) so composed store
  calls stay atomic. A create honours a client-supplied `record.id` by writing it into the insert row.
- **`record` / `attributes` / `relationships` / `composite`** — materialised rows and their
  field/relationship data.
- **`query_parameters`** — parses JSON:API query params — `include`, `fields`, `filter`, `sort`, `page`,
  and the impl-defined `search` — against a schema.
- **`query_builder` / `connection` / `pool` / `table`** — adapter-facing interfaces (traits).
- **`data_loader`** — relationship/include resolution; loads only the *solicited* relationships (sparse
  fieldsets are honoured), so nothing unrequested reaches the serialiser.
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
3. Deserialise the body (`serde_json::from_slice`) into a `Request`; build a `Context`
   (`Context::from_request`), lending it the router's `BaseUri` and `MountTable` so handlers can render
   links — the per-request `UriGenerator` is built lazily from them.
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

Provenance rides in the **type parameter**, not the borrow duration: the router lends the context its
`BaseUri<'sch>` and `MountTable<'sch, _>` as `&'req` borrows (they are held only for the request), yet the
schema-region data they point at stays typed `<'sch>`. A reference's lifetime says how long it is lent, not
where its data came from.

The column-name **keys** of `Attributes`/`Row` are `&'sch str`, interned at the two boundaries where
raw identifiers enter — request-in (`from_value`, `Context::require_record`) and `Table`-out
(`materialise_attributes`) — through the schema's self-named lookups. A request-supplied string can
therefore never *be* a column name in generated SQL; it can only ride into a binding as a value.

A handler is `for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch`.

## Error flow

Error types translate outward toward the wire, everything funnelling into `routing::Error`:

`database::Error` → `routing::Error` → `json_api::error::Error`, with `serialisation::Error`
(document-serialisation and link-generation faults) folding into `routing::Error` alongside them.

- `database::Error` is **source-classified**: each variant carries a single HTTP meaning exposed via
  `status()` / `code()` / `title()`, and a consumer-facing `Display` message.
- `routing::Error` is built either from a `database::Error` (a **lossless** `From`, status/code/title/
  detail preserved), from a `serialisation::Error` (surfaced as a `500`), or from a **named payload type**
  defined in `routing::error` (e.g. `RequiredParameterMissingError`, `ClientGeneratedIdNotSupportedError`)
  via its `From` impl — call sites raise the named type rather than inlining `Error::new`.
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

**Layout.** `test_support` is the fixture (a `ConnectionManager` + a `Router::try_new` mount over an
abstract five-resource schema, one read-only, one accepting client-generated ids, with a nullable
to-many for clearing/replacement tests), rooted at an absolute `BASE_URL`; `validations` holds the generic,
reusable validators (JSON:API grammar, full linkage, application URL set); the `mandatory` and
`recommended` modules hold the tests.

**Current state — partially red by design.** The suite runs partially red: the remaining failures are
this implementation's tracked conformance gaps — content-negotiation (`Accept` / `Content-Type`) is not
enforced, a couple of `recommended` niceties (the `Location` header on a `201`, human-readable `detail`
on a conflict) are unimplemented, and one query-parsing edge (equivalence of encoded and unencoded bracket
parameters) is unhandled. **Link generation is done**: `self`/relationship/related links now render
absolute against the router's configured `BaseUri`. The remaining gaps are an accepted, tracked liability
(see *Known rework* and the roadmap), not regressions.

## Features

Declared in `Cargo.toml`:

- **`sqlite`** *(default)* — pulls `rusqlite`, `base64`, `r2d2`, `r2d2_sqlite`; enables `SqliteAdapter`.
- **`builtin_migrations`** — pulls `include_dir` to embed migration files. Off in dev/test.

## Known rework

The near-term plan reshapes parts of this document; treat the following as in-flux:

- The key-bearing signatures (`Attributes`, `ForeignKeys`, `Relationships`, `QueryParameters`) now key
  on schema-borrowed `&'sch str` under a "parse, don't validate" layer. The value side stays **owned**:
  a request-borrowed `Attribute` value (`Cow<'req, str>`) was explored and dropped — serde cannot
  zero-copy a map/container value into `Cow` without a hand-written borrowing `Deserialize`, so the win
  did not justify the machinery. Likewise a `Cow<'static, str>` error payload was dropped: every error
  funnels into the owned wire type (`json_api::error::Error`) and is re-owned there, so an upstream
  borrow only relocates the allocation rather than removing it.
- **Content negotiation** (`Accept` / `Content-Type` validation → `406` / `415`) is not yet enforced.
- The **controller model** will grow: `ResourceContext` becomes a user-extensible per-request
  controller (`new(schema, context)` + user fields), and `Context` is renamed `RoutingContext`.
