//! Recommended conformance — the spec's SHOULD/RECOMMENDED rules.
//!
//! A server may realise our contract fully and still decline these; red here is
//! a recommendation skipped, not a contract violation. A SHOULD that rides on an
//! optional feature (multi-field `sort`) guards on that feature's non-support
//! signal, as in the mandatory tier.

mod creating_resources;
mod deleting_resources;
mod errors;
mod sorting;
mod sparse_fieldsets;
mod updating_resources;
