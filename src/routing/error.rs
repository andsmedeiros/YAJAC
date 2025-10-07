use crate::{
    database::error::Error as DatabaseError,
    http_wrappers::StatusCode,
    json_api::error::{Source},
    parameters::error::Error as ParameterError,
};
use http::{Error as HttpError};
use itertools::Itertools;
use serde::{Serialize, Deserialize, Serializer};
use serde_json::{Error as JsonError, Value, json};
use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
};

#[derive(Debug, Serialize, Deserialize)]
pub enum ApiErrorKind {
    ModelValidationFailed
}
//
// #[derive(Debug, Serialize, Deserialize)]
// pub struct FieldValidationError {
//     pub attribute: String,
//     pub description: String,
//     pub details: Value
// }
//
// #[derive(Debug, Serialize, Deserialize)]
// pub struct ModelValidationError {
//     pub kind: ApiErrorKind,
//     pub message: String,
//     pub model: String,
//     pub errors: Vec<FieldValidationError>
// }

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
        U: Into<String>
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

impl From<HttpError> for Error {
    fn from(error: HttpError) -> Self {
        Error::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "UnexpectedError",
            &error.to_string()
        )
    }
}

impl From<JsonError> for Error {
    fn from(error: JsonError) -> Self {
        Error::new(
            StatusCode::BAD_REQUEST,
            "JsonSerialisationError",
            &error.to_string()
        )
        .with_meta(json!({
            "kind": format!("{:?}", error.classify()),
            "line": error.line(),
            "column": error.column()
        }))
    }
}

// impl From<FieldValidationError> for Error {
//     fn from(error: FieldValidationError) -> Self {
//         Error::new(
//             StatusCode::BAD_REQUEST,
//             "FieldValidationError",
//             error.description
//         )
//             .with_source(Source::pointer_for_attribute(error.attribute))
//             .with_meta(error.details)
//     }
// }

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
                "The requested record could not be found."
            ),
            error @ DatabaseError::SchemaValidationFailure { .. } => Error::new(
                StatusCode::BAD_REQUEST,
                "SchemaValidationFailure",
                error.to_string()
            ),
            error @ DatabaseError::InvalidAttribute { .. } => Error::new(
                StatusCode::BAD_REQUEST,
                "InvalidAttribute",
                error.to_string()
            ),
            error @ DatabaseError::DatabaseFailure { .. } => Error::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DatabaseFailure",
                error.to_string()
            ),
            error => Error::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                &error.to_string()
            )
        }
    }
}

impl From<ParameterError> for Error {
    fn from(error: ParameterError) -> Self {
        match error {
            ParameterError::ParseParameterFailure { parameter, message } =>
                Error::new(
                    StatusCode::BAD_REQUEST,
                    "ParameterParseFailure",
                    message
                    // TODO: Proper source formatting
                )
                .with_source(Source::Pointer(parameter)),
            ParameterError::RequiredParameterMissing { parameter } =>
                Error::new(
                    StatusCode::BAD_REQUEST,
                    "RequiredParameterMissing",
                    format!("Required parameter '{}' was not provided", parameter),
                ),
            ParameterError::InvalidEncodingFailure =>
                Error::new(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "InvalidEncodingFailure",
                    "Encoding is not UTF-8 or is not valid"
                )
        }
    }
}

impl StdError for Error {}
//
// impl<T> From<T> for Error
// where
//     T: Into<Error>
// {
//     fn from(value: T) -> Self {
//         Error { content: vec![value.into()] }
//     }
// }
//
// impl<T> From<Vec<T>> for Error
// where
//     T: Into<Error>
// {
//     fn from(value: Vec<T>) -> Self {
//         Error { content: value.into_iter().map_into().collect() }
//     }
// }

// impl From<ModelValidationError> for Error {
//     fn from(error: ModelValidationError) -> Self {
//         Error::from(error.errors)
//     }
// }