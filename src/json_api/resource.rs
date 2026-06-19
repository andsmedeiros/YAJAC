use crate::{
    http_wrappers::Uri,
    json_api::{identifier::Identifier, relationship::Relationship},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Links {
    #[serde(rename = "self")]
    pub this: Uri,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_resource_serializes_with_flattened_identifier_and_omits_empty_fields() {
        let resource = Resource {
            identifier: Identifier::Existing {
                kind: "articles".to_string(),
                id: "1".to_string(),
            },
            attributes: Some(HashMap::from([("title".to_string(), json!("Hello"))])),
            relationships: None,
            links: Some(Links {
                this: "/articles/1".parse().unwrap(),
            }),
            meta: None,
        };

        assert_eq!(
            serde_json::to_value(&resource).unwrap(),
            json!({
                "type": "articles",
                "id": "1",
                "attributes": { "title": "Hello" },
                "links": { "self": "/articles/1" }
            })
        );
    }
}
