use serde::{Serialize, Deserialize};
use serde_json::Value;
use std::collections::HashMap;
use crate::{
    http_wrappers::Uri,
    spec::{
        identifier::Identifier,
        relationship::Relationship
    }
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Links {
    #[serde(rename="self")]
    pub this: Uri
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    #[serde(flatten)]
    pub identifier: Identifier,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<HashMap<String, Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationships: Option<HashMap<String, Relationship>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, Value>>,
}
