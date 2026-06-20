#[cfg(test)]
mod tests;

use super::{
    adapters::Adapter as AdapterInterface,
    attributes::Attribute,
    error::Error,
    query_parameters::{FilterValue::In, QueryParameters},
    record::Record,
    registry::Registry,
    relationships::Relationship::*,
    schema::{RelatedResource, Relationship},
    table::Table,
};
use crate::database::attributes::Identifier;
use crate::database::query_parameters::FieldsParameters;
use crate::database::relationships::Relationships;
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    slice,
};

type GlobalIdentifier<'sch> = (&'sch str, Identifier);

type RecordCache<'sch> = HashMap<GlobalIdentifier<'sch>, Record<'sch>>;

pub struct DataLoader<'sch, 'req, Adapter: AdapterInterface> {
    registry: &'sch Registry<'sch, Adapter>,
    connection: &'req Adapter::Connection,
    cache: RecordCache<'sch>,
    included_identifiers: HashSet<GlobalIdentifier<'sch>>,
}

impl<'sch, 'req, Adapter: AdapterInterface> DataLoader<'sch, 'req, Adapter> {
    pub fn new(
        registry: &'sch Registry<'sch, Adapter>,
        connection: &'req Adapter::Connection,
    ) -> Self {
        DataLoader {
            registry,
            connection,
            cache: HashMap::new(),
            included_identifiers: HashSet::new(),
        }
    }

    pub fn load_for_record(
        self,
        record: &mut Record<'sch>,
        query_parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<Vec<Record<'sch>>, Error> {
        self.load_for_collection(slice::from_mut(record), query_parameters)
    }

    pub fn load_for_collection(
        mut self,
        collection: &mut [Record<'sch>],
        query_parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<Vec<Record<'sch>>, Error> {
        if !collection.is_empty() {
            self.load_with_context(collection, query_parameters)?;
        }

        let included = self
            .cache
            .into_iter()
            .filter_map(|(identifier, record)| {
                if self.included_identifiers.contains(&identifier) {
                    Some(record)
                } else {
                    None
                }
            })
            .collect();

        Ok(included)
    }

    fn load_with_context(
        &mut self,
        collection: &mut [Record<'sch>],
        query_parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<(), Error> {
        for (relationship, descriptor) in query_parameters.relationships_to_load() {
            self.load_relationship(collection, query_parameters, relationship, descriptor)?;
        }

        Ok(())
    }

    fn load_relationship(
        &mut self,
        collection: &mut [Record<'sch>],
        query_parameters: &QueryParameters<'sch, 'req>,
        relationship: &'sch str,
        descriptor: &'sch Relationship,
    ) -> Result<(), Error> {
        let mut related_collection = match descriptor {
            Relationship::BelongsTo(descriptor) => {
                self.load_belongs_to(relationship, descriptor, collection, query_parameters)?
            }
            Relationship::HasMany(descriptor) => {
                self.load_has_many(relationship, descriptor, collection, query_parameters)?
            }
            Relationship::HasOne(descriptor) => {
                self.load_has_one(relationship, descriptor, collection, query_parameters)?
            }
        };

        if query_parameters.is_included(relationship) {
            let derived_context = query_parameters.derive(relationship, self.registry)?;
            self.load_with_context(related_collection.as_mut_slice(), &derived_context)?;

            let related_identifiers = related_collection
                .iter()
                .map(|record| Ok((record.schema.name, record.id.clone())))
                .collect::<Result<Vec<_>, Error>>()?;
            self.included_identifiers.extend(related_identifiers);
        }

        for record in related_collection {
            match self.cache.entry((record.schema.name, record.id.clone())) {
                Entry::Occupied(mut existing) => {
                    Self::merge_records(
                        record.relationships,
                        &mut existing.get_mut().relationships,
                    )?;
                }
                Entry::Vacant(entry) => {
                    entry.insert(record);
                }
            }
        }

        Ok(())
    }

    fn load_belongs_to(
        &mut self,
        relationship: &'sch str,
        descriptor: &'sch RelatedResource,
        collection: &mut [Record<'sch>],
        query_parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<Vec<Record<'sch>>, Error> {
        let related_schema = self.registry.schema(descriptor.resource)?;
        let joins_on_primary_key = related_schema.is_primary_key(descriptor.keys.related);

        let requested = query_parameters.is_requested(relationship);
        let included = query_parameters.is_included(relationship);
        let query_needed = included || (requested && !joins_on_primary_key);

        let related_collection = if query_needed {
            let table = self.registry.table(descriptor.resource, self.connection)?;
            let own_attributes = Self::collection_attribute(collection, descriptor.keys.own);
            Self::load_collection_by(
                &table,
                descriptor.keys.related,
                own_attributes.as_slice(),
                &query_parameters.fields,
            )?
        } else {
            Vec::new()
        };

        if requested {
            if joins_on_primary_key {
                for record in collection {
                    if let Some(related_id) = Self::get_attribute(record, descriptor.keys.own)
                        && !matches!(related_id, Attribute::Null)
                    {
                        record
                            .relationships
                            .insert(relationship, BelongsTo(Identifier::try_from(related_id)?));
                    }
                }
            } else {
                let index = Self::index_for_unique_attribute(
                    related_collection.as_slice(),
                    descriptor.keys.related,
                    relationship,
                )?;

                for record in collection {
                    if let Some(attribute) = Self::get_attribute(record, descriptor.keys.own)
                        && !matches!(attribute, Attribute::Null)
                    {
                        let related_id = index
                            .get(&attribute)
                            .ok_or_else(|| Error::DataLoadingError {
                                message: format!(
                                    "Relationship '{}' of model '{}' with id '{}' references record '{}' with attribute '{}' set to '{}', but the record was not found",
                                    relationship, record.schema.name, record.id,
                                    descriptor.resource, descriptor.keys.related, attribute
                                )
                            })?;
                        record
                            .relationships
                            .insert(relationship, BelongsTo(related_id.clone()));
                    }
                }
            }
        }

        Ok(related_collection)
    }

    fn load_has_one(
        &mut self,
        relationship: &'sch str,
        descriptor: &'sch RelatedResource,
        collection: &mut [Record<'sch>],
        query_parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<Vec<Record<'sch>>, Error> {
        let table = self.registry.table(descriptor.resource, self.connection)?;
        let own_attributes = Self::collection_attribute(collection, descriptor.keys.own);
        let related_collection = Self::load_collection_by(
            &table,
            descriptor.keys.related,
            &own_attributes,
            &query_parameters.fields,
        )?;
        let mut index = Self::index_for_unique_attribute(
            &related_collection,
            descriptor.keys.related,
            relationship,
        )?;

        if query_parameters.is_requested(relationship) {
            for record in collection {
                if let Some(attribute) = Self::get_attribute(record, descriptor.keys.own)
                    && let Some(related_id) = index.remove(&attribute)
                {
                    record
                        .relationships
                        .insert(relationship, HasOne(related_id));
                }
            }
        }

        Ok(related_collection)
    }

    fn load_has_many(
        &mut self,
        relationship: &'sch str,
        descriptor: &'sch RelatedResource,
        collection: &mut [Record<'sch>],
        query_parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<Vec<Record<'sch>>, Error> {
        let table = self.registry.table(descriptor.resource, self.connection)?;
        let own_attributes = Self::collection_attribute(collection, descriptor.keys.own);
        let related_collection = Self::load_collection_by(
            &table,
            descriptor.keys.related,
            own_attributes.as_slice(),
            &query_parameters.fields,
        )?;
        let mut index = Self::index_for_repeating_attribute(
            related_collection.as_slice(),
            descriptor.keys.related,
            relationship,
        )?;

        if query_parameters.is_requested(relationship) {
            for record in collection {
                if let Some(attribute) = Self::get_attribute(record, descriptor.keys.own)
                    && let Some(related_ids) = index.remove(&attribute)
                {
                    record
                        .relationships
                        .insert(relationship, HasMany(related_ids));
                }
            }
        }

        Ok(related_collection)
    }

    fn load_collection_by(
        table: &Adapter::Table<'sch, 'req>,
        column: &'sch str,
        attributes: &[Option<Attribute>],
        fields: &FieldsParameters,
    ) -> Result<Vec<Record<'sch>>, Error> {
        let attributes = attributes
            .iter()
            .filter_map(|entry| match entry {
                None | Some(Attribute::Null) => None,
                Some(attribute) => Some(attribute.clone()),
            })
            .collect();

        let query_parameters = QueryParameters {
            filter: Some([(column, vec![In(attributes)])].into()),
            fields: fields
                .iter()
                .map(|(key, value)| (*key, value.iter().copied().collect()))
                .collect(),
            ..QueryParameters::new(table.schema())
        };

        table.query(&query_parameters)
    }

    fn get_attribute(record: &Record, key: &str) -> Option<Attribute> {
        if record.schema.is_primary_key(key) {
            Some(Attribute::from(record.id.clone()))
        } else {
            record
                .attributes
                .get(key)
                .or_else(|| record.foreign_keys.get(key))
                .map(ToOwned::to_owned)
        }
    }

    fn get_foreign_key(record: &Record, key: &str, relationship: &str) -> Result<Attribute, Error> {
        record.foreign_keys.get(key)
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::DataLoadingError {
                message: format!(
                    "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                    key, relationship, record.schema.name
                )
            })
    }

    fn index_for_unique_attribute(
        collection: &[Record],
        attribute: &str,
        relationship: &str,
    ) -> Result<HashMap<Attribute, Identifier>, Error> {
        collection
            .iter()
            .map(|record| {
                let foreign_key = Self::get_foreign_key(record, attribute, relationship)?;
                Ok((foreign_key, record.id.clone()))
            })
            .collect()
    }

    fn index_for_repeating_attribute(
        collection: &[Record],
        attribute: &str,
        relationship: &str,
    ) -> Result<HashMap<Attribute, Vec<Identifier>>, Error> {
        let mut index = HashMap::new();
        for record in collection {
            let foreign_key = Self::get_foreign_key(record, attribute, relationship)?;

            index
                .entry(foreign_key)
                .or_insert_with(Vec::new)
                .push(record.id.clone());
        }

        Ok(index)
    }

    fn collection_attribute(collection: &[Record], attribute: &str) -> Vec<Option<Attribute>> {
        collection
            .iter()
            .map(|record| Self::get_attribute(record, attribute))
            .collect()
    }

    fn merge_records(
        source: Relationships<'sch>,
        destination: &mut Relationships<'sch>,
    ) -> Result<(), Error> {
        use Entry::*;

        for (relationship, value) in source {
            match destination.entry(relationship) {
                Occupied(entry) if value != *entry.get() => Err(Error::DataLoadingError {
                    message: format!(
                        "Attempted to merge relationship '{}' into a record that already had it set",
                        relationship
                    ),
                }),
                Vacant(entry) => {
                    entry.insert(value);
                    Ok(())
                }
                _ => Ok(()),
            }?;
        }

        Ok(())
    }
}
