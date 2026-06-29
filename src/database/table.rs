use super::{
    attributes::{Attribute, Row},
    connection::Connection as ConnectionInterface,
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    query_parameters::QueryParameters,
    schema::TableSchema,
};
use crate::database::attributes::Identifier;

pub trait Table<
    'sch,
    'req,
    Connection: ConnectionInterface + 'req,
    QueryBuilder: QueryBuilderInterface<'sch>,
>
{
    fn new(table_schema: &'sch TableSchema<'sch>, connection: &'req Connection) -> Self;

    fn schema(&self) -> &'sch TableSchema<'sch>;

    fn connection(&self) -> &'req Connection;

    fn is_attribute(&self, name: &str) -> bool {
        self.schema().attribute(name).is_some()
    }

    fn is_foreign_key(&self, name: &str) -> bool {
        self.schema().foreign_key(name).is_some()
    }

    fn query(&self, parameters: &QueryParameters) -> Result<Vec<Row>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).query(parameters)?;
        self.run_fetch(query, bindings)
    }

    fn first(&self, parameters: &QueryParameters) -> Result<Option<Row>, Error> {
        self.query(parameters).map(|rows| rows.into_iter().next())
    }

    fn find(&self, id: Identifier, parameters: &QueryParameters) -> Result<Row, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).find(id, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn insert(&self, row: Row, parameters: &QueryParameters) -> Result<Row, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).insert(row, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn update(&self, id: Identifier, row: Row, parameters: &QueryParameters) -> Result<Row, Error> {
        self.require_columns(&row)?;
        let (query, bindings) = QueryBuilder::new(self.schema()).update(id, row, parameters)?;
        self.run_fetch_single(query, bindings)
    }

    fn update_batch(&self, row: Row, parameters: &QueryParameters) -> Result<Vec<Row>, Error> {
        self.require_columns(&row)?;
        let (query, bindings) = QueryBuilder::new(self.schema()).update_batch(row, parameters)?;
        self.run_fetch(query, bindings)
    }

    fn insert_batch(
        &self,
        rows: Vec<Row>,
        parameters: &QueryParameters,
    ) -> Result<Vec<Row>, Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let (query, bindings) = QueryBuilder::new(self.schema()).insert_batch(rows, parameters)?;
        self.run_fetch(query, bindings)
    }

    fn require_columns(&self, row: &Row) -> Result<(), Error> {
        if row.is_empty() {
            return Err(Error::InvalidOperation {
                schema: self.schema().name.to_string(),
                operation: "UPDATE".to_string(),
                message: "no attributes to update".to_string(),
            });
        }

        Ok(())
    }

    fn delete(&self, id: Identifier) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete(id);
        self.connection().execute(query, bindings)
    }

    fn delete_batch(&self, parameters: &QueryParameters) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete_batch(parameters)?;
        self.connection().execute(query, bindings)
    }

    fn run_fetch(&self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Row>, Error> {
        self.connection().query(query, bindings, self.schema())
    }

    fn run_fetch_single(&self, query: String, bindings: Vec<Attribute>) -> Result<Row, Error> {
        self.run_fetch(query, bindings)?
            .into_iter()
            .next()
            .ok_or(Error::RecordNotFound)
    }
}
