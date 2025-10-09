use super::{
    attributes::{Attribute, Attributes},
    error::Error,
};

pub trait Connection {
    fn query(&self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Attributes>, Error>;
    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<(), Error>;
}