# YAJAC — Middleware Layer (design)

**Status: proposed — not yet implemented.** This note records the agreed design for an inside-route,
per-level middleware layer and the polyglot (raw + JSON:API) router it entails. It is the reference
for the implementation to follow; once landed, the relevant parts fold into `ARCHITECTURE.md`.

## Motivation

Middleware is declared **inside** normal routing definitions, interspersed with routes, and scoped by
an explicit block. A middleware inherits the context of its declaration surroundings: one declared at
the primary level receives a `PrimaryContext`, one declared under a resource receives a
`ResourceContext`. Each middleware can **constrain matching** (from the request's headers, URL, and
captured route parameters — a request either matches or falls through) and **wrap handling** (act on
the way in, call the next layer, act on the response coming back). The first concrete user is **content
negotiation** (`Accept` / `Content-Type` → 406/415).

Pursuing this exposes a deeper truth: the router is **not JSON:API-exclusive**. A user may serve
non-resourceful endpoints (a file download, a `multipart/form-data` upload) that never speak
`Document`. So the router becomes **polyglot**: raw at its base, with JSON:API as one tier entered at
the `resource::<T>()` boundary.

## Two tiers

The router has two tiers, each **homogeneous in representation** — this is what keeps middleware
simple (no generic body-transform machinery is needed).

| Tier         | Context           | Body            | Headers         | Fallibility                                 |
| ------------ | ----------------- | --------------- | --------------- | ------------------------------------------- |
| **Primary**  | `PrimaryContext`  | raw byte stream | none implicit   | fallible to the embedder (`Result` out)     |
| **Resource** | `ResourceContext` | `Document`      | JSON:API in/out | fallible; caught & rendered by the crossing |

The tiers meet at exactly one place: the **resource boundary**, entered implicitly by
`resource::<T>()` and its subordinate scopes (`member` / `collection`). Crossing it, in one step:

- **request:** parse the byte stream into a `Document` (`serde_json::from_reader`, straight off the
  stream — no intermediate buffer); validate `Content-Type` / `Accept` → 415 / 406;
- **schema binding:** build the `ResourceContext` (as today);
- **response:** serialise the `Document` → bytes, set `Content-Type: application/vnd.api+json`;
- **errors:** catch any resource-tier `Err`, render it as an error `Document`, serialise it.

Because the boundary catches and renders, every **expected** error — every JSON:API error — becomes a
proper serialised response there. Only a truly-unhandled fault escapes it (see *Error model*).

The boundary is the function `serve_resource`. It is **not** a formally registered transform (see
*Why `Document` is special-cased*); it is the concrete, special-cased crossing that `resource::<T>()`
installs.

## The two traits

One trait per tier, same skeleton. Both methods have defaults, so a match-only middleware overrides
just `matches`, a handle-only one just `handle`. `next` is a `&dyn Fn` (not a generic) so the traits
stay object-safe and instances can be stored per route. Middleware is passed **by value** and stored
**`Arc`-shared**: cheap to clone around the route table, and it may hold unique resources (a cache
handle) behind interior mutability. `Send + Sync` keeps the whole router shareable across worker
threads (the request path runs on whichever thread owns the connection manager —
`adapters::tests::sqlite_connection_manager_is_send_and_sync`), and it *forces* any interior mutability
onto sync primitives (`Mutex` / `RwLock`), which a cross-thread cache needs anyway.

```rust
pub trait PrimaryMiddleware<'sch, Adapter: AdapterInterface>: Send + Sync + 'sch {
    /// Constrain routing: `false` makes the wrapped routes not match this request. Runs during
    /// matching, before any context exists, from the headers, URL, and captured route parameters.
    fn matches(&self, _headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        true
    }

    /// Act on the context, call `next` (the rest of the chain), act on its response. Call `next`
    /// once, or skip it to short-circuit.
    fn handle<'req>(
        &self,
        context: PrimaryContext<'sch, 'req, Adapter>,
        next: &dyn Fn(PrimaryContext<'sch, 'req, Adapter>)
            -> Result<Response<Box<dyn Read + Send>>, Box<dyn std::error::Error>>,
    ) -> Result<Response<Box<dyn Read + Send>>, Box<dyn std::error::Error>>
    where
        'sch: 'req,
    {
        next(context)
    }
}

pub trait ResourceMiddleware<'sch, Adapter: AdapterInterface>: Send + Sync + 'sch {
    fn matches(&self, _headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        true
    }

    fn handle<'req>(
        &self,
        context: ResourceContext<'sch, 'req, Adapter>,
        next: &dyn Fn(ResourceContext<'sch, 'req, Adapter>) -> routing::Result,
    ) -> routing::Result
    where
        'sch: 'req,
    {
        next(context)
    }
}
```

`matches` sees the **headers, URL, and route parameters** — never the body: the body is a *stream* that
cannot be inspected during matching without consuming it, and body length is read from `Content-Length`
anyway. Because matching happens before the resource boundary, a `ResourceMiddleware` guard inspects
the same request head as a `PrimaryMiddleware` guard. Content negotiation is therefore **not** a match
guard: a failed guard makes the route fall through to a 404, whereas negotiation must answer 406/415, so
it lives in the crossing's `handle`-side, returning the right status in-band.

## The builder surface

Middleware is declared with an explicit block, like `scope` but tagging instead of prefixing. It
appears on the primary builders taking a `PrimaryMiddleware`, and on the resource / subordinate
builders taking a `ResourceMiddleware`.

```rust
Router::try_new(base_uri, |root| {
    root.middleware(RequestLog, |root| {                     // primary tier: sees bytes, wraps all below
        root.scope("api", |api| {
            api.get("health", health_check)                  // raw route: PrimaryContext + byte stream
               .resource::<Articles>("articles", &ARTICLES)  // JSON:API crossing installed implicitly
               .resource_with::<Users>("users", &USERS, |users| {
                   users.middleware(RequireAuth, |users| {   // resource tier: sees the Document
                       users.all_relationships()
                   })
               })
        })
    })
})
```

The builder carries the middleware active at its nesting level. `mount` stamps that list onto each
route it creates; `spawn` threads **both** inherited prefixes (route segments *and* middleware), so
`scope` extends the path while `middleware` extends the list. The list is a `Vec` of cheap
`Arc`-clones:

```rust
// RouteBuilder trait
fn spawn(
    &self,
    prefix: Vec<Cow<'sch, str>>,
    middleware: Vec<Middleware<'sch, Adapter>>,
) -> Self;

fn middleware(&self) -> &[Middleware<'sch, Adapter>];        // accessor for spawn/scope

fn scope(mut self, segment: &'sch str, build: impl FnOnce(Self) -> Self) -> Self {
    let child = build(self.spawn(self.extend_prefix(segment), self.middleware().to_vec()));
    self.routes_mut().absorb(child.into_routes());
    self
}
```

```rust
// PrimaryRouteBuilder — inherent, since the middleware type is tier-specific
pub fn middleware(
    mut self,
    middleware: impl PrimaryMiddleware<'sch, Adapter>,
    build: impl FnOnce(Self) -> Self,
) -> Self {
    let mut list = self.middleware.clone();
    list.push(Middleware::Primary(Arc::new(middleware)));
    let child = build(self.spawn(self.prefix.clone(), list));
    self.routes.absorb(child.into_routes());
    self
}

/// `scope` + `middleware` in one.
pub fn middleware_at(
    self,
    path: &'sch str,
    middleware: impl PrimaryMiddleware<'sch, Adapter>,
    build: impl FnOnce(Self) -> Self,
) -> Self {
    self.scope(path, |scope| scope.middleware(middleware, build))
}
```

`resource::<T>()` passes its accumulated middleware list into the `ResourceRouteBuilder`, so any
primary middleware carried into a resource end up **ahead of** that resource's own middleware in the
one list — the primaries-before-resources partition holds by construction.

The trait accessor is named `middleware()` and coexists with the inherent `.middleware(m, build)`
builder method — **verified to compile**: the accessor is only ever called from the generic default
method `scope`, where `Self: RouteBuilder` and inherent methods are invisible, so `self.middleware()`
there resolves unambiguously to the accessor, while the two-arg inherent method wins at concrete sites.

All builder blocks take a `build` closure. Only the closures that thread a real `RelationshipConfig`
(the `*_with` relationship options) keep the name `configure`, because those genuinely configure.

## Route storage & dispatch

A route holds **one** middleware list and **one** handler, each an enum; dispatch is a runtime match.
The list is partitioned primaries-first, then resources (route entities before resource entities).

```rust
enum Middleware<'sch, Adapter: AdapterInterface> {
    Primary(Arc<dyn PrimaryMiddleware<'sch, Adapter>>),
    Resource(Arc<dyn ResourceMiddleware<'sch, Adapter>>),
}

enum Handler<'sch, Adapter: AdapterInterface> {
    Primary(Box<dyn PrimaryHandler<'sch, Adapter>>),          // PrimaryContext -> byte response
    Resource { schema: &'sch Schema<'sch>, handler: Box<dyn ResourceHandler<'sch, Adapter>> },
}

struct Route<'sch, Adapter: AdapterInterface> {
    method: Method,
    path: Vec<Cow<'sch, str>>,
    middleware: Vec<Middleware<'sch, Adapter>>,
    handler: Handler<'sch, Adapter>,
}
```

Handlers are `Box` (one per route, owned, framework-generated); middleware is the `Arc`-shared value.
Storing plain lists and iterating them means **no type erasure** of the middleware and **no fold** into
a single composed closure — the wrapping is plain recursion.

**Matching** is one pass: URL / method as today (capturing the route parameters first), then every
middleware guard against the URL, headers, and those parameters:

```rust
fn matches(&self, method: &Method, segments: &[&str], uri: &Uri, headers: &HeaderMap)
    -> Option<RouteParameters>
{
    let params = self.match_path(method, segments)?;
    self.middleware.iter().all(|m| m.matches(headers, uri, &params)).then_some(params)
}
```

**Serving** walks the single list. While the head is `Primary`, run it on the `PrimaryContext`. Once
the primaries are exhausted, either the raw handler runs, or we cross into the JSON:API tier for the
remaining (all-`Resource`) middleware and the resource handler:

```rust
fn serve<'req>(
    middleware: &[Middleware<'sch, Adapter>],
    handler: &Handler<'sch, Adapter>,
    context: PrimaryContext<'sch, 'req, Adapter>,
) -> Result<Response<Box<dyn Read + Send>>, Box<dyn std::error::Error>> {
    match middleware.split_first() {
        Some((Middleware::Primary(m), rest)) => m.handle(context, &|context| serve(rest, handler, context)),
        _ => match handler {                                 // primaries exhausted
            Handler::Primary(h) => h(context),               // raw route
            Handler::Resource { schema, handler } => serve_resource(middleware, schema, handler, context),
        },
    }
}
```

`serve_resource` runs the crossing: parse the body into a `Document`, build the `ResourceContext`, run
the remaining `Resource` middleware as an onion around the handler, and serialise out (rendering an
error `Document` on `Err`).

> **Note.** The `for<'req>` lifetime on `serve`'s recursive continuation is the one spot expected to
> need care against the borrow checker — the same higher-ranked bound the current `Handler` already
> satisfies, applied to a recursive closure.

## Streams

Bodies are `std::io` byte streams both ways — a `File` streams straight out for a download, the
crossing drains the input `Read` (via `from_reader`) to parse and returns a `Cursor` over the
serialised document. This is a performance choice (no full-body buffering) and it accommodates
non-JSON:API payloads.

```rust
// PrimaryContext
body: Box<dyn Read + Send + 'req>,   // was Option<Document>

// Router — fallible: the final disposition of an unhandled internal fault is the embedder's
pub fn handle(
    &self,
    database: &'sch ConnectionManager<'sch, Adapter>,
    request: http::Request<Box<dyn Read + Send>>,
) -> Result<http::Response<Box<dyn Read + Send>>, Box<dyn std::error::Error>>;
```

## Error model

`Router::handle` is **fallible**, returning `Result<Response, Box<dyn std::error::Error>>`. The library
renders every **expected** error itself: all JSON:API errors are turned into proper error documents
inside the crossing. What reaches the top as `Err` is therefore genuinely exceptional — a raw primary
handler that returned `Err`, or a catastrophic failure while the crossing was rendering (serialisation
itself failing). That fault is handed to the **embedder** to dispose of per its environment: log and
alert, map to a 500, or assert in a test.

This is deliberate. YAJAC does not own the socket, so the final disposition of an unhandled internal
fault is embedder policy, not the library's. Fallible **preserves** the fault; a bodyless-500 caught
internally would **destroy** it (only a log would survive). It is also strictly more general — an
embedder that wants bodyless-500 collapses to it in one line:

```rust
let response = router.handle(db, request).unwrap_or_else(|_| bodyless_500());
```

which the crate can ship as a convenience helper. The reverse — recovering an error a bodyless-500
router already swallowed — is impossible.

## Why `Document` is special-cased

The crossing is deliberately **not** modelled as a formally registered body transform. A general
`BodyTransform` (`parse: stream → Inner`, `serialise: Response<Inner> → Response<stream>`) is
expressible, but does not fit:

1. **The contexts are purpose-built, not generic over body.** `ResourceContext` is not "`PrimaryContext`
   with an `Inner` body" — it is a distinct type with a `Document`-shaped API (`require_record`, …). A
   formal transform would force the contexts to become generic over the inner representation,
   genericising the whole context machinery for a single real instance.
2. **The crossing does more than transform the body** — it also **binds the schema**
   (`resource::<T>()`), which is not a body transform at all.

`Document` genuinely is a special type deserving special handling, and it is **not optional**: a
resourceful endpoint *is* JSON:API, mandated headers included — a `Document` served without the correct
`Content-Type` is not a valid JSON:API response. `resource::<T>()` creating a resource segment *is* the
registration; there is no separate transform mechanism and no opting out.

## Consequences & migration

- **Rename `Context` → `PrimaryContext`** (parity with `PrimaryRouteBuilder` / `PrimaryMiddleware`).
- `PrimaryContext.body`: `Option<Document>` → `Box<dyn Read + Send>`. The dispatch-time
  `serde_json::from_slice` moves **into** the crossing as `from_reader`, so a malformed body becomes a
  clean resource-tier 400 rather than a dispatch failure.
- The `Document`-consuming API (`require_resource` / `require_record` / `require_relationship` /
  `require_linkage` / `materialise_id`) **moves down** from `PrimaryContext` to `ResourceContext`, which
  owns the parsed `Document`. Bare `PrimaryContext` exposes only the raw stream.
- `Router::handle` returns `Result<http::Response<Box<dyn Read + Send>>, Box<dyn std::error::Error>>`;
  **YAJAC now owns serialisation**. The embedder receives bytes (or the residual fault), not a `Document`.
- Resource handlers still build `Response<Option<Document>>`; `respond` / `no_content` are untouched —
  the crossing serialises. `default_response`'s hardcoded `Content-Type: application/vnd.api+json` moves
  into the crossing.
- The old router-root JSON:API error-document fallback is gone from the top: expected errors are rendered
  in the crossing, and the unhandled residual propagates as `Err` to the embedder.

## Deferred / open

- **Content negotiation** is realised inside the crossing (406/415), closing the four mandatory
  content-negotiation conformance reds.
