use std::fmt::{Display, Formatter};
use crate::database::error::Error as DatabaseError;

#[derive(Debug, Clone)]
pub enum Error {
    DocumentSerialisationError { message: String },
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::DocumentSerialisationError { message } =>
                write!(f, "Failed to serialise document: {}", message),
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