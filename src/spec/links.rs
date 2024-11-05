use serde::{Serialize, Deserialize};
use serde_json::Value;
use crate::http_wrappers::Uri;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinkLang {
    Single(Uri),
    Multiple(Vec<Uri>)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct LinkObject {
    pub href: Uri,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub described_by: Option<Uri>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub href_lang: Option<LinkLang>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Link {
    Uri(Uri),
    Object(LinkObject),
}