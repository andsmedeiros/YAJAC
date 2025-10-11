use super::{
    attributes::{Attribute, Attributes},
    error::Error,
    schema::TableSchema,
};

pub trait Connection {
    fn query(&mut self, query: String, bindings: Vec<Attribute>, table_schema: &TableSchema) 
        -> Result<Vec<Attributes>, Error>;
    fn execute(&mut self, query: String, bindings: Vec<Attribute>) -> Result<(), Error>;
}