use super::{
    QueryParameters,
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    record::Record,
    schema::TableSchema,
};
use std::sync::{Arc, Mutex};

pub trait Table<Connection : ConnectionInterface, QueryBuilder : QueryBuilderInterface> {
    fn schema(&self) -> &'static TableSchema;
    fn connection(&self) -> Result<&mut Connection, Error>;

    fn new(table_schema: &'static TableSchema, connection: Arc<Mutex<Connection>>) -> Self;

    fn query(&self, parameters: &QueryParameters) -> Result<Vec<Record>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).query(parameters)?;
        self.run_fetch(query, bindings)
    }

    fn first(&self, parameters: &QueryParameters) -> Result<Option<Record>, Error> {
        self.query(parameters)
            .map(|rows|
                rows
                .into_iter()
                .next()
            )
    }

    fn find(&self, id: i32, parameters: &QueryParameters) -> Result<Record, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).find(id, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn insert(&self, attributes: Attributes, parameters: &QueryParameters) -> Result<Record, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema())
            .insert(attributes, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn update(&self, id: i32, attributes: Attributes, parameters: &QueryParameters) -> Result<Record, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema())
            .update(id, attributes, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn delete(&self, id: i32) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete(id);
        self.connection()?.execute(query, bindings)
    }

    fn run_fetch(&self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Record>, Error> {
        self.connection()?
            .query(query, bindings)
            .map(|rows|
                rows
                    .into_iter()
                    .map(|attributes| Record {
                        attributes,
                        ..Record::new(self.schema())
                    })
                    .collect()
            )
    }
    
    fn run_fetch_single(&self, query: String, bindings: Vec<Attribute>) -> Result<Record, Error> {
        self.run_fetch(query, bindings)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::RecordNotFound)
    }
}