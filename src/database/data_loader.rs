use super::{
    adapters::Adapter as AdapterInterface,
    attributes::Attribute,
    error::Error,
    record::Record,
    relationships::Relationship::*,
    registry::Registry,
    schema::{RelatedTable, Relationship, TableSchema},
    table::Table,
    query_parameters::{QueryParameters, FieldsParameters, FilterValue::In},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    ptr,
    slice
};

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
        if let Some(relationship_paths) = &query_parameters.include {
            for relationship_path in relationship_paths {
                self.load_relationship(relationship_path, collection, &query_parameters.fields)?;
            }
        }

        Ok(self.cache.into_values().collect())
    }
    
    fn load_relationship<'b>(&mut self, relationship_path: &'b str, collection: &'b mut [Record<'a>], fields: &'b Option<FieldsParameters>)
                         -> Result<(), Error>
    {
        let schema = collection_schema(collection)?;

        let (relationship, rest) = match relationship_path.split_once('.') {
                Some((relationship, rest)) => (relationship, Some(rest)),
                None => (relationship_path, None)
            };

        let (relationship, relationship_def) = schema
            .relationships
            .iter()
            .find(|(r, _)| *r == relationship)
            .ok_or_else(|| Error::DataLoadingError {
                message: format!("Relationship '{}' is not valid for model '{}'", relationship, schema.name)
            })?;


        let table = self.registry.table(match relationship_def {
            Relationship::BelongsTo(related) |
            Relationship::HasOne(related) |
            Relationship::HasMany(related) => related.table
        })?;

        let mut related_collection = match relationship_def {
            Relationship::BelongsTo(related) => {
                Self::load_belongs_to(table, relationship, related, collection, fields)?
            },
            Relationship::HasMany(related) => {
                Self::load_has_many(table, relationship, related, collection, fields)?
            },
            Relationship::HasOne(related) => {
                Self::load_has_one(table, relationship, related, collection, fields)?
            },
        };

        if let Some(path) = rest {
            self.load_relationship(path, related_collection.as_mut_slice(), fields)?
        }

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

        Ok(())
    }

    fn load_belongs_to<'b>(table: &Adapter::Table<'a>, relationship: &'a str, related_table: &'a RelatedTable, collection: &'b mut [Record<'a>], fields: &'b Option<FieldsParameters>)
        -> Result<Vec<Record<'a>>, Error>
    {
        let own_attributes = collection_attribute(collection, related_table.columns.own);
        let related_collection =
            Self::load_collection_by(table, related_table.columns.related, own_attributes.as_slice(), fields)?;
        let index =
            Self::index_for_unique_attribute(related_collection.as_slice(), related_table.columns.related, relationship)?;

        for record in collection {
            let id = record_id(&record)?;

            if let Some(attribute) = record.attributes.get(related_table.columns.own) {
                let related_id = index
                    .get(attribute)
                    .ok_or_else(|| Error::DataLoadingError {
                        message: format!(
                            "Relationship '{}' of model '{}' with id '{}' references record '{}' with attribute '{}' set to '{}', but the record was not found",
                            relationship, record.schema.name, id,
                            related_table.table, related_table.columns.related, attribute
                        )
                    })?;
                record.relationships.insert(relationship, BelongsTo(*related_id));
            }
        }

        Ok(related_collection)
    }

    fn load_has_one<'b>(table: &Adapter::Table<'a>, relationship: &'a str, related_table: &'a RelatedTable, collection: &'b mut [Record<'a>], fields: &'b Option<FieldsParameters>)
                        -> Result<Vec<Record<'a>>, Error>
    {
        let own_attributes = collection_attribute(collection, related_table.columns.own);
        let related_collection =
            Self::load_collection_by(table, related_table.columns.related, own_attributes.as_slice(), fields)?;
        let index =
            Self::index_for_unique_attribute(related_collection.as_slice(), related_table.columns.related, relationship)?;

        for record in collection {
            if let Some(attribute) = record.attributes.get(related_table.columns.own) {
                if let Some(related_id) = index.get(attribute) {
                    record.relationships.insert(relationship, HasOne(*related_id));
                }
            }
        }

        Ok(related_collection)
    }

    fn load_has_many<'b>(table: &Adapter::Table<'a>, relationship: &'a str, related_table: &'a RelatedTable, collection: &'b mut [Record<'a>], fields: &'b Option<FieldsParameters>)
                         -> Result<Vec<Record<'a>>, Error>
    {
        let own_attributes = collection_attribute(collection, related_table.columns.own);
        let related_collection =
            Self::load_collection_by(table, related_table.columns.related, own_attributes.as_slice(), fields)?;
        let mut index =
            Self::index_for_repeating_attribute(related_collection.as_slice(), related_table.columns.related, relationship)?;

        for record in collection {
            if let Some(attribute) = record.attributes.get(related_table.columns.own) {
                if let Some(related_ids) = index.remove(attribute) {
                    record.relationships.insert(relationship, HasMany(related_ids));
                }
            }
        }

        Ok(related_collection)
    }

    fn load_collection_by<'b>(table: &Adapter::Table<'a>, column: &str, attributes: &[Option<&Attribute>], fields: &Option<FieldsParameters>)
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
            fields: fields.clone(),
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

