use super::{
    attributes::{Attribute, Attributes},
    error::Error,
    schema::Schema,
};

pub trait Connection {
    fn query(
        &self,
        query: String,
        bindings: Vec<Attribute>,
        schema: &Schema,
    ) -> Result<Vec<Attributes>, Error>;

    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<(), Error>;

    /// Runs `operation` inside a database transaction, committing on `Ok` and rolling back on
    /// `Err` or panic.
    fn transaction<R>(&self, operation: impl FnOnce() -> Result<R, Error>) -> Result<R, Error>;
}
