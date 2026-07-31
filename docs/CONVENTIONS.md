# YAJAC — Conventions & Idiosyncrasies

Rules the codebase already follows. Keep new code homogeneous with them. This complements
[ARCHITECTURE.md](ARCHITECTURE.md), which describes structure; this file describes *practice*.

## Errors

- **Consumer-facing messages.** Every error `Display` is serialised to the API consumer. Write from
  the consumer's point of view; never leak internal logic or framework concepts (e.g. "the schema does
  not declare X" is internal — say what the *request* did wrong). Resource, field, and relationship
  **names** are fine to serialise — the implementation deliberately maps them to tables/columns.
- **Capitalised.** All error messages start capitalised, uniformly.
- **Single-purpose, source-classified variants.** Each `database::Error` variant carries one HTTP
  meaning, exposed exhaustively via `status()` / `code()` / `title()`. Query-param validation → 400
  (`QueryValidationFailure`), resource/document validation → 422 (`ResourceValidationFailure`),
  schema-consistency faults → 500 (`InconsistentSchema`), generic bad accessor input → 500
  (`InvalidAttributeAccess`), and so on. Pick the variant by *who caused it and what it means*, not by
  the current call site.
- **Named error types over inline construction.** An error raised at a call site gets a **named type
  defined in the error module** — a payload struct with `Display` + `StdError` + `From<…> for Error`
  (the sole place its status/code/title live), used via `?` / `.into()` (e.g. `routing::error`'s
  `RequiredParameterMissingError`, `ClientGeneratedIdNotSupportedError`). Don't inline
  `Error::new(status, code, title)` where the error is raised; keep construction in the error module,
  and prefer a chain that breaks on it (`cond.then_some(x).ok_or(TheError)?`) over an imperative
  `if !cond { return Err(…) }`.
- **Lossless outward, redacted at the boundary.** `From<database::Error> for routing::Error` preserves
  status/code/title/detail. 5xx **detail redaction** (to a generic `InternalServerError`) and
  **logging** happen *only* at the router response boundary (`Router::handle`), and only in non-debug
  builds — dev builds return full detail. Do not scatter redaction or logging into `From` impls or
  handlers.
- **`cfg!(debug_assertions)`** is the dev/non-dev signal (matches the migrator precedent).

## Panics & fallibility

- **No panicking inside the request path.** Nothing reachable from `Router::handle` may `.unwrap()`,
  `.expect()`, `unreachable!()`, `panic!()`, or otherwise panic. A "can't happen" invariant or a
  missing-but-expected value *there* is a broken server-side invariant, not grounds to abort the
  process: return the fitting `5xx` `Error` variant — minting a dedicated internal-error variant in
  preference to reusing an ill-fitting one — and let the router boundary redact + log it.
- **Never `.unwrap()`, anywhere.** Off the request path — construction, startup, tests — propagate with
  `?` or use `.expect("message")` with a message stating the invariant, and keep `expect` / `unreachable!`
  scarce. (The one bare-`Response::new` fallback in `Router::handle` is a deliberate last-resort
  infallible construction, not an `unwrap`.)

## Lifetimes

`'sch` and `'req` are load-bearing conventions, not incidental names — see ARCHITECTURE.md. `'sch` =
trusted, schema/binary-scoped strings (the only ones allowed into SQL text). `'req` = untrusted,
request-scoped data (bindings only). Thread them consistently; a new structure that holds identifiers
should key them at `'sch`.

## Schema definition

- Schemas are defined through **`SchemaBuilder`** (the public, intended API), never by constructing a
  `Schema` directly (`Schema::new` is `pub(crate)`); the registry validates and mints them.
  The relationship DSL reads directionally: `Related::to(resource).pointing_own(fk).to_related(pk)` when
  the foreign key is on our table, `.pointing_related(fk).to_own(pk)` when it is on the related table.
- **Tests build through the registry.** Fixtures construct `SchemaBuilder`s, pass them to
  `Registry::try_new`, and take a `&Schema` via `registry.schema(name)` — they do not reach for
  the `pub(crate)` constructor. Pure schema-only tests build a bare pool-free `Registry`; tests that
  touch the database wrap it in a `ConnectionManager` (`ConnectionManager::new(registry, pool)`) and
  reach schemas through `manager.registry()`.
- **Container order is observable.** Schema field/relationship order surfaces in generated SQL (column
  lists, `RETURNING`) and is test-pinned; keep order-preserving containers (`IndexMap`).

## Comments

- Comment the **shape and behaviour** of an entity, not the reasoning that produced it. No
  chain-of-thought, no circumstantial history ("we did this because earlier the bug…"), no
  restating the code.
- Default to **sparse**. Add a comment only where it adds understanding a reader can't get from the
  signature.

## Tests

Two layers, with different remits:

- **In-crate unit tests — per-module.** Tests live in a `#[cfg(test)] mod tests` in the same file, or a
  sibling `tests.rs` for larger suites (`routing/tests.rs`, `routing/controller/tests.rs`,
  `database/data_loader/tests.rs`). They
  assert **exact** collection contents and exact error *variants* — not inclusion checks, not "is an
  error." A test that accepts a superset has a hole. Cover the whole public surface of the entity.
- **Black-box conformance suite — `tests/conformance/`.** A separate integration target (registered as
  `[[test]] name = "conformance"`) driving `Router::handle` exactly as an embedder would, asserting
  *only* at the request→response boundary so it survives internal refactors. Structure and the contract
  it pins are in ARCHITECTURE.md; its authoring rules:
    - **Spec-first, never impl-first.** Derive every expectation from the JSON:API spec, never by
      reading the implementation. Each test carries the **verbatim spec clause** it encodes as a
      comment directly above it.
    - **Placed by obligation level.** MUSTs (and contract obligations) go in `mandatory`; SHOULDs in
      `recommended`. An invariant that is MUST/SHOULD *given support* for an optional feature lives at
      its true tier, guarded on the feature's spec-defined non-support signal (see ARCHITECTURE.md).
    - **Assert exactly what the spec mandates — which may be a bound, not equality.** Where the spec
      says "MUST NOT include *additional*", a subset check is correct, not exact equality; optional
      members are checked only when present (`if let Some`). Do **not** "tighten" a spec-bounded
      assertion into equality — and note we cannot assert data round-trips, since the server is free to
      alter the data it stores.
    - Tests return `Result` and use `?`; helpers only access/normalise response bits, assertions stay
      in the test body.
- **`test-log`** surfaces log output in tests (including the guarded-affordance skip messages).
- **Red on `main`:** ordinary unit/integration tests must be green on every branch and on `main`. The
  **conformance suite is the one sanctioned exception** — it is a spec-conformance *target*, red by
  design until the implementation is conformed (its reds are the tracked gaps). Ordinary red tests
  still must never reach `main`.

## Formatting & build

- **`cargo fmt` is mandatory** and gates commits. There is no custom `rustfmt.toml` — a former
  nightly-only `imports_granularity = "Crate"` setting was removed, as stable `cargo fmt` only warned
  on it and skipped it.
- Edition **2024**.
- **Zero warnings** at commit time.

## Adapters & features

- Backends are feature-gated (`#[cfg(feature = "sqlite")]`) and implement `database::adapters::Adapter`.
  `default = ["sqlite"]`.
- `builtin_migrations` stays **off** in dev/test.
- A `ConnectionManager` (registry + pool) must stay `Send + Sync` (asserted in `adapters::tests`);
  don't introduce non-shareable state into the request-handling path.

## Pre-commit gate

Before every commit, run a tidy-up pass and fix (not just report) what it finds:

1. `cargo fmt` clean,
2. `cargo build` warning-free,
3. `cargo test` green — **except** the conformance suite, whose by-design reds track un-implemented
   spec obligations (see *Tests*); no *new* red beyond those,
4. no stray debug artifacts, leftover comments, or typos,
5. `docs/` (`ARCHITECTURE.md`, `CONVENTIONS.md`) updated in the same commit (or an adjacent one) when
   the change altered documented structure or conventions — the docs track reality, they do not lag it.

Commits are **granular** — one or few files, logically ordered; never lump a whole feature into one
commit. Commit messages state motivation (when relevant) and describe the changes across files; they
do **not** narrate the conversation, reasoning, or contain an AI co-author trailer.
