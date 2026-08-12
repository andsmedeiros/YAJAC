use crate::{http_wrappers::StatusCode, json_api::links::Link};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{error::Error as StdError, fmt::Display};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Links {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<Link>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<Link>,
}

/// A reference to whatever in the request caused an error: a JSON Pointer into the request document,
/// a query parameter name, or a request header name. Exactly one, serialised under the member the
/// standard names it by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Pointer(String),
    Parameter(String),
    Header(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusCode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl Default for Error {
    fn default() -> Self {
        Error {
            id: None,
            links: None,
            status: Some(StatusCode::INTERNAL_SERVER_ERROR),
            code: Some("InternalServerFault".into()),
            title: Some("An unexpected error occurred. No more information is available.".into()),
            detail: None,
            source: None,
            meta: None,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl StdError for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pointer_source_names_the_pointer_member() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_value(Source::Pointer("/data/attributes/title".to_string()))?,
            json!({ "pointer": "/data/attributes/title" })
        );
        Ok(())
    }

    #[test]
    fn a_parameter_source_names_the_parameter_member() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_value(Source::Parameter("sort".to_string()))?,
            json!({ "parameter": "sort" })
        );
        Ok(())
    }

    #[test]
    fn a_header_source_names_the_header_member() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_value(Source::Header("Accept".to_string()))?,
            json!({ "header": "Accept" })
        );
        Ok(())
    }

    #[test]
    fn a_source_round_trips_through_its_member_name() -> Result<(), serde_json::Error> {
        let deserialised: Source = serde_json::from_value(json!({ "pointer": "/data" }))?;

        assert_eq!(deserialised, Source::Pointer("/data".to_string()));
        Ok(())
    }
}
