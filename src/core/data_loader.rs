use crate::database::{
    adapters::Adapter as AdapterInterface,
    record::Record,
    registry::Registry,
    schema::TableSchema,
    query_parameters::QueryParameters,
};
use std::{
    collections::HashMap,
    ptr,
    slice
};
use super::error::Error;

type RecordCache<'a> = HashMap<&'a str, HashMap<i32, Record<'a>>>;

struct DataLoader<'a, Adapter: AdapterInterface> {
    registry: &'a Registry<'a, Adapter>,
    cache: RecordCache<'a>,
}


fn collection_schema<'a>(collection: &'a [Record]) -> Result<&'a TableSchema, Error> {
    let mut collection = collection.iter();
    let first = collection.next()
        .ok_or_else(|| Error::DataLoadingError {
            message: "Attempted to load related data from an empty collection".to_string()
        })?;

    if collection.any(|record|
        ptr::from_ref(record.schema) != ptr::from_ref(first.schema)
    ) {
        return Err(Error::DataLoadingError {
            message: "Attempted to load related data from an heterogeneous collection; this is not supported.".to_string()
        });
    }

    Ok(first.schema)
}

impl<'a, Adapter: AdapterInterface> DataLoader<'a, Adapter> {
    pub fn new(registry: &'a Registry<'a, Adapter>) -> Self {
        DataLoader { registry, cache: HashMap::new() }
    }

    pub fn load_for_record(&self, record: &Record, query_parameters: &QueryParameters)
                           -> Result<(), Error>
    {
        self.load_for_collection(slice::from_ref(record), query_parameters)
    }

    pub fn load_for_collection(&self, collection: &[Record], query_parameters: &QueryParameters)
        -> Result<(), Error>
    {
        if let Some(relationship_paths) = &query_parameters.include {
            for relationship_path in relationship_paths {
                self.load_relationship(relationship_path, collection, query_parameters)?;
            }
        }

        Ok(())
    }

    fn load_relationship(&self, relationship_path: &str, collection: &[Record], query_parameters: &QueryParameters)
        -> Result<(), Error>
    {
        let schema = collection_schema(collection)?;
        let (relationship, rest) = match relationship_path.split_once('.') {
            Some((relationship, rest)) => (relationship, Some(rest)),
            None => (relationship_path, None)
        };

        todo!("Implement proper loading")
    }
}

