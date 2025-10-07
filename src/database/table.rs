use crate::database::{
    attributes::Record,
    error::Error,
};
use crate::routing::parameters::QueryParameters;

pub trait Table {
    fn query(&self, parameters: &QueryParameters) -> Result<Vec<Record>, Error>;
    fn first(&self, parameters: &QueryParameters) -> Result<Option<Record>, Error>;
    fn find(&self, id: i32, parameters: &QueryParameters) -> Result<Record, Error>;
    fn insert(&self, attributes: Record, parameters: &QueryParameters) -> Result<Record, Error>;
    fn update(&self, id: i32, attributes: Record, parameters: &QueryParameters) -> Result<Record, Error>;
    fn delete(&self, id: i32) -> Result<(), Error>;
}