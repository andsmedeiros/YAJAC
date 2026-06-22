use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
#[serde(untagged, from = "IdentifierFields")]
pub enum Identifier {
    New {
        #[serde(rename = "type")]
        kind: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        lid: Option<String>,
    },
    Existing {
        #[serde(rename = "type")]
        kind: String,
        id: String,
    },
}

#[derive(Deserialize)]
struct IdentifierFields {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    lid: Option<String>,
}

impl From<IdentifierFields> for Identifier {
    fn from(fields: IdentifierFields) -> Self {
        match fields.id {
            Some(id) => Identifier::Existing {
                kind: fields.kind,
                id,
            },
            None => Identifier::New {
                kind: fields.kind,
                lid: fields.lid,
            },
        }
    }
}
