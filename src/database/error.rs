use std::string::FromUtf8Error;
use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
};

#[derive(Debug, Clone)]
pub enum Error {
    ParseParameterFailure {
        parameter: String,
        message: String,
    },
    InvalidEncodingFailure,
    InconsistentSchema {
        schema: String,
        attribute: String,
        message: String,
    },
    SchemaValidationFailure {
        schema: String,
        attribute: String,
        message: String,
    },
    UnknownSchema {
        schema: String,
        message: String,
    },
    InvalidAttributeSet,
    InvalidAttribute {
        attribute: String,
        kind: String,
        message: String,
    },
    InvalidAttributeConversion {
        kind: String,
    },
    InvalidOperation {
        schema: String,
        operation: String,
        message: String,
    },
    DatabaseFailure {
        message: String,
    },
    ConstraintViolation {
        message: String,
    },
    RecordNotFound,
    DataLoadingError {
        message: String,
    },
    UnloadedAttributeAccess {
        schema: String,
        attribute: String,
    },
    MissingRecordId {
        schema: String,
    },
    InconsistentCollection,
    InvalidIndexAccess,
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        let message = error.to_string();

        match error {
            rusqlite::Error::SqliteFailure(failure, _)
                if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::ConstraintViolation { message }
            }
            _ => Error::DatabaseFailure { message },
        }
    }
}

impl From<FromUtf8Error> for Error {
    fn from(_: FromUtf8Error) -> Self {
        Error::InvalidEncodingFailure
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        use Error::*;

        match self {
            ParseParameterFailure { parameter, message } => {
                write!(f, "Failed to parse parameter '{}': {}", parameter, message)
            }
            InvalidEncodingFailure => write!(f, "A provided parameter has an invalid encoding"),
            InconsistentSchema {
                schema,
                attribute,
                message,
            } => write!(
                f,
                "Schema '{}' is inconsistent for attribute '{}': {}",
                schema, attribute, message
            ),
            SchemaValidationFailure {
                schema,
                attribute,
                message,
            } => write!(
                f,
                "Invalid attribute '{}' for schema '{}': {}",
                attribute, schema, message
            ),
            UnknownSchema { schema, message } => write!(
                f,
                "Attempted to load an unknown schema '{}': {}",
                schema, message
            ),
            InvalidAttributeSet => {
                write!(f, "The provided attributes are in an unexpected format.")
            }
            InvalidAttribute {
                attribute,
                kind,
                message,
            } => write!(
                f,
                "Attribute '{}' is an invalid {}: {}",
                attribute, kind, message
            ),
            InvalidAttributeConversion { kind } => {
                write!(f, "Cannot convert attribute to {}", kind)
            }
            InvalidOperation {
                schema,
                operation,
                message,
            } => write!(
                f,
                "Operation '{}' is invalid for schema '{}': {}",
                operation, schema, message
            ),
            DatabaseFailure { message } => write!(f, "Failed to execute query: {}", message),
            ConstraintViolation { message } => write!(f, "Constraint violation: {}", message),
            RecordNotFound => write!(f, "Record not found"),
            DataLoadingError { message } => write!(
                f,
                "Failed to load relationships for primary content: {}",
                message
            ),
            UnloadedAttributeAccess { schema, attribute } => write!(
                f,
                "Attempted to read attribute '{attribute}' of a record with schema '{schema}', but it was not loaded"
            ),
            MissingRecordId { schema } => write!(
                f,
                "Attempted to access ID of record with schema '{schema}', but it was not loaded"
            ),
            InconsistentCollection => write!(
                f,
                "Attempted to apply operation over an heterogeneous collection"
            ),
            InvalidIndexAccess => write!(f, "Attempted to extract an non-indexed value"),
        }
    }
}

impl StdError for Error {}
