use super::{
    QueryParameters,
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    record::Record,
    schema::TableSchema,
};
use std::sync::{Arc, Mutex, MutexGuard};

pub trait Table<'a, Connection : ConnectionInterface, QueryBuilder : QueryBuilderInterface<'a>> {
    fn new(table_schema: &'a TableSchema, connection: Arc<Mutex<Connection>>) -> Self;

    fn schema(&self) -> &'a TableSchema;

    fn connection(&self) -> Result<MutexGuard<Connection>, Error>;


    fn query(&self, parameters: &QueryParameters) -> Result<Vec<Record<'a>>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).query(parameters)?;
        self.run_fetch(query, bindings)
    }

    fn first(&self, parameters: &QueryParameters) -> Result<Option<Record<'a>>, Error> {
        self.query(parameters)
            .map(|rows|
                rows
                .into_iter()
                .next()
            )
    }

    fn find(&self, id: i32, parameters: &QueryParameters) -> Result<Record<'a>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).find(id, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn insert(&self, attributes: Attributes, parameters: &QueryParameters) -> Result<Record<'a>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema())
            .insert(attributes, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn update(&self, id: i32, attributes: Attributes, parameters: &QueryParameters) -> Result<Record<'a>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema())
            .update(id, attributes, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn delete(&self, id: i32) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete(id);
        self.connection()?.execute(query, bindings)
    }

    fn run_fetch(&self, query: String, bindings: Vec<Attribute>) -> Result<Vec<Record<'a>>, Error> {
        self.connection()?
            .query(query, bindings, self.schema())
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
    
    fn run_fetch_single(&self, query: String, bindings: Vec<Attribute>) -> Result<Record<'a>, Error> {
        self.run_fetch(query, bindings)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::RecordNotFound)
    }
}