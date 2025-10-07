use std::{
    convert::{AsMut, AsRef},
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
    str::FromStr,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde::de::{Error, Visitor};

#[derive(Debug, Clone)]
pub struct StatusCode(http::StatusCode);

impl StatusCode {
    pub const CONTINUE: StatusCode = StatusCode(http::StatusCode::CONTINUE);
    pub const SWITCHING_PROTOCOLS: StatusCode = StatusCode(http::StatusCode::SWITCHING_PROTOCOLS);
    pub const PROCESSING: StatusCode = StatusCode(http::StatusCode::PROCESSING);
    pub const OK: StatusCode = StatusCode(http::StatusCode::OK);
    pub const CREATED: StatusCode = StatusCode(http::StatusCode::CREATED);
    pub const ACCEPTED: StatusCode = StatusCode(http::StatusCode::ACCEPTED);
    pub const NON_AUTHORITATIVE_INFORMATION: StatusCode = StatusCode(http::StatusCode::NON_AUTHORITATIVE_INFORMATION);
    pub const NO_CONTENT: StatusCode = StatusCode(http::StatusCode::NO_CONTENT);
    pub const RESET_CONTENT: StatusCode = StatusCode(http::StatusCode::RESET_CONTENT);
    pub const PARTIAL_CONTENT: StatusCode = StatusCode(http::StatusCode::PARTIAL_CONTENT);
    pub const MULTI_STATUS: StatusCode = StatusCode(http::StatusCode::MULTI_STATUS);
    pub const ALREADY_REPORTED: StatusCode = StatusCode(http::StatusCode::ALREADY_REPORTED);
    pub const IM_USED: StatusCode = StatusCode(http::StatusCode::IM_USED);
    pub const MULTIPLE_CHOICES: StatusCode = StatusCode(http::StatusCode::MULTIPLE_CHOICES);
    pub const MOVED_PERMANENTLY: StatusCode = StatusCode(http::StatusCode::MOVED_PERMANENTLY);
    pub const FOUND: StatusCode = StatusCode(http::StatusCode::FOUND);
    pub const SEE_OTHER: StatusCode = StatusCode(http::StatusCode::SEE_OTHER);
    pub const NOT_MODIFIED: StatusCode = StatusCode(http::StatusCode::NOT_MODIFIED);
    pub const USE_PROXY: StatusCode = StatusCode(http::StatusCode::USE_PROXY);
    pub const TEMPORARY_REDIRECT: StatusCode = StatusCode(http::StatusCode::TEMPORARY_REDIRECT);
    pub const PERMANENT_REDIRECT: StatusCode = StatusCode(http::StatusCode::PERMANENT_REDIRECT);
    pub const BAD_REQUEST: StatusCode = StatusCode(http::StatusCode::BAD_REQUEST);
    pub const UNAUTHORIZED: StatusCode = StatusCode(http::StatusCode::UNAUTHORIZED);
    pub const PAYMENT_REQUIRED: StatusCode = StatusCode(http::StatusCode::PAYMENT_REQUIRED);
    pub const FORBIDDEN: StatusCode = StatusCode(http::StatusCode::FORBIDDEN);
    pub const NOT_FOUND: StatusCode = StatusCode(http::StatusCode::NOT_FOUND);
    pub const METHOD_NOT_ALLOWED: StatusCode = StatusCode(http::StatusCode::METHOD_NOT_ALLOWED);
    pub const NOT_ACCEPTABLE: StatusCode = StatusCode(http::StatusCode::NOT_ACCEPTABLE);
    pub const PROXY_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(http::StatusCode::PROXY_AUTHENTICATION_REQUIRED);
    pub const REQUEST_TIMEOUT: StatusCode = StatusCode(http::StatusCode::REQUEST_TIMEOUT);
    pub const CONFLICT: StatusCode = StatusCode(http::StatusCode::CONFLICT);
    pub const GONE: StatusCode = StatusCode(http::StatusCode::GONE);
    pub const LENGTH_REQUIRED: StatusCode = StatusCode(http::StatusCode::LENGTH_REQUIRED);
    pub const PRECONDITION_FAILED: StatusCode = StatusCode(http::StatusCode::PRECONDITION_FAILED);
    pub const PAYLOAD_TOO_LARGE: StatusCode = StatusCode(http::StatusCode::PAYLOAD_TOO_LARGE);
    pub const URI_TOO_LONG: StatusCode = StatusCode(http::StatusCode::URI_TOO_LONG);
    pub const UNSUPPORTED_MEDIA_TYPE: StatusCode = StatusCode(http::StatusCode::UNSUPPORTED_MEDIA_TYPE);
    pub const RANGE_NOT_SATISFIABLE: StatusCode = StatusCode(http::StatusCode::RANGE_NOT_SATISFIABLE);
    pub const EXPECTATION_FAILED: StatusCode = StatusCode(http::StatusCode::EXPECTATION_FAILED);
    pub const IM_A_TEAPOT: StatusCode = StatusCode(http::StatusCode::IM_A_TEAPOT);
    pub const MISDIRECTED_REQUEST: StatusCode = StatusCode(http::StatusCode::MISDIRECTED_REQUEST);
    pub const UNPROCESSABLE_ENTITY: StatusCode = StatusCode(http::StatusCode::UNPROCESSABLE_ENTITY);
    pub const LOCKED: StatusCode = StatusCode(http::StatusCode::LOCKED);
    pub const FAILED_DEPENDENCY: StatusCode = StatusCode(http::StatusCode::FAILED_DEPENDENCY);
    pub const TOO_EARLY: StatusCode = StatusCode(http::StatusCode::TOO_EARLY);
    pub const UPGRADE_REQUIRED: StatusCode = StatusCode(http::StatusCode::UPGRADE_REQUIRED);
    pub const PRECONDITION_REQUIRED: StatusCode = StatusCode(http::StatusCode::PRECONDITION_REQUIRED);
    pub const TOO_MANY_REQUESTS: StatusCode = StatusCode(http::StatusCode::TOO_MANY_REQUESTS);
    pub const REQUEST_HEADER_FIELDS_TOO_LARGE: StatusCode = StatusCode(http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
    pub const UNAVAILABLE_FOR_LEGAL_REASONS: StatusCode = StatusCode(http::StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);
    pub const INTERNAL_SERVER_ERROR: StatusCode = StatusCode(http::StatusCode::INTERNAL_SERVER_ERROR);
    pub const NOT_IMPLEMENTED: StatusCode = StatusCode(http::StatusCode::NOT_IMPLEMENTED);
    pub const BAD_GATEWAY: StatusCode = StatusCode(http::StatusCode::BAD_GATEWAY);
    pub const SERVICE_UNAVAILABLE: StatusCode = StatusCode(http::StatusCode::SERVICE_UNAVAILABLE);
    pub const GATEWAY_TIMEOUT: StatusCode = StatusCode(http::StatusCode::GATEWAY_TIMEOUT);
    pub const HTTP_VERSION_NOT_SUPPORTED: StatusCode = StatusCode(http::StatusCode::HTTP_VERSION_NOT_SUPPORTED);
    pub const VARIANT_ALSO_NEGOTIATES: StatusCode = StatusCode(http::StatusCode::VARIANT_ALSO_NEGOTIATES);
    pub const INSUFFICIENT_STORAGE: StatusCode = StatusCode(http::StatusCode::INSUFFICIENT_STORAGE);
    pub const LOOP_DETECTED: StatusCode = StatusCode(http::StatusCode::LOOP_DETECTED);
    pub const NOT_EXTENDED: StatusCode = StatusCode(http::StatusCode::NOT_EXTENDED);
    pub const NETWORK_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(http::StatusCode::NETWORK_AUTHENTICATION_REQUIRED);
}

impl Display for StatusCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.to_string())
    }
}

impl Deref for StatusCode {
    type Target = http::StatusCode;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StatusCode {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<http::StatusCode> for StatusCode {
    fn as_ref(&self) -> &http::StatusCode {
        &self.0
    }
}

impl AsMut<http::StatusCode> for StatusCode {
    fn as_mut(&mut self) -> &mut http::StatusCode {
        &mut self.0
    }
}

impl From<http::StatusCode> for StatusCode {
    fn from(status: http::StatusCode) -> Self {
        StatusCode(status)
    }
}

impl From<StatusCode> for http::StatusCode {
    fn from(status: StatusCode) -> Self {
        status.0
    }
}

impl FromStr for StatusCode {
    type Err = <http::StatusCode as FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<http::StatusCode>().map(|u| u.into())
    }
}

impl Serialize for StatusCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(self.0.as_u16())
    }
}

struct StatusCodeVisitor;
impl<'de> Visitor<'de> for StatusCodeVisitor {
    type Value = http::StatusCode;
    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("a u16 containing valid HTTP status code")
    }
    fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
    where
        E: Error,
    {
        http::StatusCode::from_u16(v).map_err(Error::custom)
    }
}

impl<'de> Deserialize<'de> for StatusCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(StatusCode(deserializer.deserialize_u16(StatusCodeVisitor)?))
    }
}