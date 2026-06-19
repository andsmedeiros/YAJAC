use crate::database::attributes::Identifier;
use super::{
    QueryParameters,
    attributes::{Attribute, Attributes},
    error::Error,
    schema::TableSchema,
};

pub type Bindings = Vec<Attribute>;

pub trait QueryBuilder<'a> {
    fn new(schema: &'a TableSchema) -> Self;
    fn query(&self, parameters: &QueryParameters) -> Result<(String, Bindings), Error>;
    fn find(&self, id: Identifier, parameters: &QueryParameters) -> Result<(String, Bindings), Error>;
    fn insert(
        &self,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error>;
    fn update(
        &self,
        id: Identifier,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error>;
    fn delete(&self, id: Identifier) -> (String, Bindings);
}
