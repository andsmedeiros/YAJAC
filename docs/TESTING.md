# Testing guide

The conventions every test in this crate follows. They are cumulative — a correction made once
applies everywhere afterwards.

## Shape of a test

- Every test returns `test_support::Result` and propagates with `?`. A test panics only where it
  asserts.
- Names are sentences describing the invariant: `show_of_a_missing_record_is_not_found`. The `test_`
  prefix is redundant and is not used.
- `#[test]` comes from `test_log::test`, everywhere, so `RUST_LOG=trace` reaches every suite.
- A unit is tested at its own boundary, in its own module's suite. A controller is tested by calling
  the controller, not by dispatching through a router.

## Assertions

- `assert_eq!` is the default. `assert!` is tolerated where equality does not apply.
- **Never** `matches!`, `unwrap`, `expect`, or `match { … _ => panic!() }`.
- Assert exact contents, not inclusion: a collection assertion names every member it expects.
- Errors are asserted by value, which needs no unwrapping and pins the payload:

  ```rust
  assert_eq!(store.fetch_record(schema, missing, &parameters).err(), Some(Error::RecordNotFound));
  ```

- Never assert on an error's message text. Messages change with rewording and translation; assert the
  variant, the status, or a structural property.
- `Document` and `Resource` are serialisation concerns. A test whose subject is not serialisation
  leaves them as early as it can and asserts in record terms, where an attribute is an `Attribute`
  rather than a `serde_json::Value`. Serialising a response to `Value` and walking it by pointer is
  never the way.

## Where expectations come from

- Never compute an expected value with the crate's own helpers, and never derive test inputs from its
  internals. An expectation produced by the code under test cannot fail when that code is wrong.
- Prefer declared data (a constant, a literal, a fixture) or a third-party constructor. To build a
  `DateTime`, use the library's own type — `DateTime("2018-03-14T09:00:00Z".parse()?)` — not our
  conversion functions.

## Building the world

- **Never hand-assemble anything the framework parses or validates.** Route parameters, query
  parameters and mount tables are produced by the framework from real input; a test builds the real
  input and lets it do so. Grab the already-built pieces and stitch them together.
- Build from the outside in: construct a real `Router` through the ordinary builders, then take what
  it produced.
- The harness builds everything strictly beneath the unit under test; the test builds the unit
  itself.
- Do not widen a library item's visibility when the information is already at hand. A base URI handed
  to `Router::try_new` is already the test's — it does not need a getter.
- Do not alter library code to suit a test. Where the library genuinely lacks something a consumer
  would want (`Clone` on a value type), that is a library decision, made on its own merits.
- Data is rows or records. YAJAC has no models and no ORM, so no test invents typed per-table
  structs.
- There is no seed shared across the crate. A test inserts the rows its assertion is about; anything
  needed once it builds inline. Where most tests in one file start from the same population, that
  file may reach for a preset — always optional, always named, always called explicitly.

## Helpers

- Helpers are **fat**. Extract behaviour only when it is very reusable, very cumbersome, or very easy
  to get wrong. Everything else stays inline, where it can be read in place.
- A thin wrapper around a constructor — `existing(kind, id)` for an `Identifier`, `build_request` for
  `Request::builder` — buys nothing but indirection and a context switch. Do not write one.
- Name actions `verb_noun`: `build_registry`, not `registry`. A get-or-error accessor is
  `require_something`, matching the crate's own vocabulary.
- Import types at the top of the file. A signature reading
  `Result<&[crate::json_api::resource::Resource]>` is unacceptable.

## Comments

- Describe the entity — its shape and behaviour — from the outside, as a consumer needs it. Not where
  it is called from, not what pipeline it belongs to, not what it deliberately omits.
- No chain-of-thought, no history, no rationale for decisions already made.

## Layout

- A directory module puts its suite in a sibling `tests.rs`; a plain file module keeps an inline
  `#[cfg(test)] mod tests`. Tests never live in a `mod.rs`.
- A file module whose inline suite has outgrown it is promoted to a directory module, so its tests
  get a file of their own.
- Shared scaffolding lives in `src/test_support`, gated on `#[cfg(test)]`. It cannot live under
  `tests/`, which compiles as a separate crate and cannot see either the unit suites or the
  `pub(crate)` items they rely on.
- The conformance suite under `tests/conformance` is separate and stays black-box: it reaches the
  crate through the public API alone.
