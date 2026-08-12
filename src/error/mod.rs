//! The crate's error funnel.
//!
//! Every layer owns its own error vocabulary and knows nothing of the ones above it; this module
//! sits above all of them and projects each into the JSON:API error object that is ultimately
//! serialised into an error document. The funnel *is* the wire type — there is no parallel
//! framework-side copy of it — and the projections live here rather than beside either the source
//! or the target error, so that `json_api` stays pure spec modelling and no layer below has to
//! learn what a JSON:API error object is.
//!
//! Each layer error answers the same three questions — its status, its stable `code`, and its
//! occurrence-independent `title` — leaving `Display` to supply the per-occurrence `detail`.
//!
//! `Source` and the [`pointer`] builders are shared here for the same reason the error object is:
//! an error should name its origin with everything the raising site knows, and only that site knows
//! it. What a layer may name is bounded by what it can see — the database names a query parameter,
//! never a document pointer, since it holds no document to point into.

pub mod pointer;

use crate::{
    database::error::Error as DatabaseError, serialisation::error::Error as SerialisationError,
};

pub use crate::json_api::error::{Error, Source};

impl From<DatabaseError> for Error {
    fn from(error: DatabaseError) -> Self {
        Error {
            status: Some(error.status()),
            code: Some(error.code().to_string()),
            title: Some(error.title().to_string()),
            detail: Some(error.to_string()),
            ..Error::default()
        }
    }
}

impl From<SerialisationError> for Error {
    fn from(error: SerialisationError) -> Self {
        Error {
            status: Some(error.status()),
            code: Some(error.code().to_string()),
            title: Some(error.title().to_string()),
            detail: Some(error.to_string()),
            ..Error::default()
        }
    }
}
