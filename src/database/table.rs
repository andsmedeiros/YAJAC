use super::{
    QueryParameters,
    attributes::Attributes,
    connection::Connection as ConnectionInterface,
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    schema::TableSchema,
};
use std::sync::{Arc, Mutex};

pub trait Table<Connection : ConnectionInterface, QueryBuilder : QueryBuilderInterface> {
    fn schema(&self) -> &TableSchema;
    fn connection(&self) -> Result<&mut Connection, Error>;

    fn new(table_schema: &TableSchema, connection: Arc<Mutex<Connection>>) -> Self;

    fn query(&self, parameters: &QueryParameters) -> Result<Vec<Attributes>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).query(parameters)?;
        self.connection()?.query(query, bindings)
    }

    fn first(&self, parameters: &QueryParameters) -> Result<Option<Attributes>, Error> {
        let rows = self.query(parameters)?;
        Ok(rows.into_iter().next())
    }

    fn find(&self, id: i32, parameters: &QueryParameters) -> Result<Attributes, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).find(id, parameters)?;

        let rows = self.connection()?.query(query, bindings)?;
        let row = rows.into_iter().next().ok_or_else(|| Error::RecordNotFound)?;

        Ok(row)
    }

    fn insert(&self, attributes: Attributes, parameters: &QueryParameters) -> Result<Attributes, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema())
            .insert(attributes, parameters)?;
        let rows = self.connection()?.query(query, bindings)?;
        let row = rows.into_iter().next().ok_or_else(|| Error::RecordNotFound)?;

        Ok(row)
    }

    fn update(&self, id: i32, attributes: Attributes, parameters: &QueryParameters) -> Result<Attributes, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema())
            .update(id, attributes, parameters)?;
        let rows = self.connection()?.query(query, bindings)?;
        let row = rows.into_iter().next().ok_or_else(|| Error::RecordNotFound)?;

        Ok(row)
    }

    fn delete(&self, id: i32) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete(id);
        self.connection()?.execute(query, bindings)
    }

}