use crate::database::{
    adapters::Adapter as AdapterInterface,
    attributes::Attribute,
    record::Record,
    relationships::Relationship::*,
    registry::Registry,
    schema::{RelatedTable, Relationship, TableSchema},
    query_parameters::{QueryParameters, FieldsParameters},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    ptr,
    slice
};
use crate::core::error::Error::DataLoadingError;
use crate::database::query_parameters::FilterValue::In;
use crate::database::table::Table;
use super::error::Error;

type RecordCache<'a> = HashMap<&'a str, HashMap<i32, Record<'a>>>;

struct DataLoader<'a, Adapter: AdapterInterface> {
    registry: &'a Registry<'a, Adapter>,
    cache: RecordCache<'a>,
}


fn collection_schema<'a, 'b>(collection: &'a [Record<'b>]) -> Result<&'b TableSchema, Error> {
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

fn collection_attribute<'a>(collection: &'a [Record], attribute: &str, relationship: &str,) -> Result<Vec<&'a Attribute>, Error> {
    collection
        .iter()
        .map(|record|
            record.attributes
                .get(attribute)
                .ok_or_else(|| DataLoadingError {
                    message: format!(
                        "Attribute '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                        attribute, relationship, record.schema.name)
                })
        )
        .collect()
}

fn record_id(record: &Record) -> Result<i32, Error> {
    record.attributes
        .get("id")
        .map(|id| match id {
            Attribute::Integer(id) => Ok(*id as i32),
            _ => Err(DataLoadingError {
                message: "Record id was in an unexpected format".to_string(),
            })
        })
        .unwrap_or_else(|| Err(DataLoadingError {
            message: "Record is was not loaded".to_string()
        }))
}

fn merge_into(source: &mut Record, destination: &mut Record) -> Result<(), Error> {
    if(source.schema.name != destination.schema.name) {
        Err(DataLoadingError {
            message: "Attempted to merge records with differing schemas".to_string()
        })?;
    }

    for (relationship, _) in source.schema.relationships.iter() {
        let source_entry = source.relationships.entry(relationship);
        let destination_entry = destination.relationships.entry(relationship);

        match (source_entry, destination_entry) {
            (Entry::Occupied(_), Entry::Occupied(_)) =>
                Err(DataLoadingError {
                    message: format!(
                        "Attempted to merge relationship '{}' into a record that already had it set",
                        relationship
                    )
                }),
            (Entry::Occupied(occupied), Entry::Vacant(vacant)) => {
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

    pub fn load_for_record(&mut self, record: &mut Record, query_parameters: &QueryParameters)
                           -> Result<(), Error>
    {
        self.load_for_collection(slice::from_mut(record), query_parameters)
    }

    pub fn load_for_collection(&mut self, collection: &mut [Record], query_parameters: &QueryParameters)
        -> Result<(), Error>
    {
        if let Some(relationship_paths) = &query_parameters.include {
            for relationship_path in relationship_paths {
                self.load_relationship(relationship_path, collection, &query_parameters.fields)?;
            }
        }

        Ok(())
    }
    fn load_relationship(&mut self, relationship_path: &str, collection: &mut [Record], fields: &Option<FieldsParameters>)
                         -> Result<(), Error>
    {
        let schema = collection_schema(collection)?;

        let (relationship, rest) = match relationship_path.split_once('.') {
                Some((relationship, rest)) => (relationship, Some(rest)),
                None => (relationship_path, None)
            };

        let relationship_def = schema
            .relationship(relationship)
            .ok_or_else(|| DataLoadingError {
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

        if let Some(relationship_path) = rest {
            self.load_relationship(relationship_path, related_collection.as_mut_slice(), fields)?
        }

        for mut record in related_collection.into_iter() {
            let id = record_id(&record)?;
            match self.cache
                .entry(record.schema.name).or_insert_with(|| HashMap::new())
                .entry(id)
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

    fn load_belongs_to<'b>(table: &Adapter::Table<'a>, relationship: &str, related_table: &RelatedTable, collection: &mut [Record], fields: &Option<FieldsParameters>)
        -> Result<Vec<Record<'b>>, Error>
    {
        let own_attributes = collection_attribute(collection, related_table.columns.own, relationship)?;
        let filter_value = vec![
            In(own_attributes
                .into_iter()
                .map(ToString::to_string)
                .collect()
            )
        ];
        let query_parameters = QueryParameters {
            filter: Some([
                (related_table.columns.related.to_string(), filter_value)
            ].into()),
            fields: fields.clone(),
            ..QueryParameters::default()
        };

        let related_collection = table.query(&query_parameters)?;
        let index = HashMap::from_iter(related_collection
            .iter()
            .map(|record| {
                let foreign_key = record.attributes.get(related_table.columns.related)
                    .ok_or_else(|| DataLoadingError {
                        message: format!(
                            "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                            related_table.columns.related, relationship, record.schema.name
                        )
                    })?;

                let id = record_id(record)?;

                Ok((foreign_key, id))
            })
            .collect::<Result<HashMap<_, _>, _>>()?
        );

        for (record, attribute) in collection.iter_mut().zip(own_attributes) {
            let id = record_id(&record)?;
            let related_id = index
                .get(attribute)
                .ok_or_else(|| DataLoadingError {
                    message: format!(
                        "Relationship '{}' of model '{}' with id '{}' references record '{}' with attribute '{}' set to '{}', but the record was not found",
                        relationship, record.schema.name, id,
                        related_table.table, related_table.columns.related, attribute
                    )
                })?;
            record.relationships.insert(relationship, BelongsTo(*related_id));
        }

        Ok(related_collection)
    }

    fn load_has_one<'b>(table: &Adapter::Table<'a>, relationship: &str, related_table: &RelatedTable, collection: &mut [Record], fields: &Option<FieldsParameters>)
                        -> Result<Vec<Record<'a>>, Error>
    {
        let own_attributes = collection_attribute(collection, related_table.columns.own, relationship)?;
        let filter_value = vec![
            In(own_attributes
                .iter()
                .map(|attr| attr.to_string())
                .collect()
            )
        ];
        let query_parameters = QueryParameters {
            filter: Some([
                (related_table.columns.related.to_string(), filter_value)
            ].into()),
            fields: fields.clone(),
            ..QueryParameters::default()
        };

        let related_collection = table.query(&query_parameters)?;

        // let mut index: HashMap<&Attribute, i32> = HashMap::new();
        // for record in related_collection.iter() {
        //     let foreign_key = record.attributes.get(related_table.columns.related)
        //         .ok_or_else(|| DataLoadingError {
        //             message: format!(
        //                 "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
        //                 related_table.columns.related, relationship, record.schema.name
        //             )
        //         })?;
        //
        //     let id = record_id(record)?;
        //
        //     // Only insert if not already present (first match wins)
        //     index.entry(foreign_key).or_insert(id);
        // }

        let index = HashMap::from_iter(related_collection
            .iter()
            .map(|record| {
                let foreign_key = record.attributes.get(related_table.columns.related)
                    .ok_or_else(|| DataLoadingError {
                        message: format!(
                            "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                            related_table.columns.related, relationship, record.schema.name
                        )
                    })?;

                let id = record_id(record)?;

                Ok((foreign_key, id))
            })
            .collect::<Result<HashMap<_, _>, _>>()?
        );

        // Link parent records to their related record
        for (record, attribute) in collection.iter_mut().zip(own_attributes) {
            if let Some(&related_id) = index.get(attribute) {
                record.relationships.insert(relationship, HasOne(related_id));
            }
            // If no match found, the relationship remains unset (optional relationship)
        }

        Ok(related_collection)
    }


    fn load_has_many<'b>(table: &Adapter::Table<'a>, relationship: &str, related_table: &RelatedTable, collection: &mut [Record], fields: &Option<FieldsParameters>)
                         -> Result<Vec<Record<'a>>, Error>
    {
        let own_attributes = collection_attribute(collection, related_table.columns.own, relationship)?;
        let filter_value = vec![
            In(own_attributes
                .iter()
                .map(|attr| attr.to_string())
                .collect()
            )
        ];
        let query_parameters = QueryParameters {
            filter: Some([
                (related_table.columns.related.to_string(), filter_value)
            ].into()),
            fields: fields.clone(),
            ..QueryParameters::default()
        };

        let related_collection = table.query(&query_parameters)?;

        // Build index: foreign_key -> Vec<record_id>
        let mut index: HashMap<&Attribute, Vec<i32>> = HashMap::new();
        for record in related_collection.iter() {
            let foreign_key = record.attributes.get(related_table.columns.related)
                .ok_or_else(|| DataLoadingError {
                    message: format!(
                        "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                        related_table.columns.related, relationship, record.schema.name
                    )
                })?;

            let id = record_id(record)?;

            index.entry(foreign_key).or_insert_with(Vec::new).push(id);
        }

        // Link parent records to their related records
        for (record, attribute) in collection.iter_mut().zip(own_attributes) {
            if let Some(related_ids) = index.get(attribute) {
                record.relationships.insert(relationship, HasMany(related_ids.clone()));
            } else {
                // Empty has_many relationship
                record.relationships.insert(relationship, HasMany(Vec::new()));
            }
        }

        Ok(related_collection)
    }

    fn load_related_collection_by(related_table: Adapter::Table<'_>, related_column: &str, collection: &[Record], column: &str, relationship: &str, fields: &Option<FieldsParameters>)
        -> Result<Vec<Record<'a>>, Error>
    {
        let attributes = collection_attribute(collection, column, relationship)?;
        let filter_value = vec![
            In(attributes
                .iter()
                .map(|attr| attr.to_string())
                .collect()
            )
        ];
        let query_parameters = QueryParameters {
            filter: Some([
                (related_column.to_string(), filter_value)
            ].into()),
            fields: fields.clone(),
            ..QueryParameters::default()
        };

        related_table.query(&query_parameters)
    }
}

