use super::{
    attributes::{Attribute, Attributes},
    error::Error,
};

pub trait Connection {
    fn query(&mut self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Attributes>, Error>;
    fn execute(&mut self, query: String, bindings: Vec<Attribute>) -> Result<(), Error>;
}