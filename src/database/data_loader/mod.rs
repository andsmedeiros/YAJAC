mod load_context;

use super::{
    adapters::Adapter as AdapterInterface,
    attributes::Attribute,
    error::Error,
    record::Record,
    relationships::Relationship::*,
    registry::Registry,
    schema::{RelatedTable, Relationship, TableSchema},
    table::Table,
    query_parameters::{QueryParameters, FilterValue::In},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    ptr,
    slice
};
use crate::database::data_loader::load_context::{LoadContext, RequestedFields};

type RecordCache<'a> = HashMap<(&'a str, i32), Record<'a>>;

pub struct DataLoader<'a, Adapter: AdapterInterface> {
    registry: &'a Registry<'a, Adapter>,
    cache: RecordCache<'a>,
}

fn collection_schema<'a>(collection: &[Record<'a>]) -> Result<&'a TableSchema, Error> {
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

fn collection_attribute<'a>(collection: &'a [Record], attribute: &str) -> Vec<Option<&'a Attribute>> {
    collection
        .iter()
        .map(|record| record.attributes.get(attribute))
        .collect()
}

fn record_id(record: &Record) -> Result<i32, Error> {
    record.attributes
        .get("id")
        .map(|id| match id {
            Attribute::Integer(id) => Ok(*id as i32),
            _ => Err(Error::DataLoadingError {
                message: "Record id was in an unexpected format".to_string(),
            })
        })
        .unwrap_or_else(|| Err(Error::DataLoadingError {
            message: "Record is was not loaded".to_string()
        }))
}

fn merge_into(source: &mut Record, destination: &mut Record) -> Result<(), Error> {
    use Entry::*;

    if source.schema.name != destination.schema.name {
        Err(Error::DataLoadingError {
            message: "Attempted to merge records with differing schemas".to_string()
        })?;
    }

    for (relationship, _) in source.schema.relationships.iter() {
        let source_entry = source.relationships.entry(relationship);
        let destination_entry = destination.relationships.entry(relationship);

        match (source_entry, destination_entry) {
            (Occupied(existing), Occupied(new)) if existing.get() != new.get() =>
                Err(Error::DataLoadingError {
                    message: format!(
                        "Attempted to merge relationship '{}' into a record that already had it set",
                        relationship
                    )
                }),
            (Occupied(occupied), Vacant(vacant)) => {
                vacant.insert(occupied.remove());
                Ok(())
            },
            _ => Ok(())

        }?;
    }

    Ok(())
}

impl<'a, Adapter: AdapterInterface> DataLoader<'a, Adapter> {
    pub fn new(registry: &'a Registry<'a, Adapter>) -> Self {
        DataLoader { registry, cache: HashMap::new() }
    }

    pub fn load_for_record<'b>(self, record: &'b mut Record<'a>, query_parameters: &'b QueryParameters)
                           -> Result<Vec<Record<'a>>, Error>
    {
        self.load_for_collection(slice::from_mut(record), query_parameters)
    }

    pub fn load_for_collection<'b>(mut self, collection: &'b mut [Record<'a>], query_parameters: &'b QueryParameters)
        -> Result<Vec<Record<'a>>, Error>
    {
        if !collection.is_empty() {
            let schema = collection_schema(collection)?;
            let context = LoadContext::new(schema, self.registry, query_parameters)?;

            self.load_with_context(collection, &context)?;
        }

        Ok(self.cache.into_values().collect())
    }

    fn load_with_context<'b>(&mut self, collection: &'b mut [Record<'a>], context: &LoadContext<'a, 'b, Adapter>)
                             -> Result<(), Error>
    {
        for (relationship, descriptor) in context.relationships_to_load() {
            self.load_relationship(collection, &context, relationship, descriptor)?;
        }

        Ok(())
    }

    fn load_relationship<'b>(&mut self, collection: &'b mut [Record<'a>], context: &LoadContext<'a, 'b, Adapter>, relationship: &'a str, descriptor: &'a Relationship)
                         -> Result<(), Error>
    {
        let mut related_collection = match descriptor {
            Relationship::BelongsTo(descriptor) =>
                self.load_belongs_to(relationship, descriptor, collection, context)?,
            Relationship::HasMany(descriptor) =>
                self.load_has_many(relationship, descriptor, collection, context)?,
            Relationship::HasOne(descriptor) =>
                self.load_has_one(relationship, descriptor, collection, context)?,
        };

        if context.is_included(relationship) {
            let derived_context = context.derive(relationship)?;
            self.load_with_context(related_collection.as_mut_slice(), &derived_context)?;

            for mut record in related_collection {
                let id = record_id(&record)?;
                match self.cache.entry((record.schema.name, id))
                {
                    Entry::Occupied(mut existing) => {
                        merge_into(&mut record, existing.get_mut())?;
                    },
                    Entry::Vacant(entry) => {
                        entry.insert(record);
                    }
                }
            }
        }

        Ok(())
    }

    fn load_belongs_to<'b>(&mut self, relationship: &'a str, descriptor: &'a RelatedTable, collection: &'b mut [Record<'a>], context: &LoadContext<'a, 'b, Adapter>)
                           -> Result<Vec<Record<'a>>, Error>
    {
        let requested = context.is_requested(relationship);
        let included = context.is_included(relationship);
        let joins_on_id = descriptor.columns.related == "id";

        let query_needed = included || (requested && !joins_on_id);
        if query_needed {
            let table = context.registry.table(descriptor.table)?;
            let own_attributes = collection_attribute(collection, descriptor.columns.own);
            let related_collection =
                Self::load_collection_by(table, descriptor.columns.related, own_attributes.as_slice(), &context.fields)?;
            let index =
                Self::index_for_unique_attribute(related_collection.as_slice(), descriptor.columns.related, relationship)?;

            for record in collection {
                let id = record_id(&record)?;

                if let Some(attribute) = record.attributes.get(descriptor.columns.own) {
                    let related_id = index
                        .get(attribute)
                        .ok_or_else(|| Error::DataLoadingError {
                            message: format!(
                                "Relationship '{}' of model '{}' with id '{}' references record '{}' with attribute '{}' set to '{}', but the record was not found",
                                relationship, record.schema.name, id,
                                descriptor.table, descriptor.columns.related, attribute
                            )
                        })?;
                    record.relationships.insert(relationship, BelongsTo(*related_id));
                }
            }

            Ok(related_collection)
        } else {
            for record in collection {
                if let Some(id) = record.attributes.get(descriptor.columns.own) {
                    record.relationships
                        .insert(relationship, BelongsTo(*id.as_i64()? as i32));
                }
            }

            Ok(Vec::new())
        }
    }

    fn load_has_one<'b>(&mut self, relationship: &'a str, descriptor: &'a RelatedTable, collection: &'b mut [Record<'a>], context: &LoadContext<'a, 'b, Adapter>)
                        -> Result<Vec<Record<'a>>, Error>
    {
        let table = context.registry.table(descriptor.table)?;
        let own_attributes = collection_attribute(collection, descriptor.columns.own);
        let related_collection =
            Self::load_collection_by(table, descriptor.columns.related, own_attributes.as_slice(), &context.fields)?;
        let index =
            Self::index_for_unique_attribute(related_collection.as_slice(), descriptor.columns.related, relationship)?;

        for record in collection {
            if let Some(attribute) = record.attributes.get(descriptor.columns.own) {
                if let Some(related_id) = index.get(attribute) {
                    record.relationships.insert(relationship, HasOne(*related_id));
                }
            }
        }

        Ok(related_collection)
    }

    fn load_has_many<'b>(&mut self, relationship: &'a str, descriptor: &'a RelatedTable, collection: &'b mut [Record<'a>], context: &LoadContext<'a, 'b, Adapter>)
                         -> Result<Vec<Record<'a>>, Error>
    {
        let table = context.registry.table(descriptor.table)?;
        let own_attributes = collection_attribute(collection, descriptor.columns.own);
        let related_collection =
            Self::load_collection_by(table, descriptor.columns.related, own_attributes.as_slice(), &context.fields)?;
        let mut index =
            Self::index_for_repeating_attribute(related_collection.as_slice(), descriptor.columns.related, relationship)?;

        for record in collection {
            if let Some(attribute) = record.attributes.get(descriptor.columns.own) {
                if let Some(related_ids) = index.remove(attribute) {
                    record.relationships.insert(relationship, HasMany(related_ids));
                }
            }
        }

        Ok(related_collection)
    }

    fn load_collection_by<'b>(table: &Adapter::Table<'a>, column: &str, attributes: &[Option<&Attribute>], fields: &RequestedFields)
        -> Result<Vec<Record<'a>>, Error>
    {
        let attributes = attributes
        .iter()
        .filter_map(|entry| match entry {
            None => None,
            Some(Attribute::Null) => None,
            Some(attribute) => Some(attribute.to_string()),
        })
        .collect();

        let query_parameters = QueryParameters {
            filter: Some([
                (column.to_string(), vec![In(attributes)])
            ].into()),
            fields: Some(fields
                .iter()
                .map(|(key, value)| {
                    (key.to_string(), value.iter().map(ToString::to_string).collect())
                })
                .collect()),
            ..QueryParameters::default()
        };

        table.query(&query_parameters)
    }

    fn index_for_unique_attribute<'b>(collection: &'b [Record], attribute: &str, relationship: &str)
                                      -> Result<HashMap<&'b Attribute, i32>, Error>
    {
        collection
            .iter()
            .map(|record| {
                let foreign_key = record.attributes.get(attribute)
                    .ok_or_else(|| Error::DataLoadingError {
                        message: format!(
                            "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                            attribute, relationship, record.schema.name
                        )
                    })?;

                let id = record_id(record)?;

                Ok((foreign_key, id))
            })
            .collect()
    }

    fn index_for_repeating_attribute<'b>(collection: &'b [Record], attribute: &str, relationship: &str)
        -> Result<HashMap<&'b Attribute, Vec<i32>>, Error>
    {
        let mut index = HashMap::new();
        for record in collection {
            let foreign_key = record.attributes.get(attribute)
                .ok_or_else(|| Error::DataLoadingError {
                    message: format!(
                        "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                        attribute, relationship, record.schema.name
                    )
                })?;

            let id = record_id(record)?;
            index
                .entry(foreign_key)
                .or_insert_with(Vec::new)
                .push(id);
        }

        Ok(index)
    }
}

