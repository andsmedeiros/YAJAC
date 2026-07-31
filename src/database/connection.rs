use super::{
    attributes::{Attribute, Attributes},
    error::Error,
    schema::Schema,
};

pub trait Connection {
    fn query<'sch>(
        &self,
        query: String,
        bindings: Vec<Attribute>,
        schema: &'sch Schema<'sch>,
    ) -> Result<Vec<Attributes<'sch>>, Error>;

    /// Runs a non-returning statement and reports how many rows it affected.
    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<usize, Error>;

    /// Runs `operation` inside a database transaction, committing on `Ok` and rolling back on
    /// `Err` or panic.
    fn transaction<R>(&self, operation: impl FnOnce() -> Result<R, Error>) -> Result<R, Error>;
}
