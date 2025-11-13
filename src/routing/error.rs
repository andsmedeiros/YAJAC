use crate::{
    core::error::Error as CoreError, database::error::Error as DatabaseError,
    http_wrappers::StatusCode, json_api::error::Source,
};
use http::Error as HttpError;
use serde::{Deserialize, Serialize};
use serde_json::{Error as JsonError, Value, json};
use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
};

#[derive(Debug, Clone, Serialize)]
pub struct RequiredParameterMissingError {
    pub parameter: String,
}

impl Display for RequiredParameterMissingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Required parameter '{}' was not provided",
            self.parameter
        )
    }
}

impl StdError for RequiredParameterMissingError {}

#[derive(Debug, Clone, Serialize)]
pub struct FailtToParseParameterError {
    pub parameter: String,
    pub message: String,
}

impl Display for FailtToParseParameterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Failed to parse parameter '{}': {}",
            self.parameter, self.message
        )
    }
}

impl StdError for FailtToParseParameterError {}

#[derive(Debug, Serialize, Deserialize)]
pub enum ApiErrorKind {
    ModelValidationFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    status: StatusCode,
    code: String,
    title: String,
    source: Option<Source>,
    detail: Option<String>,
    meta: Option<Value>,
}

impl Error {
    pub fn new<T, U>(status: StatusCode, code: T, title: U) -> Self
    where
        T: Into<String>,
        U: Into<String>,
    {
        Self {
            status,
            code: code.into(),
            title: title.into(),
            source: None,
            detail: None,
            meta: None,
        }
    }

    pub fn with_source(mut self, source: Source) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_meta(mut self, meta: Value) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn into_result<T>(self) -> Result<T, Error> {
        Err(self)
    }

    pub fn status_code(&self) -> StatusCode {
        self.status.clone()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<RequiredParameterMissingError> for Error {
    fn from(error: RequiredParameterMissingError) -> Self {
        Error::new(
            StatusCode::BAD_REQUEST,
            "RequiredParameterMissing",
            error.to_string(),
        )
    }
}

impl From<FailtToParseParameterError> for Error {
    fn from(error: FailtToParseParameterError) -> Self {
        Error::new(
            StatusCode::BAD_REQUEST,
            "FailedToParseParameterError",
            error.to_string(),
        )
    }
}

impl From<HttpError> for Error {
    fn from(error: HttpError) -> Self {
        Error::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "UnexpectedError",
            &error.to_string(),
        )
    }
}

impl From<JsonError> for Error {
    fn from(error: JsonError) -> Self {
        Error::new(
            StatusCode::BAD_REQUEST,
            "JsonSerialisationError",
            &error.to_string(),
        )
        .with_meta(json!({
            "kind": format!("{:?}", error.classify()),
            "line": error.line(),
            "column": error.column()
        }))
    }
}

impl From<Error> for Vec<Error> {
    fn from(error: Error) -> Self {
        vec![error].into()
    }
}

impl From<DatabaseError> for Error {
    fn from(error: DatabaseError) -> Self {
        match error {
            DatabaseError::RecordNotFound => Error::new(
                StatusCode::NOT_FOUND,
                "RecordNotFound",
                "The requested record could not be found.",
            ),
            error @ DatabaseError::SchemaValidationFailure { .. } => Error::new(
                StatusCode::BAD_REQUEST,
                "SchemaValidationFailure",
                error.to_string(),
            ),
            error @ DatabaseError::InvalidAttribute { .. } => Error::new(
                StatusCode::BAD_REQUEST,
                "InvalidAttribute",
                error.to_string(),
            ),
            error @ DatabaseError::DatabaseFailure { .. } => Error::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DatabaseFailure",
                error.to_string(),
            ),
            error => Error::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                error.to_string(),
            ),
        }
    }
}

impl From<CoreError> for Error {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::DocumentSerialisationError { .. } => Error::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DocumentSerialisationError",
                error.to_string(),
            ),
        }
    }
}

impl StdError for Error {}
