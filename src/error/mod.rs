//! The crate's error funnel.
//!
//! Every layer owns its own error vocabulary and knows nothing of the ones above it; this module
//! sits above all of them and drains each into one type. That type is *not* the wire type: it is
//! shaped for travelling the stack — a mandatory status, borrowed `code` and `title`, and the two
//! rare members boxed — while [`crate::json_api::error::Error`] stays a faithful model of the
//! standard's error object, shaped for serialisation. The conversion between them happens once, at
//! the boundary that renders an error document.
//!
//! The projections live here rather than beside either the source or the target error, so that
//! `json_api` stays pure spec modelling and no layer below has to learn what a JSON:API error
//! object is.
//!
//! **Nothing constructs this type inline.** Each layer names a variant of its own error enum, which
//! answers the same three questions — its status, its stable `code`, and its occurrence-independent
//! `title` — leaving `Display` to supply the per-occurrence `detail`. A downstream embedder joins
//! the same way, by implementing `From<TheirError>` for this type.

#[cfg(test)]
mod tests;

pub mod pointer;

use crate::{
    database::error::Error as DatabaseError, http_wrappers::StatusCode,
    json_api::error::Error as JsonApiError, routing::error::Error as RoutingError,
    serialisation::error::Error as SerialisationError,
};
use serde_json::Value;
use std::{
    borrow::Cow,
    error::Error as StdError,
    fmt::{Display, Formatter},
};

pub use crate::json_api::error::Source;

/// A failure on its way out of the framework, carrying everything the raising site knew about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    /// The HTTP status the failure mandates. Always known — the layer that raised it decided.
    pub status: StatusCode,
    /// Stable, machine-readable identifier for the class of failure.
    pub code: Cow<'static, str>,
    /// Human-readable summary of the class of failure, identical across occurrences.
    pub title: Cow<'static, str>,
    /// Human-readable account of *this* occurrence.
    pub detail: String,
    /// What in the request caused it, when the raising site could name it truthfully.
    pub source: Option<Box<Source>>,
    /// Non-standard detail worth carrying to the client.
    pub meta: Option<Box<Value>>,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.title, self.code, self.detail)
    }
}

impl StdError for Error {}

impl From<DatabaseError> for Error {
    fn from(error: DatabaseError) -> Self {
        Error {
            status: error.status(),
            code: Cow::Borrowed(error.code()),
            title: Cow::Borrowed(error.title()),
            detail: error.to_string(),
            source: None,
            meta: None,
        }
    }
}

impl From<SerialisationError> for Error {
    fn from(error: SerialisationError) -> Self {
        Error {
            status: error.status(),
            code: Cow::Borrowed(error.code()),
            title: Cow::Borrowed(error.title()),
            detail: error.to_string(),
            source: None,
            meta: None,
        }
    }
}

impl From<RoutingError> for Error {
    /// Drains a routing fault whole: every accessor delegates through the funnelled variants, so a
    /// nested database or serialisation failure arrives exactly as it would on its own.
    fn from(error: RoutingError) -> Self {
        Error {
            status: error.status(),
            code: Cow::Borrowed(error.code()),
            title: Cow::Borrowed(error.title()),
            detail: error.to_string(),
            source: error.source().map(Box::new),
            meta: error.meta().map(Box::new),
        }
    }
}

impl From<Error> for JsonApiError {
    /// Projects a drained failure onto the standard's error object, at the boundary that serialises
    /// it. `id` and `links` have no framework-side source; a consumer may add them.
    fn from(error: Error) -> Self {
        JsonApiError {
            id: None,
            links: None,
            status: Some(error.status),
            code: Some(error.code.into_owned()),
            title: Some(error.title.into_owned()),
            detail: Some(error.detail),
            source: error.source.map(|source| *source),
            meta: error.meta.map(|meta| *meta),
        }
    }
}
