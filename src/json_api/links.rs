use crate::http_wrappers::Uri;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinkLang {
    Single(Uri),
    Multiple(Vec<Uri>),
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
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Link {
    Uri(Uri),
    Object(Box<LinkObject>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_uri_link_serializes_as_string() {
        let link = Link::Uri("/articles/1".parse().unwrap());

        assert_eq!(serde_json::to_value(&link).unwrap(), json!("/articles/1"));
    }

    #[test]
    fn test_object_link_serializes_as_object_omitting_empty_fields() {
        let link = Link::Object(Box::new(LinkObject {
            href: "/articles/1".parse().unwrap(),
            rel: None,
            described_by: None,
            title: Some("Article".to_string()),
            kind: Some("text/html".to_string()),
            href_lang: None,
            meta: None,
        }));

        assert_eq!(
            serde_json::to_value(&link).unwrap(),
            json!({ "href": "/articles/1", "title": "Article", "type": "text/html" })
        );
    }
}
