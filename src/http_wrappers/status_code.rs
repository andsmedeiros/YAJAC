use std::fmt::Formatter;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde::de::{Error, Visitor};

#[derive(Debug, Clone)]
pub struct StatusCode(http::StatusCode);

impl From<http::StatusCode> for StatusCode {
    fn from(status: http::StatusCode) -> Self {
        StatusCode(status)
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