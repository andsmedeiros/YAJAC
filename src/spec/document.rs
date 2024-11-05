use serde::{Serialize, Deserialize};
use serde_json::Value;
use crate::{
    spec:: {
        links::Link,
        primary_content::PrimaryContent,
    },
    http_wrappers::Uri,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<Vec<Uri>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<Uri>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Links {
    #[serde(rename = "self")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub this: Option<Link>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Link>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub described_by: Option<Link>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Document {
    #[serde(flatten)]
    pub content: PrimaryContent,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonapi: Option<ImplementationInfo>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Vec<Value>>,
}