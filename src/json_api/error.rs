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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
    Pointer(String),
    Parameter(String),
    Header(String),
}

impl Source {
    pub fn pointer_for_primary_data() -> Source {
        Source::Pointer("/data".to_string())
    }

    pub fn pointer_for_attribute<T>(attribute: T) -> Source
    where
        T: Display,
    {
        Source::Pointer(format!("/data/attributes/{}", attribute))
    }
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
