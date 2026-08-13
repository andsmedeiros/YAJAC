//! Scaffolding for the crate's own test suites, offered in stages:
//!
//! | Stage | Entry point | Yields |
//! |---|---|---|
//! | schemas | [`schemas::build_registry`] | a validated registry over the shared schema set |
//! | database | [`database::build_connection_manager`] | that registry, bound to a fresh in-memory database |
//! | data | [`fixtures`] | named rows, inserted and handed back as persisted |
//! | routing | [`routing`] | request contexts, built from a router |
//!
//! Each stage stands on the ones above it.

pub(crate) mod database;
pub(crate) mod fixtures;
pub(crate) mod routing;
pub(crate) mod schemas;

mod tests;

/// The error a test propagates.
pub(crate) type Error = Box<dyn std::error::Error>;

/// The outcome of a test, defaulting to the `()` a test body yields.
pub(crate) type Result<T = ()> = std::result::Result<T, Error>;
