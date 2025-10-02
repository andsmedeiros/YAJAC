use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
    string::FromUtf8Error,
};

#[derive(Debug, Clone)]
pub enum Error {
    ParseFailure {
        parameter: String,
        message: String,
    },
    InvalidEncodingFailure,
    InconsistentSchema {
        schema: String,
        attribute: String,
        message: String
    },
    SchemaValidationFailure {
        schema: String,
        attribute: String,
        message: String,
    },
    InvalidAttributeSet,
    InvalidAttribute {
        attribute: String,
        kind: String,
        message: String
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
        message: String
    },
    RecordNotFound
}

impl From<FromUtf8Error> for Error {
    fn from(_: FromUtf8Error) -> Self {
        Error::InvalidEncodingFailure
    }
}

impl Display for Error {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        use Error::*;

        match self {
            ParseFailure { parameter, message } =>
                write!(fmt, "Failed to parse query parameter '{}': {}", parameter, message),
            InvalidEncodingFailure =>
                write!(fmt, "A parameter has an invalid encoding"),
            InconsistentSchema { schema, attribute, message } =>
                write!(fmt, "Schema '{}' is inconsistent for attribute '{}': {}", schema, attribute, message),
            SchemaValidationFailure { schema, attribute, message } =>
                write!(fmt, "Invalid attribute '{}' for schema '{}': {}", attribute, schema, message),
            InvalidAttributeSet =>
                write!(fmt, "The provided attributes are in an unexpected format."),
            InvalidAttribute { attribute, kind, message } =>
                write!(fmt, "Attribute '{}' is an invalid {}: {}", attribute, kind, message),
            InvalidAttributeConversion { kind } =>
                write!(fmt, "Cannot convert attribute to {}",  kind),
            InvalidOperation { schema, operation, message } =>
                write!(fmt, "Operation '{}' is invalid for schema '{}': {}", operation, schema, message),
            DatabaseFailure { message } =>
                write!(fmt, "Failed to execute query: {}", message),
            RecordNotFound =>
                write!(fmt, "Record not found"),
        }
    }
}

impl StdError for Error {}