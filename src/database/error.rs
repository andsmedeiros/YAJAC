use crate::http_wrappers::StatusCode;
use crate::utils::indexing::Error as IndexingError;
use std::string::FromUtf8Error;
use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
};

/// The class of a database constraint, abstracted across adapters so callers can react to a
/// violation portably (each adapter maps its native codes onto these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    ForeignKey,
    Unique,
    NotNull,
    Check,
    Other,
}

impl Display for ConstraintKind {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let label = match self {
            ConstraintKind::ForeignKey => "foreign key",
            ConstraintKind::Unique => "unique",
            ConstraintKind::NotNull => "not null",
            ConstraintKind::Check => "check",
            ConstraintKind::Other => "unknown",
        };
        write!(f, "{label}")
    }
}

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
    QueryValidationFailure {
        schema: String,
        attribute: String,
        message: String,
    },
    ResourceValidationFailure {
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
    InvalidAttributeAccess {
        schema: String,
        attribute: String,
    },
    InvalidRelationshipAccess {
        schema: String,
        relationship: String,
    },
    UnexpectedCollection {
        schema: String,
        relationship: String,
    },
    MismatchedRelationshipKind {
        schema: String,
        relationship: String,
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
        kind: ConstraintKind,
        message: String,
    },
    RecordNotFound,
    RelatedRecordNotFound,
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

impl Error {
    /// Maps the error to the HTTP status its source mandates: `4xx` for failures the client can
    /// correct, `5xx` for broken server-side invariants.
    pub fn status(&self) -> StatusCode {
        use Error::*;

        match self {
            ParseParameterFailure { .. }
            | InvalidEncodingFailure
            | QueryValidationFailure { .. } => StatusCode::BAD_REQUEST,
            ResourceValidationFailure { .. }
            | InvalidAttributeSet
            | InvalidAttribute { .. }
            | InvalidOperation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ConstraintViolation { .. } => StatusCode::CONFLICT,
            RecordNotFound | RelatedRecordNotFound => StatusCode::NOT_FOUND,
            InconsistentSchema { .. }
            | UnknownSchema { .. }
            | InvalidAttributeConversion { .. }
            | InvalidAttributeAccess { .. }
            | InvalidRelationshipAccess { .. }
            | UnexpectedCollection { .. }
            | MismatchedRelationshipKind { .. }
            | DatabaseFailure { .. }
            | DataLoadingError { .. }
            | UnloadedAttributeAccess { .. }
            | MissingRecordId { .. }
            | InconsistentCollection
            | InvalidIndexAccess => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable, machine-readable code identifying the failure, surfaced as the JSON:API error `code`.
    pub fn code(&self) -> &'static str {
        use Error::*;

        match self {
            ParseParameterFailure { .. } => "ParseParameterFailure",
            InvalidEncodingFailure => "InvalidEncodingFailure",
            InconsistentSchema { .. } => "InconsistentSchema",
            QueryValidationFailure { .. } => "QueryValidationFailure",
            ResourceValidationFailure { .. } => "ResourceValidationFailure",
            UnknownSchema { .. } => "UnknownSchema",
            InvalidAttributeSet => "InvalidAttributeSet",
            InvalidAttribute { .. } => "InvalidAttribute",
            InvalidAttributeConversion { .. } => "InvalidAttributeConversion",
            InvalidAttributeAccess { .. } => "InvalidAttributeAccess",
            InvalidRelationshipAccess { .. } => "InvalidRelationshipAccess",
            UnexpectedCollection { .. } => "UnexpectedCollection",
            MismatchedRelationshipKind { .. } => "MismatchedRelationshipKind",
            InvalidOperation { .. } => "InvalidOperation",
            DatabaseFailure { .. } => "DatabaseFailure",
            ConstraintViolation { .. } => "ConstraintViolation",
            RecordNotFound => "RecordNotFound",
            RelatedRecordNotFound => "RelatedRecordNotFound",
            DataLoadingError { .. } => "DataLoadingError",
            UnloadedAttributeAccess { .. } => "UnloadedAttributeAccess",
            MissingRecordId { .. } => "MissingRecordId",
            InconsistentCollection => "InconsistentCollection",
            InvalidIndexAccess => "InvalidIndexAccess",
        }
    }

    /// Stable, human-readable summary of the failure, surfaced as the JSON:API error `title`. Unlike
    /// `Display`, it carries no per-occurrence detail.
    pub fn title(&self) -> &'static str {
        use Error::*;

        match self {
            ParseParameterFailure { .. } => "Failed to parse a request parameter",
            InvalidEncodingFailure => "A request parameter has an invalid encoding",
            InconsistentSchema { .. } => "The schema is inconsistent",
            QueryValidationFailure { .. } => "A query parameter is invalid",
            ResourceValidationFailure { .. } => "The submitted resource is invalid",
            UnknownSchema { .. } => "The requested schema is unknown",
            InvalidAttributeSet => "The provided attributes are malformed",
            InvalidAttribute { .. } => "An attribute is invalid",
            InvalidAttributeConversion { .. } => "An attribute could not be converted",
            InvalidAttributeAccess { .. } => "An undeclared attribute was accessed",
            InvalidRelationshipAccess { .. } => "An undeclared relationship was accessed",
            UnexpectedCollection { .. } => "A single related record was expected",
            MismatchedRelationshipKind { .. } => {
                "The relationship kind is incompatible with the requested access"
            }
            InvalidOperation { .. } => "The operation is invalid",
            DatabaseFailure { .. } => "The database operation failed",
            ConstraintViolation { .. } => "A database constraint was violated",
            RecordNotFound => "The requested record was not found",
            RelatedRecordNotFound => "A related record was not found",
            DataLoadingError { .. } => "Failed to load related data",
            UnloadedAttributeAccess { .. } => "An unloaded attribute was accessed",
            MissingRecordId { .. } => "The record is missing an identifier",
            InconsistentCollection => "The collection is heterogeneous",
            InvalidIndexAccess => "An invalid index was accessed",
        }
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        let message = error.to_string();

        match error {
            rusqlite::Error::SqliteFailure(failure, _)
                if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                let kind = match failure.extended_code {
                    rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY => ConstraintKind::ForeignKey,
                    rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => ConstraintKind::Unique,
                    rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL => ConstraintKind::NotNull,
                    rusqlite::ffi::SQLITE_CONSTRAINT_CHECK => ConstraintKind::Check,
                    _ => ConstraintKind::Other,
                };
                Error::ConstraintViolation { kind, message }
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

impl From<IndexingError> for Error {
    /// Any indexing failure is a broken internal invariant, surfaced uniformly as an index access.
    fn from(_: IndexingError) -> Self {
        Error::InvalidIndexAccess
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
            QueryValidationFailure {
                schema,
                attribute,
                message,
            } => write!(
                f,
                "Invalid query parameter '{}' for schema '{}': {}",
                attribute, schema, message
            ),
            ResourceValidationFailure {
                schema,
                attribute,
                message,
            } => write!(
                f,
                "Invalid resource field '{}' for schema '{}': {}",
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
            InvalidAttributeAccess { schema, attribute } => write!(
                f,
                "Attempted to access attribute '{attribute}', which schema '{schema}' does not declare"
            ),
            InvalidRelationshipAccess {
                schema,
                relationship,
            } => write!(
                f,
                "Attempted to access relationship '{relationship}', which schema '{schema}' does not declare"
            ),
            UnexpectedCollection {
                schema,
                relationship,
            } => write!(
                f,
                "Expected a single related record for relationship '{relationship}' on schema '{schema}', but it resolved to a collection"
            ),
            MismatchedRelationshipKind {
                schema,
                relationship,
            } => write!(
                f,
                "Relationship '{relationship}' on schema '{schema}' has a kind incompatible with the requested access"
            ),
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
            ConstraintViolation { kind, message } => {
                write!(f, "Constraint violation ({kind}): {message}")
            }
            RecordNotFound => write!(f, "Record not found"),
            RelatedRecordNotFound => write!(f, "A referenced resource does not exist"),
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
