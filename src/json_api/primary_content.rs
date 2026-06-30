use crate::json_api::{error::Error, resource::Resource};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrimaryContent {
    Record { data: Box<Resource> },
    Collection { data: Vec<Resource> },
    Errors { errors: Vec<Error> },
}

impl From<Resource> for PrimaryContent {
    fn from(resource: Resource) -> Self {
        PrimaryContent::Record {
            data: Box::new(resource),
        }
    }
}

impl From<Vec<Resource>> for PrimaryContent {
    fn from(collection: Vec<Resource>) -> Self {
        PrimaryContent::Collection { data: collection }
    }
}

impl<const N: usize> From<[Resource; N]> for PrimaryContent {
    fn from(collection: [Resource; N]) -> Self {
        PrimaryContent::Collection {
            data: collection.into(),
        }
    }
}

impl From<Vec<Error>> for PrimaryContent {
    fn from(errors: Vec<Error>) -> Self {
        PrimaryContent::Errors { errors }
    }
}

impl<const N: usize> From<[Error; N]> for PrimaryContent {
    fn from(errors: [Error; N]) -> Self {
        PrimaryContent::Errors {
            errors: errors.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_api::identifier::Identifier;
    use serde_json::json;

    fn make_resource(id: &str) -> Resource {
        Resource {
            identifier: Identifier::Existing {
                kind: "articles".to_string(),
                id: id.to_string(),
            },
            attributes: None,
            relationships: None,
            links: None,
            meta: None,
        }
    }

    #[test]
    fn test_record_serializes_under_data() {
        let content = PrimaryContent::from(make_resource("1"));

        assert_eq!(
            serde_json::to_value(&content).unwrap(),
            json!({ "data": { "type": "articles", "id": "1" } })
        );
    }

    #[test]
    fn test_collection_serializes_under_data() {
        let content = PrimaryContent::from(vec![make_resource("1"), make_resource("2")]);

        assert_eq!(
            serde_json::to_value(&content).unwrap(),
            json!({ "data": [
                { "type": "articles", "id": "1" },
                { "type": "articles", "id": "2" }
            ] })
        );
    }

    #[test]
    fn test_errors_serialize_under_errors() {
        let content = PrimaryContent::Errors {
            errors: vec![Error {
                code: Some("Boom".to_string()),
                title: Some("It broke".to_string()),
                ..Error::default()
            }],
        };

        assert_eq!(
            serde_json::to_value(&content).unwrap(),
            json!({ "errors": [
                { "status": "500", "code": "Boom", "title": "It broke" }
            ] })
        );
    }
}
