//! Mandatory conformance — obligations of *any* implementation of our public
//! contract.
//!
//! Two sources feed this tier: the spec's MUST-level rules, and the arbitrary
//! contract we publish (schema, locations, and each resource's read-only or
//! read-write action mode). Invariants that are MUST *given support* for an
//! optional feature (`include`, `sort`, client-generated ids) live here too, at
//! their true obligation level, guarded on the feature's spec-defined
//! non-support signal — see `test_support::enforced`.

mod client_generated_ids;
mod content_negotiation;
mod creating_resources;
mod deleting_resources;
mod errors;
mod fetching_relationships;
mod fetching_resources;
mod inclusion;
mod query_parameters;
mod read_only_resources;
mod sorting;
mod sparse_fieldsets;
mod updating_relationships;
mod updating_resources;
