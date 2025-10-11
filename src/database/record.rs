use super::{
    attributes::Attributes,
    relationships::Relationships,
    schema::TableSchema,
};

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
}