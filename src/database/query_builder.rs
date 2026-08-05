use super::{
    QueryParameters,
    attributes::{Attribute, Attributes},
    error::Error,
    schema::Schema,
};
use crate::database::attributes::Identifier;

pub type Bindings = Vec<Attribute>;

pub trait QueryBuilder<'sch> {
    fn new(schema: &'sch Schema<'sch>) -> Self;
    fn query(&self, parameters: &QueryParameters) -> Result<Option<(String, Bindings)>, Error>;
    fn find(
        &self,
        id: Identifier,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error>;
    fn insert(
        &self,
        attributes: Attributes<'sch>,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error>;
    fn update(
        &self,
        id: Identifier,
        attributes: Attributes<'sch>,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error>;
    fn update_batch(
        &self,
        attributes: Attributes<'sch>,
        parameters: &QueryParameters,
    ) -> Result<Option<(String, Bindings)>, Error>;
    fn insert_batch(
        &self,
        rows: Vec<Attributes<'sch>>,
        parameters: &QueryParameters,
    ) -> Result<(String, Bindings), Error>;
    fn delete(&self, id: Identifier) -> (String, Bindings);
    fn delete_batch(
        &self,
        parameters: &QueryParameters,
    ) -> Result<Option<(String, Bindings)>, Error>;
}
