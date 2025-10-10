use super::{
    attributes::Attributes,
    relationships::Relationships,
    schema::TableSchema,
};

#[derive(Debug, Clone)]
pub struct Record {
    pub schema: &'static TableSchema,
    pub attributes: Attributes,
    pub relationships: Relationships<'static>
}

impl Record {
    pub fn new(schema: &'static TableSchema) -> Self {
        Record {
            schema,
            attributes: Attributes::new(),
            relationships: Relationships::new()
        }
    }
}