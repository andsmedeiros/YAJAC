use crate::database::error::Error as DatabaseError;
use crate::http_wrappers::StatusCode;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    DocumentSerialisationError {
        message: String,
    },
    /// Link generation could not produce a valid URI — an unresolved dynamic segment or a path that
    /// fails to parse. Always internal: the router owns every template and resolver, so a failure
    /// here is a framework fault, not a client one.
    LinkGenerationError {
        message: String,
    },
}

impl Error {
    /// Maps the error to the HTTP status its source mandates. Both failures are broken server-side
    /// invariants: the framework owns the document and every link template it renders.
    pub fn status(&self) -> StatusCode {
        match self {
            Error::DocumentSerialisationError { .. } | Error::LinkGenerationError { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Stable, machine-readable code identifying the failure, surfaced as the JSON:API error `code`.
    pub fn code(&self) -> &'static str {
        match self {
            Error::DocumentSerialisationError { .. } => "DocumentSerialisationError",
            Error::LinkGenerationError { .. } => "LinkGenerationFailed",
        }
    }

    /// Stable, human-readable summary of the failure, surfaced as the JSON:API error `title`. Unlike
    /// `Display`, it carries no per-occurrence detail.
    pub fn title(&self) -> &'static str {
        match self {
            Error::DocumentSerialisationError { .. } => "The response document could not be built",
            Error::LinkGenerationError { .. } => "A link could not be generated",
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::DocumentSerialisationError { message } => {
                write!(f, "Failed to serialise document: {}", message)
            }
            Error::LinkGenerationError { message } => {
                write!(f, "Failed to generate a link: {}", message)
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<DatabaseError> for Error {
    fn from(error: DatabaseError) -> Self {
        Error::DocumentSerialisationError {
            message: error.to_string(),
        }
    }
}
