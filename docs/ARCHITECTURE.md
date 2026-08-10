# YAJAC — Architecture

**Y**et **A**nother **J**SON:**A**PI **C**rate. A server-agnostic Rust implementation of the
[JSON:API v1.1](https://jsonapi.org/format/) specification over a pluggable, schema-driven data
layer. Edition 2024.

The crate produces JSON:API responses from HTTP requests; it does **not** own a socket. The routing
layer takes an `http::Request<ByteStream>` — a request whose body is a streamed `Read` — and returns a
streamed `http::Response`, leaving transport (a web framework, a Tauri command, a test harness) to the
embedder. `Router::handle` is **fallible**: every expected JSON:API error is rendered into a document
internally, so what escapes to the embedder as `Err` is a genuinely exceptional fault (see *Request
lifecycle*). YAJAC owns serialisation — the embedder receives bytes, not a `Document`.

## Module map

The crate root (`src/lib.rs`) exposes six top-level modules, layered from the wire inward:

| Module          | Role                                                                                                        |
| --------------- | ---------------------------------------------------------------------------------------------------------- |
| `routing`       | HTTP surface: two-tier route matching and middleware, controllers, per-request context, the JSON:API boundary. |
| `json_api`      | JSON:API v1.1 document model — the serialised wire types.                                                   |
| `serialisation` | Builds JSON:API documents from records: the document factory (`to_document`), link generation (the `UriGenerator` trait over a `BaseUri`), the `ByteStream` body currency, and its error type. |
| `database`      | Schema-driven data layer: schema, registry, store, records, query building, adapters.                      |
| `http_wrappers` | Serde-friendly newtypes over the `http` crate (`StatusCode`, `Uri`).                                       |
| `utils`         | Generic, domain-agnostic helpers — the `indexing` iterator adaptors and the `MediaType` header parser.     |

### The two tiers

The router is **polyglot**, not JSON:API-exclusive: an embedder may serve raw endpoints (a file
download, a `multipart/form-data` upload) that never speak `Document`. So routing has two tiers, each
**homogeneous in representation** — which is what keeps middleware simple:

| Tier         | Context           | Body                | Result type       | Fallibility                                  |
| ------------ | ----------------- | ------------------- | ----------------- | -------------------------------------------- |
| **Primary**  | `PrimaryContext`  | raw byte stream     | `PrimaryResult`   | fallible to the embedder (`Box<dyn Error>`)  |
| **Resource** | `ResourceContext` | `Document`          | `ResourceResult`  | fallible; caught and rendered at the boundary |

The tiers meet at the **resource boundary**, entered by `resource::<T>()` and its `member` scope. There
the byte stream is bound to a schema (`ResourceContext`), the JSON:API boundary middleware runs, and the
resulting `Document` is serialised back to bytes. Every builder is **single-tier**: a builder mounts
either raw routes or schema-bound routes, never both, so a route's middleware is tier-ordered by
construction (schema-less first, then schema-bound).

### `routing`

The embedder-facing layer.

- **`router`** — `Router`, assembled by `Router::try_new(base_uri, |root| …)` which runs a
  `PrimaryRouteBuilder`. A `Router` holds a flat, ordered list of `Route`s (method + path template +
  middleware chain + `EndpointHandler`), the `MountTable` its handlers resolve controllers and link
  templates through, and the `BaseUri` that roots every link it mints. Routes are **eagerly built and
  validated** (a resource mounted twice, a relationship name the schema doesn't declare, or a schema-bound
  middleware wrapping a raw route, is a build-time `RouterError`). `Router::handle` is the single
  request→response boundary; `serve` / `serve_resource` walk a matched route's middleware chain (see
  *Request lifecycle*). `root.resource::<T>(scope, schema)` wires a resource's full default CRUD **and**
  every relationship endpoint; `resource_with` is the custom form (mounts only what its closure asks
  for — the default endpoints are opt-in via `default_endpoints()`); `read_only_resource[_with]` refuses
  writes with `403`.
- **`middleware`** — the two-tier middleware layer. `PrimaryMiddleware` and `ResourceMiddleware` (one per
  tier, same skeleton: a `matches` guard consulted during routing, and an around-`handle` that calls
  `next`) are declared **inside** routing blocks and wrap every route below. Middleware is passed by value
  and stored `Arc`-shared. The **`json_api`** submodule is the boundary itself: `JsonApi`, a stateless ZST
  seeded as the outermost schema-bound middleware at every resourceful route, which negotiates content
  (`415` / `406`), catches a resource-tier error and renders it into an error document (redacting a 5xx),
  and stamps the JSON:API `Content-Type` (plus any applied `filter` / pagination profile).
- **`builders`** — the route-builder DSL behind `Router::try_new`. `PrimaryRouteBuilder` (root and nested
  `scope`s) mounts **raw** routes via `UnboundVerbs` (`get`/`post`/… → a `PrimaryContext` handler), takes a
  **primary** `.middleware` (and `.middleware_at`), and opens resources. `ResourceRouteBuilder` is
  **schema-bound**: its `ResourceVerbs` (`get`/`post`/…) mount collection-scoped `ResourceContext`
  handlers, `member` opens a record-scoped (`:id`) block, the relationship methods mount linkage/related
  endpoints, `default_endpoints()` mounts the default CRUD, and `.middleware` takes a **resource**
  middleware. `SubordinateRouteBuilder` is the `member` body (shares `ResourceVerbs`, resource
  `.middleware`). Both `.middleware` forms and `scope` are **spawn-then-absorb** (`spawn_with_path` /
  `spawn_with_middleware` / `spawn_with` extend a level's path and/or middleware). `RelationshipConfig` /
  `RelationshipsConfig` carry per-relationship options (read-only, path/keyword relocation via `*_with`).
- **`controller`** (a directory module — `controller/{mod.rs, tests.rs}`) — `ResourceController`, the trait
  an embedder implements per resource as a **stateless marker type**. `DefaultController` is the
  no-customisation impl. Its default handler methods receive a `ResourceContext<'sch, 'req, Adapter>` —
  the resource's schema paired with the request `PrimaryContext` — and reach record parsing (off the
  streamed body), query parameters, the store, and id resolution through it, already bound to the schema.
  It carries `parameters_for_route`, which resolves a mounted route's dynamic segments (`:id` from the
  record, others echoed from the request) for link rendering — **infallible**, omitting anything it cannot
  resolve. Overriding `configuration()` returns a `Configuration` shaping framework behaviour (today,
  whether the resource accepts **client-generated ids**).
- **`context`** — `PrimaryContext<'sch, 'req, Adapter>`, the raw-tier per-request bundle (connection
  manager, uri, route params, headers, the streamed body, and — lent by the router — the `BaseUri` and
  `MountTable`, from which it lazily builds the per-request link generator). The body is taken by value
  (`require_body`); `contains_body` probes and caches whether it carries content (one byte, prepended back)
  so negotiation and parsing agree without re-reading. `ResourceContext` wraps it (and `Deref`s to it),
  adding schema-bound document parsing (`require_record` / `require_resource` / `require_linkage`,
  `parse_body` straight off the stream) and the cached, own-schema query parameters.
- **`request` / `responder` / `result` / `route_parameters` / `base_uri` / `mount_table` / `error`** — the
  primary request wrapper (`PrimaryRequest`), response builders, the tier result aliases
  (`PrimaryResult` / `ResourceResult`), captured path params, the `BaseUri` link-rooting knob, the router's
  mount table, and the routing error type.

### `json_api`

Pure (de)serialisation types mirroring the spec: `document`, `resource`, `identifier`,
`relationship`, `links`, `primary_content` (the `data` vs `errors` split), and `error`. No behaviour
beyond serde.

### `serialisation`

Turns database `Record`s into `json_api` documents. `to_document` assembles the top-level document
(primary `data` or `errors`, plus `included`); `make_record_resource` projects one record into a
`resource::Resource`. Links are rendered through the **`UriGenerator` trait**, which the factories drive
oblivious to which implementor they hold:

- **`CanonicalUriGenerator`** — a per-request view the router builds; resolves each record's `self`,
  relationship, and related links from the `MountTable`'s templates, rooted at the `BaseUri`. A type or
  relationship slot that is not mounted yields no link rather than a broken one; dynamic segments resolve
  through the record's controller (`parameters_for_route`).
- **`NullUriGenerator`** — a ZST whose every method fails, passed where a document renders **no** per-record
  links (an errors document). It makes that contract explicit: a link request against it is a framework
  bug surfaced loudly, never a fabricated link.

`ByteStream` (`Box<dyn Read + Send>`) is the body currency both ways — a request body streamed in, a
serialised document streamed out. The generator's `Error` carries document-serialisation and
link-generation faults (both internal `500`, both folding into `routing::Error`). Both `to_document` and
the generators are crate-private: the public serialisation entry point is `Router::handle`.

### `database`

The engine. Schema-driven and adapter-generic.

- **`schema`** — `Schema<'sch>` and its parts (`PrimaryKey`, `AttributeType`, `ColumnDescriptor`,
  `RelationshipDescriptor`, `RelationshipKind`, `RelatedResource`, `RelationshipKeys`). Owned `IndexMap`
  containers keyed by borrowed `&'sch str`: O(1) lookup with **definition order preserved** (that order
  is observable in generated SQL). The `attribute`/`foreign_key`/`relationship` lookups return the
  matching *descriptor*, which carries the schema's own `&'sch` name alongside its type — so a lookup hands
  back a self-named, trusted identifier. Built by an ergonomic **`SchemaBuilder`** — the public way to
  define a schema — which collects inert `SchemaParts` that the registry validates and mints into
  `Schema`s (`Schema::new` is `pub(crate)`).
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
  and the impl-defined `search` — against a schema. A `filter[field]` value carries an operator
  (`eq:`, `in:`, …).
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

## Middleware layer

Middleware is declared **inside** normal routing definitions, interspersed with routes and scoped by an
explicit block, and inherits the tier of its surroundings: a `.middleware` on a primary builder takes a
`PrimaryMiddleware`, one on a resource/member builder takes a `ResourceMiddleware`. Each can **constrain
matching** (`matches`, from the request head and captured route parameters — a rejected request falls the
route through to the next, or to a 404) and **wrap handling** (act on the way in, call `next`, act on the
response — or skip `next` to short-circuit).

**Storage & dispatch.** A `Route` holds one ordered `Vec<Middleware>` and one `EndpointHandler`
(`Primary` byte handler, or `Resource { schema, handler }`). `serve` walks the chain: each leading
primary middleware runs against the `PrimaryContext`, wrapping the recursive call for the rest; once they
are exhausted, a raw handler runs directly, or the route crosses into `serve_resource`, which builds the
`ResourceContext`, runs the schema-bound middleware onion around the handler, and serialises the
`Document` to bytes. A schema-less middleware found in the schema-bound chain is a partition violation —
an internal `500`, never silently skipped.

**The JSON:API boundary.** `JsonApi` (in `middleware::json_api`) is the outermost schema-bound
middleware, seeded **once** in `ResourceRouteBuilder::new` so it wraps every route the resource emits.
It is where the tier *becomes* JSON:API: content negotiation (`Content-Type` mandatory on a body-carrying
request → `415`; `Accept` → `406`; unsupported `ext` rejected, `profile` ignored), error rendering (any
resource-tier `Err` → an error document, with a 5xx redacted and logged), and `Content-Type` stamping.
Negotiation and the parser share the context's cached `contains_body`, so the body is probed at most once.

**Tier integrity.** Because every builder is single-tier, the schema-less→schema-bound partition holds by
construction and a raw route can never carry a schema-bound middleware. `RouteBuilder::mount` still guards
it (recording a `RouterError::ResourceMiddlewareOnPrimaryRoute` in place of the route) — a build-time
backstop that outlives the type-level guarantee. A raw route that shares a resource's path space is
mounted on the **primary** tier (e.g. `root.scope("articles", |s| s.get("download", …))`); ordering in the
flat route list is the disambiguator, so it is declared where its precedence is wanted.

## Request lifecycle

`Router::handle(&self, database: &'sch ConnectionManager<'sch, Adapter>, request: PrimaryRequest) -> PrimaryResult`:

1. Extract `uri`, `method`, and non-empty path segments.
2. Find the first `Route` whose method + path template matches **and** whose every middleware `matches`
   guard admits the request head, capturing `:param` segments into `RouteParameters`. No match → a bare
   bodyless `404`.
3. Build a `PrimaryContext` (`from_request`) from the streamed body, headers, and route params, lending it
   the router's `BaseUri` and `MountTable`.
4. `serve` runs the middleware chain. Leading primary middleware wrap the `PrimaryContext`; then either a
   raw handler runs, or `serve_resource` crosses into the resource tier — building the `ResourceContext`,
   running the `JsonApi` boundary and any inner resource middleware around the controller, and serialising
   the returned `Document` to a byte stream.
5. **The boundary is the `JsonApi` middleware**, not the router: negotiation, error-document rendering,
   5xx redaction (full `Debug` logged before hiding), and `Content-Type` stamping all live there, at the
   one place the resource tier funnels through.
6. What reaches the top as `Err` is only a residual, exceptional fault (a raw handler's error, or a
   failure *while* rendering) — handed to the embedder to dispose of per its environment.

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
raw identifiers enter — request-in (`from_value`, `ResourceContext::require_record`) and `Table`-out
(`materialise_attributes`) — through the schema's self-named lookups. A request-supplied string can
therefore never *be* a column name in generated SQL; it can only ride into a binding as a value.

A raw handler is `for<'req> Fn(PrimaryContext<'sch, 'req, Adapter>) -> PrimaryResult + Sync + Send + 'sch`
(`PrimaryEndpointHandler`); a schema-bound handler is the `ResourceContext` / `ResourceResult` analogue
(`ResourceEndpointHandler`).

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
- Redaction of 5xx detail happens at the JSON:API boundary, never in the `From`.

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
to-many for clearing/replacement tests), rooted at an absolute `BASE_URL`. It stands in for the client:
it streams the request body and, since a conformant client sends `Content-Type` with any body, defaults
that header when a test did not set one. `validations` holds the generic, reusable validators; the
`mandatory` and `recommended` modules hold the tests.

**Current state — partially red by design.** All mandatory MUSTs pass, including content negotiation.
The remaining `recommended` failures are this implementation's tracked SHOULD-level gaps: the `Location`
header on a `201`, human-readable `detail` on a conflict, and one query-parsing edge (equivalence of
encoded and unencoded bracket parameters — accepted as *not a bug*, since we accept percent-encoded query
*values*, not param *names*). These are an accepted, tracked liability (see *Known rework* and the
roadmap), not regressions.

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
  did not justify the machinery.
- `ResourceController::parameters_for_route` keys its resolved map on `&'sch str`, which forces the link
  generator to treat only `Cow::Borrowed` template segments as dynamic — an owned `:param` segment is
  mishandled. No template produces one today; the fix (widen the parameter names to the request lifetime
  `'req`) is logged in the bug ledger.
- The **controller model** will grow: `ResourceContext` becomes a user-extensible per-request
  controller (`new(schema, context)` + user fields).
- A **prelude** module is planned so embedders import the builder traits (`RouteBuilder`, `UnboundVerbs`,
  `ResourceVerbs`) in one line.
- **Glob route matching** (a trailing `*name` catch-all segment) is the next routing feature — it enables
  a JSON:API-tier `404` handler rather than the bare root `404`.
