use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{
    http_wrappers::Uri,
    spec::identifier::Identifier
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Linkage {
    Empty,
    ToOne(Identifier),
    ToMany(Vec<Identifier>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Links {
    #[serde(rename="self", skip_serializing_if="Option::is_none")]
    pub this: Option<Uri>,

    #[serde(skip_serializing_if="Option::is_none")]
    pub related: Option<Uri>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Linkage>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}