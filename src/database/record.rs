use super::{
    attributes::Attributes,
    schema::TableSchema,
};

#[derive(Debug, Clone)]
pub struct Record {
    pub schema: &'static TableSchema,
    pub attributes: Attributes,
}