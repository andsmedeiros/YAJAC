use super::{
    attributes::{Attribute, Attributes},
    error::Error,
    schema::TableSchema,
};

pub trait Connection {
    fn query(
        &self,
        query: String,
        bindings: Vec<Attribute>,
        table_schema: &TableSchema,
    ) -> Result<Vec<Attributes>, Error>;

    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<(), Error>;

    /// Runs `operation` inside a database transaction, committing on `Ok` and rolling back on
    /// `Err` or panic.
    fn transaction<R>(&self, operation: impl FnOnce() -> Result<R, Error>) -> Result<R, Error>;
}
