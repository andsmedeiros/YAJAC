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
- **Lossless outward, redacted at the boundary.** `From<database::Error> for routing::Error` preserves
  status/code/title/detail. 5xx **detail redaction** (to a generic `InternalServerError`) and
  **logging** happen *only* at the router response boundary (`Router::handle`), and only in non-debug
  builds — dev builds return full detail. Do not scatter redaction or logging into `From` impls or
  handlers.
- **`cfg!(debug_assertions)`** is the dev/non-dev signal (matches the migrator precedent).

## Panics & fallibility

- **Never `.unwrap()`.** Propagate with `?`, or use `.expect("message")` with a message stating the
  invariant. (The one bare-`Response::new` fallback in `Router::handle` is a deliberate
  last-resort infallible construction, not an `unwrap`.)

## Lifetimes

`'sch` and `'req` are load-bearing conventions, not incidental names — see ARCHITECTURE.md. `'sch` =
trusted, schema/binary-scoped strings (the only ones allowed into SQL text). `'req` = untrusted,
request-scoped data (bindings only). Thread them consistently; a new structure that holds identifiers
should key them at `'sch`.

## Schema definition

- Schemas are defined through **`SchemaBuilder`** (the public, intended API), never by constructing a
  `TableSchema` directly (`TableSchema::new` is `pub(crate)`); the registry validates and mints them.
  The relationship DSL reads directionally: `Related::to(resource).pointing_own(fk).to_related(pk)` when
  the foreign key is on our table, `.pointing_related(fk).to_own(pk)` when it is on the related table.
- **Tests build through the registry.** Fixtures construct `SchemaBuilder`s, pass them to
  `Registry::try_new`, and take a `&TableSchema` via `registry.schema(name)` — they do not reach for
  the `pub(crate)` constructor.
- **Container order is observable.** Schema field/relationship order surfaces in generated SQL (column
  lists, `RETURNING`) and is test-pinned; keep order-preserving containers (`IndexMap`).

## Comments

- Comment the **shape and behaviour** of an entity, not the reasoning that produced it. No
  chain-of-thought, no circumstantial history ("we did this because earlier the bug…"), no
  restating the code.
- Default to **sparse**. Add a comment only where it adds understanding a reader can't get from the
  signature.

## Tests

- **Per-module.** Tests live in a `#[cfg(test)] mod tests` in the same file, or a sibling `tests.rs`
  for larger suites (`routing/tests.rs`, `database/data_loader/tests.rs`).
- **Exact assertions.** Assert exact collection contents and exact error *variants* — not inclusion
  checks, not "is an error." A test that accepts a superset has a hole.
- **Comprehensive over the public API.** Cover the whole surface of the entity under test.
- **`test-log`** surfaces log output in tests.
- Red tests may be committed to a dev branch; they must **never** reach `main`.

## Formatting & build

- **`cargo fmt` is mandatory** and gates commits. `rustfmt.toml` sets `imports_granularity = "Crate"`
  (a nightly-only option — stable `cargo fmt` prints a warning and skips it; harmless).
- Edition **2024**.
- **Zero warnings** at commit time, except the sanctioned dead-code scaffolding in `record.rs`
  (`Index::{get_mut,require,require_mut}`, `TableCache`, `Groupable`, `index_by`, `group_by`) — kept
  intentionally for upcoming work; do not delete it, do not let *new* warnings hide among it.

## Adapters & features

- Backends are feature-gated (`#[cfg(feature = "sqlite")]`) and implement `database::adapters::Adapter`.
  `default = ["sqlite"]`.
- `builtin_migrations` stays **off** in dev/test.
- A `Registry` over a real pool must stay `Send + Sync` (asserted in `adapters::tests`); don't
  introduce non-shareable state into the request-handling path.

## Pre-commit gate

Before every commit, run a tidy-up pass and fix (not just report) what it finds:

1. `cargo fmt` clean,
2. `cargo build` warning-free (modulo the sanctioned scaffolding),
3. `cargo test` green,
4. no stray debug artifacts, leftover comments, or typos.

Commits are **granular** — one or few files, logically ordered; never lump a whole feature into one
commit. Commit messages state motivation (when relevant) and describe the changes across files; they
do **not** narrate the conversation, reasoning, or contain an AI co-author trailer.
