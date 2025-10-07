use super::{
    attributes::{Attribute, Record},
    error::Error,
};

pub trait Connection {
    fn query(&self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Record>, Error>;
    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<(), Error>;
}