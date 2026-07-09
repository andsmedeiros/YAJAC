#[cfg(test)]
mod tests;

use super::{
    adapters::Adapter as AdapterInterface,
    attributes::Attribute,
    connection_manager::ConnectionManager,
    error::Error,
    query_parameters::{FilterValue::In, QueryParameters},
    record::Record,
    relationships::Relationship::*,
    schema::{RelatedResource, RelationshipDescriptor, RelationshipKind},
    table::Table,
};
use crate::database::attributes::Identifier;
use crate::database::query_parameters::FieldsParameters;
use crate::database::relationships::Relationships;
use crate::utils::indexing::Indexable;
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    slice,
};

type GlobalIdentifier<'sch> = (&'sch str, Identifier);

type RecordCache<'sch> = HashMap<GlobalIdentifier<'sch>, Record<'sch>>;

pub struct DataLoader<'sch, 'req, Adapter: AdapterInterface> {
    manager: &'sch ConnectionManager<'sch, Adapter>,
    connection: &'req Adapter::Connection,
    cache: RecordCache<'sch>,
    included_identifiers: HashSet<GlobalIdentifier<'sch>>,
}

impl<'sch, 'req, Adapter: AdapterInterface> DataLoader<'sch, 'req, Adapter> {
    pub fn new(
        manager: &'sch ConnectionManager<'sch, Adapter>,
        connection: &'req Adapter::Connection,
    ) -> Self {
        DataLoader {
            manager,
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
        descriptor: &'sch RelationshipDescriptor,
    ) -> Result<(), Error> {
        let related = &descriptor.related;
        let mut related_collection = match descriptor.kind {
            RelationshipKind::BelongsTo => {
                self.load_belongs_to(relationship, related, collection, query_parameters)?
            }
            RelationshipKind::HasMany => {
                self.load_has_many(relationship, related, collection, query_parameters)?
            }
            RelationshipKind::HasOne => {
                self.load_has_one(relationship, related, collection, query_parameters)?
            }
        };

        if query_parameters.is_included(relationship) {
            let derived_context = query_parameters.derive(relationship, self.manager.registry())?;
            self.load_with_context(related_collection.as_mut_slice(), &derived_context)?;

            let related_identifiers = related_collection
                .iter()
                .map(|record| Ok((record.schema.name(), record.require_id()?.clone())))
                .collect::<Result<Vec<_>, Error>>()?;
            self.included_identifiers.extend(related_identifiers);
        }

        for record in related_collection {
            match self
                .cache
                .entry((record.schema.name(), record.require_id()?.clone()))
            {
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
        let related_schema = self.manager.registry().schema(descriptor.resource)?;
        let joins_on_primary_key = related_schema.is_primary_key(descriptor.keys.related);

        let requested = query_parameters.is_requested(relationship);
        let included = query_parameters.is_included(relationship);
        let query_needed = included || (requested && !joins_on_primary_key);

        let related_collection = if query_needed {
            let table = self.manager.table(descriptor.resource, self.connection)?;
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
                    if let Some(related_id) = record.get_owned(descriptor.keys.own)
                        && !matches!(related_id, Attribute::Null)
                    {
                        record
                            .relationships
                            .insert(relationship, BelongsTo(Identifier::try_from(related_id)?));
                    }
                }
            } else {
                let index = Self::index_by_unique_foreign_key(
                    related_collection.as_slice(),
                    descriptor.keys.related,
                    relationship,
                )?;

                for record in collection {
                    if let Some(attribute) = record.get_owned(descriptor.keys.own)
                        && !matches!(attribute, Attribute::Null)
                    {
                        let related_id = index
                            .get(&attribute)
                            .copied()
                            .ok_or_else(|| {
                                let id = record.require_id()
                                    .map(ToString::to_string)
                                    .unwrap_or("".to_string());

                                Error::DataLoadingError {
                                message: format!(
                                    "Relationship '{}' of model '{}' with id '{}' references record '{}' with attribute '{}' set to '{}', but the record was not found",
                                    relationship, record.schema.name(), id,
                                    descriptor.resource, descriptor.keys.related, attribute
                                )
                            }
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
        let table = self.manager.table(descriptor.resource, self.connection)?;
        let own_attributes = Self::collection_attribute(collection, descriptor.keys.own);
        let related_collection = Self::load_collection_by(
            &table,
            descriptor.keys.related,
            &own_attributes,
            &query_parameters.fields,
        )?;
        let index = Self::index_by_unique_foreign_key(
            &related_collection,
            descriptor.keys.related,
            relationship,
        )?;

        if query_parameters.is_requested(relationship) {
            for record in collection {
                if let Some(attribute) = record.get_owned(descriptor.keys.own)
                    && let Some(related_id) = index.get(&attribute).copied()
                {
                    record
                        .relationships
                        .insert(relationship, HasOne(related_id.clone()));
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
        let table = self.manager.table(descriptor.resource, self.connection)?;
        let own_attributes = Self::collection_attribute(collection, descriptor.keys.own);
        let related_collection = Self::load_collection_by(
            &table,
            descriptor.keys.related,
            own_attributes.as_slice(),
            &query_parameters.fields,
        )?;
        let mut index = Self::group_by_foreign_key(
            related_collection.as_slice(),
            descriptor.keys.related,
            relationship,
        )?;

        if query_parameters.is_requested(relationship) {
            for record in collection {
                if let Some(attribute) = record.get_owned(descriptor.keys.own)
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

        table
            .query(&query_parameters)?
            .into_iter()
            .map(|row| Record::try_from_row(table.schema(), row))
            .collect()
    }

    /// Borrows a record's loaded foreign key, erroring if the column was not materialised.
    fn require_foreign_key<'a>(
        record: &'a Record<'sch>,
        key: &str,
        relationship: &str,
    ) -> Result<&'a Attribute, Error> {
        record.foreign_keys.get(key).ok_or_else(|| Error::DataLoadingError {
            message: format!(
                "Foreign key '{}', necessary for loading the relationship '{}' on model '{}', is not loaded.",
                key, relationship, record.schema.name()
            ),
        })
    }

    /// Indexes a related collection by each record's foreign key, borrowing key and id in place. The
    /// key is expected unique (a `BelongsTo`/`HasOne` join), so a collision is an inconsistency; the
    /// id is borrowed and cloned only when a lookup matches.
    fn index_by_unique_foreign_key<'a>(
        collection: &'a [Record<'sch>],
        key: &str,
        relationship: &str,
    ) -> Result<HashMap<&'a Attribute, &'a Identifier>, Error> {
        collection
            .iter()
            .try_index_with(|record| {
                Ok::<_, Error>((
                    Self::require_foreign_key(record, key, relationship)?,
                    record.require_id()?,
                ))
            })
            .map_err(Error::from)
    }

    /// Groups a related collection by each record's foreign key, borrowing the key but owning the
    /// ids, so records sharing a key (a `HasMany` join) gather under it and the whole group can be
    /// moved out on a lookup.
    fn group_by_foreign_key<'a>(
        collection: &'a [Record<'sch>],
        key: &str,
        relationship: &str,
    ) -> Result<HashMap<&'a Attribute, Vec<Identifier>>, Error> {
        collection
            .iter()
            .try_group_with(|record| {
                Ok::<_, Error>((
                    Self::require_foreign_key(record, key, relationship)?,
                    record.require_id()?.clone(),
                ))
            })
            .map_err(Error::from)
    }

    fn collection_attribute(collection: &[Record], attribute: &str) -> Vec<Option<Attribute>> {
        collection
            .iter()
            .map(|record| record.get_owned(attribute))
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
