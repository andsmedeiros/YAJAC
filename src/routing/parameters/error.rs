use std::{
    error::Error as StdError,
    fmt::{Display, Formatter},
    string::FromUtf8Error,
};

#[derive(Debug, Clone)]
pub enum Error {
    InvalidEncodingFailure,
    ParseParameterFailure {
        parameter: String,
        message: String,
    },
    RequiredParameterMissing {
        parameter: String
    },
}

impl From<FromUtf8Error> for Error {
    fn from(_: FromUtf8Error) -> Self {
        Error::InvalidEncodingFailure
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use Error::*;

        match self {
            InvalidEncodingFailure =>
                write!(f, "A provided parameter has an invalid encoding"),
            ParseParameterFailure { parameter, message } =>
                write!(f, "Failed to parse parameter '{}': {}", parameter, message),
            RequiredParameterMissing { parameter } =>
                write!(f, "Required parameter '{}' was not provided", parameter),
        }
    }
}

impl StdError for Error {}