//! Shared scaffolding for the crate's own test suites.
//!
//! The harness builds everything strictly *beneath* the unit under test; the unit itself is always
//! built by the test. A store test takes a connection manager and a connection and writes
//! `Store::new` with its own hands; a router test takes a connection manager and writes
//! `Router::try_new`. Nothing here hands a test the layer it is meant to be exercising.
//!
//! The stages compose upwards, and a test stops at whatever depth its subject sits at:
//!
//! | Stage | Constructor | Serves |
//! |---|---|---|
//! | schemas | [`schemas::build_registry`] | registry, schema, builder, query parameters, query builder, attributes |
//! | database | [`database::build_connection_manager`] | table, connection, store, data loader |
//! | data | [`fixtures`] | anything standing on the database stage |
//!
//! A schema-level suite stops at the first stage and never opens a database.
//!
//! The conformance suite keeps its own, separate fixture: it drives the crate through the public API
//! alone, which the compiler enforces by keeping the internals used here out of its reach.

pub(crate) mod database;
pub(crate) mod fixtures;
pub(crate) mod schemas;

mod tests;

/// The error every test propagates: any failure short of a failed assertion is one `?` from ending
/// the test.
pub(crate) type Error = Box<dyn std::error::Error>;

/// The outcome of every test in the crate, defaulting to the bare `()` a test body yields. Tests
/// propagate with `?` and panic only in assertions.
pub(crate) type Result<T = ()> = std::result::Result<T, Error>;
