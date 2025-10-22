use super::{
    attributes::Attributes,
    error::Error,
    relationships::Relationships,
    schema::TableSchema,
};
use crate::json_api::identifier::Identifier;

#[derive(Debug, Clone)]
pub struct Record<'a> {
    pub schema: &'a TableSchema,
    pub attributes: Attributes,
    pub relationships: Relationships<'a>
}

impl<'a> Record<'a> {
    pub fn new(schema: &'a TableSchema) -> Self {
        Record {
            schema,
            attributes: Attributes::new(),
            relationships: Relationships::new()
        }
    }

    pub fn kind(&self) -> &'a str {
        self.schema.name
    }

    pub fn identifier(&self) -> Result<Identifier, Error> {
        // TODO: Consider implementing other id types specified by the schema
        let identifier = match self.attributes.get("id") {
            Some(id) => Identifier::Existing {
                kind: self.kind().to_string(),
                id: id.as_i64()?.to_string()
            },
            None => Identifier::New {
                kind: self.kind().to_string(),
                lid: self.attributes.get("lid")
                    .map(|lid| -> Result<_, Error> { Ok(lid.as_i64()?.to_string()) })
                    .transpose()?
            }
        };

        Ok(identifier)
    }
}