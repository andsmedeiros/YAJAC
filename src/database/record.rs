use super::{
    attributes::{Attributes, Identifier},
    error::Error,
    relationships::Relationships,
    schema::TableSchema,
};
use crate::database::attributes::ForeignKeys;
use crate::json_api::identifier::Identifier as JsonApiIdentifier;

#[derive(Debug, Clone)]
pub struct Record<'sch> {
    pub schema: &'sch TableSchema<'sch>,
    pub id: Identifier,
    pub attributes: Attributes,
    pub foreign_keys: ForeignKeys<'sch>,
    pub relationships: Relationships<'sch>,
}

impl<'sch> Record<'sch> {
    pub fn new(
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
        attributes: Attributes,
        foreign_keys: ForeignKeys<'sch>,
    ) -> Self {
        Record {
            schema,
            id,
            attributes,
            foreign_keys,
            relationships: Relationships::new(),
        }
    }

    pub fn kind(&self) -> &'sch str {
        self.schema.name
    }

    pub fn identifier(&self) -> Result<JsonApiIdentifier, Error> {
        let kind = self.kind().to_string();
        let id = match &self.id {
            Identifier::Integer(value) => value.to_string(),
            Identifier::Text(value) => value.clone(),
        };

        Ok(JsonApiIdentifier::Existing { kind, id })
    }
}

#[derive(Debug, Clone)]
pub struct NewRecord<'sch> {
    pub schema: &'sch TableSchema<'sch>,
    pub attributes: Attributes,
    pub relationships: Relationships<'sch>,
}

impl<'sch> NewRecord<'sch> {
    pub fn new(schema: &'sch TableSchema<'sch>) -> Self {
        NewRecord {
            schema,
            attributes: Attributes::new(),
            relationships: Relationships::new(),
        }
    }
}
