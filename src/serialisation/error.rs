use crate::database::error::Error as DatabaseError;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone)]
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
