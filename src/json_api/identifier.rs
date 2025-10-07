use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
#[serde(untagged)]
pub enum Identifier {
    New {
        #[serde(rename = "type")]
        kind: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        lid: Option<String>
    },
    Existing {
        #[serde(rename = "type")]
        kind: String,
        id: String
    }
}