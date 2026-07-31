use core::slice;
use std::collections::HashMap;

use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::attributes::{Attribute, Identifier, Row};
use crate::database::composite::{Composite, CompositeCollection, CompositeRecord};
use crate::database::connection::Connection as ConnectionInterface;
use crate::database::connection_manager::ConnectionManager;
use crate::database::data_loader::DataLoader;
use crate::database::error::{ConstraintKind, Error};
use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
use crate::database::record::{Record, RecordPatch, Refreshable};
use crate::database::relationships::Relationship as DatabaseRelationship;
use crate::database::schema::{RelationshipKind, Schema};
use crate::database::table::Table as TableInterface;
use crate::utils::indexing::Indexable;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;

/// Recasts a foreign-key violation into the reference-aware `Error` the record writers surface. Which
/// resource is missing depends on the key's side: a belongs-to write's key points at the target
/// (`RelatedRecordNotFound`); a has-one/has-many write's key points back at the primary
/// (`RecordNotFound`).
mod error_mapper {
    use super::{ConstraintKind, Error};

    /// A foreign-key violation whose key points at the target: the referenced resource is missing.
    pub(super) fn fk_violation_to_missing_reference(error: Error) -> Error {
        match error {
            Error::ConstraintViolation {
                kind: ConstraintKind::ForeignKey,
                ..
            } => Error::RelatedRecordNotFound,
            other => other,
        }
    }

    /// A foreign-key violation whose key points back at the primary: the primary record is missing.
    pub(super) fn fk_violation_to_missing_record(error: Error) -> Error {
        match error {
            Error::ConstraintViolation {
                kind: ConstraintKind::ForeignKey,
                ..
            } => Error::RecordNotFound,
            other => other,
        }
    }
}

pub struct Store<'sch: 'req, 'req, Adapter: AdapterInterface> {
    manager: &'sch ConnectionManager<'sch, Adapter>,
    connection: &'req Adapter::Connection,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface> Store<'sch, 'req, Adapter> {
    pub fn new(
        manager: &'sch ConnectionManager<'sch, Adapter>,
        connection: &'req Adapter::Connection,
    ) -> Self {
        Store {
            manager,
            connection,
        }
    }

    pub fn fetch_record(
        &self,
        schema: &'sch Schema<'sch>,
        id: Identifier,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let row = self.table(schema)?.find(id, parameters)?;
            let mut content = Record::try_from_row(schema, row)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn fetch_collection(
        &self,
        schema: &'sch Schema<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        self.connection.transaction(|| {
            let mut content = self
                .table(schema)?
                .query(parameters)?
                .into_iter()
                .map(|row| Record::try_from_row(schema, row))
                .collect::<Result<Vec<_>, _>>()?;
            let included = self
                .loader()
                .load_for_collection(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn create_record(
        &self,
        mut record: Record<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection
            .transaction(|| {
                let schema = record.schema;
                self.attach_belongs_to(slice::from_mut(&mut record))?;
                let id = record.id.take();
                record.refresh_with(|mut row| {
                    if let Some(id) = id {
                        row.insert(schema.primary_key().name, id.into());
                    }
                    self.table(schema)?.insert(row, parameters)
                })?;
                self.attach_has_one_many(slice::from_ref(&record), false)?;
                let included = self.loader().load_for_record(&mut record, parameters)?;

                Ok(Composite {
                    content: record,
                    included,
                })
            })
            .map_err(error_mapper::fk_violation_to_missing_reference)
    }

    pub fn update_record(
        &self,
        mut record: Record<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection
            .transaction(|| {
                let schema = record.schema;
                self.attach_belongs_to(slice::from_mut(&mut record))?;
                let id = record.require_id()?.clone();
                record.refresh_with(|row| {
                    if row.is_empty() {
                        self.table(schema)?.find(id, parameters)
                    } else {
                        self.table(schema)?.update(id, row, parameters)
                    }
                })?;
                self.attach_has_one_many(slice::from_ref(&record), true)?;
                let included = self.loader().load_for_record(&mut record, parameters)?;

                Ok(Composite {
                    content: record,
                    included,
                })
            })
            .map_err(error_mapper::fk_violation_to_missing_reference)
    }

    pub fn delete_record(&self, schema: &'sch Schema<'sch>, id: Identifier) -> Result<(), Error> {
        self.connection
            .transaction(|| self.table(schema)?.delete(id))
    }

    pub fn create_collection(
        &self,
        mut records: Vec<Record<'sch>>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        let schema = if let Some(first) = records.first() {
            first.schema
        } else {
            return Ok(Composite {
                content: Vec::new(),
                included: Vec::new(),
            });
        };

        if records.iter().any(|record| record.schema != schema) {
            return Err(Error::InconsistentCollection);
        }

        self.connection
            .transaction(|| {
                self.attach_belongs_to(&mut records)?;
                let ids: Vec<_> = records.iter_mut().map(|record| record.id.take()).collect();
                records.refresh_with(|mut rows| {
                    for (row, id) in rows.iter_mut().zip(ids) {
                        if let Some(id) = id {
                            row.insert(schema.primary_key().name, id.into());
                        }
                    }
                    self.table(schema)?.insert_batch(rows, parameters)
                })?;
                self.attach_has_one_many(&records, false)?;
                let included = self
                    .loader()
                    .load_for_collection(&mut records, parameters)?;

                Ok(Composite {
                    content: records,
                    included,
                })
            })
            .map_err(error_mapper::fk_violation_to_missing_reference)
    }

    pub fn update_collection(
        &self,
        patch: RecordPatch<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        let schema = patch.schema;
        let mut patch = Record::from(patch);
        self.connection
            .transaction(|| {
                self.attach_belongs_to(slice::from_mut(&mut patch))?;
                let row = patch.take_row();
                let mut records = self
                    .table(schema)?
                    .update_batch(row, parameters)?
                    .into_iter()
                    .map(|row| {
                        Record::try_from_row(schema, row)
                            .map(|record| record.with_relationships(patch.relationships.clone()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.attach_has_one_many(&records, true)?;
                let included = self
                    .loader()
                    .load_for_collection(&mut records, parameters)?;

                Ok(Composite {
                    content: records,
                    included,
                })
            })
            .map_err(error_mapper::fk_violation_to_missing_reference)
    }

    pub fn delete_collection(
        &self,
        schema: &'sch Schema<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<(), Error> {
        self.connection
            .transaction(|| self.table(schema)?.delete_batch(parameters))
            .and(Ok(()))
    }

    /// Fetches the full records targeted by the already-loaded `record`'s `relationship`, scoped to
    /// the relationship's foreign key and shaped by `parameters`. Empty when the relationship is
    /// unset. Errors when `relationship` is not declared on the record's schema.
    pub fn fetch_related_collection(
        &self,
        record: &Record<'sch>,
        relationship: &'req str,
        mut parameters: QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        let schema = record.schema();
        let descriptor =
            schema
                .relationship(relationship)
                .ok_or_else(|| Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship.into(),
                })?;
        let keys = &descriptor.related.keys;
        let related_schema = self
            .manager
            .registry()
            .schema(descriptor.related.resource)?;

        if related_schema != parameters.schema {
            return Err(Error::MismatchedQueryParameters {
                expected: related_schema.name().into(),
                actual: parameters.schema.name().into(),
            });
        }

        match record.require_owned(keys.own)? {
            Attribute::Null => Ok(Composite {
                content: Default::default(),
                included: Default::default(),
            }),

            value => {
                parameters
                    .filter
                    .get_or_insert_default()
                    .entry(keys.related)
                    .or_default()
                    .push(FilterValue::Equal(value));

                self.fetch_collection(related_schema, &parameters)
            }
        }
    }

    /// Fetches the single full record targeted by the already-loaded `record`'s to-one
    /// `relationship`, or `None` when it is empty. Errors when `relationship` is not declared, or
    /// when it resolves to more than one record.
    pub fn fetch_related_record(
        &self,
        record: &Record<'sch>,
        relationship: &'req str,
        parameters: QueryParameters<'sch, 'req>,
    ) -> Result<Option<CompositeRecord<'sch>>, Error> {
        let kind = record
            .schema
            .relationship(relationship)
            .ok_or_else(|| Error::InvalidRelationshipAccess {
                schema: record.schema.name().into(),
                relationship: relationship.into(),
            })?
            .kind;

        if kind != RelationshipKind::BelongsTo && kind != RelationshipKind::HasOne {
            return Err(Error::MismatchedRelationshipKind {
                schema: record.schema.name().into(),
                relationship: relationship.into(),
            });
        }

        let mut collection = self.fetch_related_collection(record, relationship, parameters)?;

        match collection.content.len() {
            0 => Ok(None),
            1 => Ok(Some(Composite {
                content: collection.content.remove(0),
                included: collection.included,
            })),
            _ => Err(Error::UnexpectedCollection {
                schema: record.schema().name().into(),
                message: "Multiple records resolved for to-one relationship".into(),
            }),
        }
    }

    /// Resolves the to-one `relationship` of the already-loaded `record` to the identifier of the
    /// record it targets, or `None` when the relationship is empty. Errors when `relationship` is
    /// not declared on the record's schema, or when it is to-many — use `peek_related_collection`
    /// for those.
    pub fn peek_related_record(
        &self,
        record: &'req Record<'sch>,
        relationship: &'req str,
    ) -> Result<Option<Identifier>, Error> {
        let descriptor = record.schema.relationship(relationship).ok_or_else(|| {
            Error::InvalidRelationshipAccess {
                schema: record.schema.name().to_string(),
                relationship: relationship.to_string(),
            }
        })?;

        let related_schema = self
            .manager
            .registry()
            .schema(descriptor.related.resource)?;
        let parameters = QueryParameters {
            fields: [(related_schema.name(), Default::default())].into(),
            ..QueryParameters::new(related_schema)
        };
        self.fetch_related_record(record, relationship, parameters)?
            .map(|mut composite| composite.content.pluck_id())
            .transpose()
    }

    /// Resolves `relationship` of the already-loaded `record` to the identifiers of the records it
    /// targets, requesting only their ids. Empty when the relationship is unset. Errors when
    /// `relationship` is not declared on the record's schema.
    pub fn peek_related_collection(
        &self,
        record: &Record<'sch>,
        relationship: &'req str,
    ) -> Result<Vec<Identifier>, Error> {
        let schema = record.schema();
        let descriptor =
            schema
                .relationship(relationship)
                .ok_or_else(|| Error::InvalidRelationshipAccess {
                    schema: schema.name().to_string(),
                    relationship: relationship.to_string(),
                })?;
        let related_schema = self
            .manager
            .registry()
            .schema(descriptor.related.resource)?;
        let parameters = QueryParameters {
            fields: [(related_schema.name(), Default::default())].into(),
            ..QueryParameters::new(related_schema)
        };

        self.fetch_related_collection(record, relationship, parameters)?
            .content
            .into_iter()
            .map(|mut record| record.pluck_id())
            .collect()
    }

    /// Links the to-one `relationship` of `record` to `related_id`, without displacing an existing
    /// target: a `has_one` whose target is already taken surfaces the related side's unique-key
    /// violation. Returns the linked identifier. Errors when the relationship is unknown, to-many,
    /// or the target does not exist.
    pub fn link_record(
        &self,
        record: Record<'sch>,
        relationship_name: &'req str,
        related_id: Identifier,
    ) -> Result<Option<Identifier>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema();
            let descriptor = schema.relationship(relationship_name).ok_or_else(|| {
                Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                }
            })?;

            let mut record = match descriptor.kind {
                RelationshipKind::BelongsTo => {
                    let id = record.require_id()?.clone();
                    let mut record = record.with_relationships(
                        [(descriptor.name, DatabaseRelationship::BelongsTo(related_id))].into(),
                    );
                    self.attach_belongs_to(slice::from_mut(&mut record))?;
                    self.table(schema)?
                        .update(id, record.take_row(), &QueryParameters::new(schema))
                        .map_err(error_mapper::fk_violation_to_missing_reference)?;
                    record
                }
                RelationshipKind::HasOne => {
                    let record = record.with_relationships(
                        [(descriptor.name, DatabaseRelationship::HasOne(related_id))].into(),
                    );
                    self.attach_has_one_many(slice::from_ref(&record), false)?;
                    record
                }
                RelationshipKind::HasMany => {
                    return Err(Error::MismatchedRelationshipKind {
                        schema: schema.name().into(),
                        relationship: relationship_name.into(),
                    });
                }
            };

            let linkage = record
                .relationships
                .remove(descriptor.name)
                .ok_or_else(|| Error::UnloadedRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                })?;
            let (DatabaseRelationship::BelongsTo(related_id)
            | DatabaseRelationship::HasOne(related_id)) = linkage
            else {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            };
            Ok(Some(related_id))
        })
    }

    /// Replaces the to-one `relationship` of `record` with `related_id`, detaching any current
    /// target first. Returns the linked identifier. Errors when the relationship is unknown,
    /// to-many, or the target does not exist.
    pub fn relink_record(
        &self,
        record: Record<'sch>,
        relationship_name: &'req str,
        related_id: Identifier,
    ) -> Result<Option<Identifier>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema();
            let descriptor = schema.relationship(relationship_name).ok_or_else(|| {
                Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                }
            })?;

            let mut record = match descriptor.kind {
                RelationshipKind::BelongsTo => {
                    let id = record.require_id()?.clone();
                    let mut record = record.with_relationships(
                        [(descriptor.name, DatabaseRelationship::BelongsTo(related_id))].into(),
                    );
                    self.attach_belongs_to(slice::from_mut(&mut record))?;
                    self.table(schema)?
                        .update(id, record.take_row(), &QueryParameters::new(schema))
                        .map_err(error_mapper::fk_violation_to_missing_reference)?;
                    record
                }
                RelationshipKind::HasOne => {
                    let record = record.with_relationships(
                        [(descriptor.name, DatabaseRelationship::HasOne(related_id))].into(),
                    );
                    self.attach_has_one_many(slice::from_ref(&record), true)?;
                    record
                }
                RelationshipKind::HasMany => {
                    return Err(Error::MismatchedRelationshipKind {
                        schema: schema.name().into(),
                        relationship: relationship_name.into(),
                    });
                }
            };

            let linkage = record
                .relationships
                .remove(descriptor.name)
                .ok_or_else(|| Error::UnloadedRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                })?;
            let (DatabaseRelationship::BelongsTo(related_id)
            | DatabaseRelationship::HasOne(related_id)) = linkage
            else {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            };
            Ok(Some(related_id))
        })
    }

    /// Clears the to-one `relationship` of `record`, detaching whichever side holds the foreign key.
    /// Returns `None`. Idempotent. Errors when the relationship is unknown or to-many.
    pub fn unlink_record(
        &self,
        record: Record<'sch>,
        relationship_name: &'req str,
    ) -> Result<Option<Identifier>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema();
            let descriptor = schema.relationship(relationship_name).ok_or_else(|| {
                Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                }
            })?;

            match descriptor.kind {
                RelationshipKind::BelongsTo => {
                    let id = record.require_id()?.clone();
                    let mut record = record.with_relationships(
                        [(descriptor.name, DatabaseRelationship::Empty)].into(),
                    );
                    self.attach_belongs_to(slice::from_mut(&mut record))?;
                    self.table(schema)?.update(
                        id,
                        record.take_row(),
                        &QueryParameters::new(schema),
                    )?;
                }
                RelationshipKind::HasOne => {
                    let record = record.with_relationships(
                        [(descriptor.name, DatabaseRelationship::Empty)].into(),
                    );
                    self.attach_has_one_many(slice::from_ref(&record), true)?;
                }
                RelationshipKind::HasMany => {
                    return Err(Error::MismatchedRelationshipKind {
                        schema: schema.name().into(),
                        relationship: relationship_name.into(),
                    });
                }
            }

            Ok(None)
        })
    }

    /// Adds `related_ids` to the to-many `relationship` of `record`, leaving existing members in
    /// place, and returns the resulting membership. Errors when the relationship is unknown, to-one,
    /// or any target does not exist.
    pub fn link_collection(
        &self,
        record: Record<'sch>,
        relationship_name: &'req str,
        related_ids: Vec<Identifier>,
    ) -> Result<Vec<Identifier>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema();
            let descriptor = schema.relationship(relationship_name).ok_or_else(|| {
                Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                }
            })?;

            if descriptor.kind != RelationshipKind::HasMany {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            }

            let record = record.with_relationships(
                [(descriptor.name, DatabaseRelationship::HasMany(related_ids))].into(),
            );
            self.attach_has_one_many(slice::from_ref(&record), false)?;

            self.peek_related_collection(&record, relationship_name)
        })
    }

    /// Replaces the to-many `relationship` of `record` with `related_ids`, detaching any members not
    /// in the set, and returns the resulting membership. Errors when the relationship is unknown,
    /// to-one, or any target does not exist.
    pub fn relink_collection(
        &self,
        record: Record<'sch>,
        relationship_name: &'req str,
        related_ids: Vec<Identifier>,
    ) -> Result<Vec<Identifier>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema();
            let descriptor = schema.relationship(relationship_name).ok_or_else(|| {
                Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                }
            })?;

            if descriptor.kind != RelationshipKind::HasMany {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            }

            let mut record = record.with_relationships(
                [(descriptor.name, DatabaseRelationship::HasMany(related_ids))].into(),
            );
            self.attach_has_one_many(slice::from_ref(&record), true)?;

            let linkage = record
                .relationships
                .remove(descriptor.name)
                .ok_or_else(|| Error::UnloadedRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                })?;
            let DatabaseRelationship::HasMany(related_ids) = linkage else {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            };
            Ok(related_ids)
        })
    }

    /// Removes `related_ids` from the to-many `relationship` of `record`, and returns the resulting
    /// membership. Peeks the current members, then replaces with the remainder — so already-absent
    /// members are a no-op. Errors when the relationship is unknown or to-one.
    pub fn unlink_collection(
        &self,
        record: Record<'sch>,
        relationship_name: &'req str,
        related_ids: Vec<Identifier>,
    ) -> Result<Vec<Identifier>, Error> {
        self.connection.transaction(|| {
            let schema = record.schema();
            let descriptor = schema.relationship(relationship_name).ok_or_else(|| {
                Error::InvalidRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                }
            })?;

            if descriptor.kind != RelationshipKind::HasMany {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            }

            let remaining: Vec<Identifier> = self
                .peek_related_collection(&record, relationship_name)?
                .into_iter()
                .filter(|id| !related_ids.contains(id))
                .collect();

            let mut record = record.with_relationships(
                [(descriptor.name, DatabaseRelationship::HasMany(remaining))].into(),
            );
            self.attach_has_one_many(slice::from_ref(&record), true)?;

            let linkage = record
                .relationships
                .remove(descriptor.name)
                .ok_or_else(|| Error::UnloadedRelationshipAccess {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                })?;
            let DatabaseRelationship::HasMany(remaining) = linkage else {
                return Err(Error::MismatchedRelationshipKind {
                    schema: schema.name().into(),
                    relationship: relationship_name.into(),
                });
            };
            Ok(remaining)
        })
    }

    fn table(&self, schema: &'sch Schema<'sch>) -> Result<Adapter::Table<'sch, 'req>, Error> {
        self.manager.table(schema.name(), self.connection)
    }

    fn loader(&self) -> DataLoader<'sch, 'req, Adapter> {
        DataLoader::new(self.manager, self.connection)
    }

    /// Populates each record's `foreign_keys` with whatever `belongs_to` relationships
    /// are specified.
    /// This prepares the records for inserting or updating and must be called prior to any
    /// of these operations.
    fn attach_belongs_to(&self, records: &mut [Record<'sch>]) -> Result<(), Error> {
        let mut required_queries = HashMap::new();

        for record in records.iter_mut() {
            let schema = record.schema();
            for (&name, linkage) in &record.relationships {
                let descriptor =
                    schema
                        .relationship(name)
                        .ok_or_else(|| Error::ResourceValidationFailure {
                            schema: schema.name().to_string(),
                            attribute: name.to_string(),
                            message: "Attempted to attach unknown relationship".to_string(),
                        })?;
                let name = descriptor.name;

                if descriptor.kind == RelationshipKind::BelongsTo {
                    let related = &descriptor.related;
                    match linkage {
                        DatabaseRelationship::BelongsTo(id) => {
                            let related_table = self.manager.registry().schema(related.resource)?;
                            if related_table.is_primary_key(related.keys.related) {
                                record
                                    .foreign_keys
                                    .insert(related.keys.own, id.clone().into());
                            } else {
                                let (_, attributes, ids, relationships) = required_queries
                                    .entry(related_table.name())
                                    .or_insert_with(|| {
                                        (
                                            related_table,
                                            IndexSet::new(),
                                            IndexSet::new(),
                                            HashMap::new(),
                                        )
                                    });
                                attributes.insert(related.keys.related);
                                ids.insert(id.clone().into());
                                relationships.insert(name, related);
                            }
                        }
                        DatabaseRelationship::Empty => {
                            record
                                .foreign_keys
                                .insert(related.keys.own, Attribute::Null);
                        }
                        _ => {
                            return Err(Error::ResourceValidationFailure {
                                schema: schema.name().to_string(),
                                attribute: name.to_string(),
                                message: "Attempted to attach relationship with wrong linkage"
                                    .to_string(),
                            });
                        }
                    }
                }
            }
        }

        for (related_table, attributes, ids, relationships) in required_queries.into_values() {
            let index = self
                .table(related_table)?
                .query(&QueryParameters {
                    fields: IndexMap::from([(related_table.name(), attributes)]),
                    filter: Some(FilterParameters::from([(
                        related_table.primary_key().name,
                        vec![FilterValue::In(ids)],
                    )])),
                    ..QueryParameters::new(related_table)
                })?
                .into_iter()
                .map(|row| Record::try_from_row(related_table, row))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .try_index_by(|record| Ok::<_, Error>(record.require_id()?.clone()))?;

            for (relationship, related) in relationships {
                for record in records.iter_mut() {
                    if let Some(DatabaseRelationship::BelongsTo(id)) =
                        record.relationships.get(relationship)
                    {
                        let related_record = index.get(id).ok_or(Error::RelatedRecordNotFound)?;
                        let value = related_record.require(related.keys.related).cloned()?;
                        record.foreign_keys.insert(related.keys.own, value);
                    }
                }
            }
        }

        Ok(())
    }

    /// Writes the `has_one` and `has_many` linkages of `records` by setting the foreign key on the
    /// related side. With `replace`, detaches the related records no longer in each set before
    /// setting the new members. Errors with `RelatedRecordNotFound` when a targeted related record
    /// does not exist, or `RecordNotFound` when the owning key is dangling (the primary is missing).
    fn attach_has_one_many(&self, records: &[Record<'sch>], replace: bool) -> Result<(), Error> {
        use DatabaseRelationship as Data;
        use RelationshipKind as Kind;
        let mut patches: HashMap<&str, HashMap<Attribute, Row<'sch>>> = HashMap::new();
        let mut full_detachments: HashMap<&str, HashMap<&str, IndexSet<_>>> = HashMap::new();

        for record in records.iter() {
            let schema = record.schema;
            for (name, relationship) in &record.relationships {
                let descriptor =
                    schema
                        .relationship(name)
                        .ok_or_else(|| Error::ResourceValidationFailure {
                            schema: schema.name().to_string(),
                            attribute: name.to_string(),
                            message: "Attempted to attach unknown relationship".to_string(),
                        })?;
                let related = &descriptor.related;

                let ids = match (&relationship, descriptor.kind) {
                    (Data::Empty, Kind::HasOne | Kind::HasMany) => [].as_slice(),
                    (Data::HasOne(id), Kind::HasOne) => slice::from_ref(id),
                    (Data::HasMany(ids), Kind::HasMany) => ids.as_slice(),
                    (Data::HasOne(..) | Data::HasMany(..), _) => {
                        Err(Error::ResourceValidationFailure {
                            schema: schema.name().to_string(),
                            attribute: name.to_string(),
                            message: "Attempted to attach relationship with wrong linkage"
                                .to_string(),
                        })?
                    }
                    _ => continue,
                };

                let value = record.require_owned(related.keys.own)?;
                if !ids.is_empty() {
                    for id in ids {
                        patches
                            .entry(related.resource)
                            .or_default()
                            .entry(id.clone().into())
                            .or_default()
                            .insert(related.keys.related, value.clone());
                    }
                } else {
                    full_detachments
                        .entry(related.resource)
                        .or_default()
                        .entry(related.keys.related)
                        .or_default()
                        .insert(value);
                }
            }
        }

        let queries = patches.into_iter().map(|(table, patches)| {
            (
                table,
                patches.into_iter().fold(
                    HashMap::new(),
                    |mut map: HashMap<Vec<_>, IndexSet<_>>, (id, patch)| {
                        let key = patch
                            .into_iter()
                            .sorted_by(|a, b| Ord::cmp(a.0, b.0))
                            .collect_vec();
                        map.entry(key).or_default().insert(id);
                        map
                    },
                ),
            )
        });

        if replace {
            for (schema, columns) in full_detachments {
                let schema = self.manager.registry().schema(schema)?;
                let table = self.table(schema)?;

                for (column, values) in columns {
                    table.update_batch(
                        Row::from([(column, Attribute::Null)]),
                        &QueryParameters {
                            filter: Some(
                                [(column, vec![FilterValue::In(values)])]
                                    .into_iter()
                                    .collect(),
                            ),
                            ..QueryParameters::new(schema)
                        },
                    )?;
                }
            }
        }

        for (name, patches) in queries {
            let schema = self.manager.registry().schema(name)?;
            let table = self.table(schema)?;

            if replace {
                let complement = patches.iter().fold(
                    HashMap::<(&str, Attribute), IndexSet<Attribute>>::new(),
                    |mut complement, (patch, ids)| {
                        for (column, value) in patch {
                            complement
                                .entry((*column, value.clone()))
                                .or_default()
                                .extend(ids.iter().cloned());
                        }
                        complement
                    },
                );

                for ((column, value), ids) in complement {
                    table.update_batch(
                        Row::from([(column, Attribute::Null)]),
                        &QueryParameters {
                            filter: Some(FilterParameters::from([
                                (column, vec![FilterValue::Equal(value)]),
                                (schema.primary_key().name, vec![FilterValue::NotIn(ids)]),
                            ])),
                            ..QueryParameters::new(schema)
                        },
                    )?;
                }
            }

            for (patch, ids) in &patches {
                let attached = table
                    .update_batch(
                        Row::from_iter(patch.clone()),
                        &QueryParameters {
                            filter: Some(FilterParameters::from([(
                                schema.primary_key().name,
                                vec![FilterValue::In(ids.clone())],
                            )])),
                            ..QueryParameters::new(schema)
                        },
                    )
                    .map_err(error_mapper::fk_violation_to_missing_record)?
                    .into_iter()
                    .map(|mut row| {
                        row.shift_remove(schema.primary_key().name).ok_or_else(|| {
                            Error::MissingRecordId {
                                schema: schema.name().into(),
                            }
                        })
                    })
                    .collect::<Result<IndexSet<Attribute>, _>>()?;

                if attached != *ids {
                    return Err(Error::RelatedRecordNotFound);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;

    use super::Store;
    use crate::database::adapters::SqliteAdapter;
    use crate::database::adapters::sqlite::{Connection, Pool};
    use crate::database::attributes::{Attribute, Attributes, Identifier, Row};
    use crate::database::connection_manager::ConnectionManager;
    use crate::database::error::{ConstraintKind, Error};
    use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
    use crate::database::record::{Builder, Record, RecordPatch};
    use crate::database::registry::Registry;
    use crate::database::relationships::{Relationship, Relationships};
    use crate::database::schema::{AttributeType, Related, Schema, SchemaBuilder};
    use crate::database::table::Table;
    use crate::http_wrappers::Uri;
    use std::collections::HashMap;
    use std::error::Error as StdError;
    use test_log::test;

    fn users_schema() -> SchemaBuilder<'static> {
        SchemaBuilder::table("users")
            .attribute("name", AttributeType::Text)
            .has_many(
                "posts",
                Related::to("posts")
                    .pointing_related("author_id")
                    .to_own("id"),
            )
            .has_one(
                "profile",
                Related::to("profiles")
                    .pointing_related("user_id")
                    .to_own("id"),
            )
    }

    fn posts_schema() -> SchemaBuilder<'static> {
        SchemaBuilder::table("posts")
            .attribute("title", AttributeType::Text)
            .foreign_key("author_id", AttributeType::Integer)
            .belongs_to(
                "author",
                Related::to("users")
                    .pointing_own("author_id")
                    .to_related("id"),
            )
    }

    fn profiles_schema() -> SchemaBuilder<'static> {
        SchemaBuilder::table("profiles")
            .attribute("bio", AttributeType::Text)
            .foreign_key("user_id", AttributeType::Integer)
            .belongs_to(
                "user",
                Related::to("users")
                    .pointing_own("user_id")
                    .to_related("id"),
            )
    }

    // A 1:1 pair keyed on a non-primary-key column (`orgs.code`), exercising the
    // relationship branches whose own/related key is not the primary key.
    fn orgs_schema() -> SchemaBuilder<'static> {
        SchemaBuilder::table("orgs")
            .attribute("code", AttributeType::Text)
            .has_one(
                "member",
                Related::to("members")
                    .pointing_related("org_code")
                    .to_own("code"),
            )
    }

    fn members_schema() -> SchemaBuilder<'static> {
        SchemaBuilder::table("members")
            .attribute("handle", AttributeType::Text)
            .foreign_key("org_code", AttributeType::Text)
            .belongs_to(
                "org",
                Related::to("orgs")
                    .pointing_own("org_code")
                    .to_related("code"),
            )
    }

    fn schema<'sch>(
        manager: &'sch ConnectionManager<SqliteAdapter>,
        name: &str,
    ) -> &'sch Schema<'sch> {
        manager
            .registry()
            .schema(name)
            .expect("schema is registered")
    }

    fn with_manager<F>(func: F) -> Result<(), Box<dyn StdError>>
    where
        F: FnOnce(&ConnectionManager<SqliteAdapter>) -> Result<(), Box<dyn StdError>>,
    {
        let manager: ConnectionManager<SqliteAdapter> = ConnectionManager::new(
            Registry::try_new([
                users_schema(),
                posts_schema(),
                profiles_schema(),
                orgs_schema(),
                members_schema(),
            ])?,
            Pool::memory()?,
        );

        manager.acquire()?.execute_batch(
            "
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );

            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                author_id INTEGER,
                title TEXT NOT NULL,
                FOREIGN KEY(author_id) REFERENCES users(id)
            );

            CREATE TABLE profiles (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL UNIQUE,
                bio TEXT,
                FOREIGN KEY(user_id) REFERENCES users(id)
            );

            CREATE TABLE orgs (
                id INTEGER PRIMARY KEY,
                code TEXT NOT NULL UNIQUE
            );

            CREATE TABLE members (
                id INTEGER PRIMARY KEY,
                handle TEXT NOT NULL,
                org_code TEXT UNIQUE,
                FOREIGN KEY(org_code) REFERENCES orgs(code)
            );
            ",
        )?;

        func(&manager)
    }

    fn seed_user(
        manager: &ConnectionManager<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        name: &str,
    ) -> Result<(), Error> {
        manager.table("users", connection)?.insert(
            Row::from_iter([
                ("id", Attribute::Integer(id)),
                ("name", Attribute::Text(name.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "users")),
        )?;

        Ok(())
    }

    fn seed_post(
        manager: &ConnectionManager<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        author_id: i64,
        title: &str,
    ) -> Result<(), Error> {
        manager.table("posts", connection)?.insert(
            Row::from_iter([
                ("id", Attribute::Integer(id)),
                ("author_id", Attribute::Integer(author_id)),
                ("title", Attribute::Text(title.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "posts")),
        )?;

        Ok(())
    }

    fn seed_profile(
        manager: &ConnectionManager<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        user_id: i64,
        bio: &str,
    ) -> Result<(), Error> {
        manager.table("profiles", connection)?.insert(
            Row::from_iter([
                ("id", Attribute::Integer(id)),
                ("user_id", Attribute::Integer(user_id)),
                ("bio", Attribute::Text(bio.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "profiles")),
        )?;

        Ok(())
    }

    fn seed_org(
        manager: &ConnectionManager<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        code: &str,
    ) -> Result<(), Error> {
        manager.table("orgs", connection)?.insert(
            Row::from_iter([
                ("id", Attribute::Integer(id)),
                ("code", Attribute::Text(code.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "orgs")),
        )?;

        Ok(())
    }

    fn seed_member(
        manager: &ConnectionManager<SqliteAdapter>,
        connection: &Connection,
        id: i64,
        handle: &str,
        org_code: &str,
    ) -> Result<(), Error> {
        manager.table("members", connection)?.insert(
            Row::from_iter([
                ("id", Attribute::Integer(id)),
                ("handle", Attribute::Text(handle.to_string())),
                ("org_code", Attribute::Text(org_code.to_string())),
            ]),
            &QueryParameters::new(schema(manager, "members")),
        )?;

        Ok(())
    }

    fn new_post<'sch>(
        manager: &'sch ConnectionManager<SqliteAdapter>,
        title: &str,
        author: i64,
    ) -> Record<'sch> {
        Record::from((
            schema(manager, "posts"),
            Attributes::from_iter([("title", Attribute::Text(title.to_string()))]),
            Relationships::from_iter([(
                "author",
                Relationship::BelongsTo(Identifier::Integer(author)),
            )]),
        ))
    }

    // --- fetch_record ------------------------------------------------------

    #[test]
    fn test_fetch_record_returns_content() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "hello")?;

            let store = Store::new(manager, &connection);
            let parameters = QueryParameters::new(schema(manager, "posts"));
            let fetched = store.fetch_record(
                schema(manager, "posts"),
                Identifier::Integer(1),
                &parameters,
            )?;

            assert_eq!(fetched.content.require_id()?.to_i64()?, 1);
            assert_eq!(fetched.content.require("title")?.as_string()?, "hello");
            assert_eq!(
                fetched.content.require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(1))
            );
            assert!(fetched.included.is_empty());

            Ok(())
        })
    }

    #[test]
    fn test_fetch_record_loads_includes() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "hello")?;

            let store = Store::new(manager, &connection);
            let uri: Uri = "/posts/1?include=author".parse()?;
            let parameters =
                QueryParameters::parse(&uri, schema(manager, "posts"), manager.registry())?;
            let fetched = store.fetch_record(
                schema(manager, "posts"),
                Identifier::Integer(1),
                &parameters,
            )?;

            assert_eq!(fetched.included.len(), 1);
            assert_eq!(fetched.included[0].schema.name(), "users");
            assert_eq!(fetched.included[0].require_id()?, &Identifier::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_fetch_record_missing_is_not_found() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            let store = Store::new(manager, &connection);

            let parameters = QueryParameters::new(schema(manager, "posts"));
            let result = store.fetch_record(
                schema(manager, "posts"),
                Identifier::Integer(999),
                &parameters,
            );

            assert!(matches!(result, Err(Error::RecordNotFound)));

            Ok(())
        })
    }

    // --- fetch_collection --------------------------------------------------

    #[test]
    fn test_fetch_collection_returns_all_records() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "one")?;
            seed_post(manager, &connection, 2, 1, "two")?;
            seed_post(manager, &connection, 3, 1, "three")?;

            let store = Store::new(manager, &connection);
            let parameters = QueryParameters::new(schema(manager, "posts"));
            let fetched = store.fetch_collection(schema(manager, "posts"), &parameters)?;

            assert_eq!(fetched.content.len(), 3);

            Ok(())
        })
    }

    #[test]
    fn test_fetch_collection_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 1, 1, "alice-one")?;
            seed_post(manager, &connection, 2, 1, "alice-two")?;
            seed_post(manager, &connection, 3, 2, "bob-one")?;

            let store = Store::new(manager, &connection);
            let parameters = QueryParameters {
                filter: Some(FilterParameters::from([(
                    "author_id",
                    vec![FilterValue::Equal(Attribute::Integer(1))],
                )])),
                ..QueryParameters::new(schema(manager, "posts"))
            };

            let fetched = store.fetch_collection(schema(manager, "posts"), &parameters)?;

            assert_eq!(fetched.content.len(), 2);
            for record in &fetched.content {
                assert_eq!(
                    record.require_related("author")?,
                    &Relationship::BelongsTo(Identifier::Integer(1))
                );
            }

            Ok(())
        })
    }

    #[test]
    fn test_fetch_collection_loads_includes() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 1, 1, "alice-one")?;
            seed_post(manager, &connection, 2, 2, "bob-one")?;

            let store = Store::new(manager, &connection);
            let uri: Uri = "/posts?include=author".parse()?;
            let parameters =
                QueryParameters::parse(&uri, schema(manager, "posts"), manager.registry())?;
            let fetched = store.fetch_collection(schema(manager, "posts"), &parameters)?;

            assert_eq!(fetched.content.len(), 2);
            assert_eq!(fetched.included.len(), 2);
            assert!(
                fetched
                    .included
                    .iter()
                    .all(|record| record.schema.name() == "users")
            );

            Ok(())
        })
    }

    // --- create_record -----------------------------------------------------

    #[test]
    fn test_create_record_persists_attributes_and_belongs_to() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let parameters = QueryParameters::new(schema(manager, "posts"));
            let created = store.create_record(new_post(manager, "Hello", 1), &parameters)?;

            assert_eq!(created.content.require("title")?.as_string()?, "Hello");
            assert_eq!(
                created.content.require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(1))
            );

            let persisted = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(persisted.len(), 1);
            assert_eq!(persisted[0]["author_id"], Attribute::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_create_record_links_has_many() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "one")?;
            seed_post(manager, &connection, 2, 1, "two")?;

            let store = Store::new(manager, &connection);
            let attributes = Attributes::from_iter([("name", Attribute::Text("dave".to_string()))]);
            let relationships = Relationships::from_iter([(
                "posts",
                Relationship::HasMany(vec![Identifier::Integer(1), Identifier::Integer(2)]),
            )]);
            let user = Record::from((schema(manager, "users"), attributes, relationships));

            let parameters = QueryParameters::new(schema(manager, "users"));
            let created = store.create_record(user, &parameters)?;
            let new_id = *created.content.require_id()?.as_i64()?;

            let posts = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(post["author_id"], Attribute::Integer(new_id));
            }

            Ok(())
        })
    }

    #[test]
    fn test_create_record_invalid_belongs_to_is_related_not_found() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;

            let store = Store::new(manager, &connection);
            // `author` 999 is provided but references no user: a missing reference (404).
            let parameters = QueryParameters::new(schema(manager, "posts"));
            let result = store.create_record(new_post(manager, "Orphan", 999), &parameters);

            assert!(matches!(result, Err(Error::RelatedRecordNotFound)));

            Ok(())
        })
    }

    #[test]
    fn test_create_record_absent_required_belongs_to_is_a_constraint_violation()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;

            let store = Store::new(manager, &connection);
            // profiles.user_id is NOT NULL: an absent `user` leaves it null, which is a NOT NULL
            // violation -- not a missing reference, so it must not be recast to a 404.
            let profile = Record::from_attributes(
                schema(manager, "profiles"),
                Attributes::from_iter([("bio", Attribute::Text("no user".to_string()))]),
            );
            let parameters = QueryParameters::new(schema(manager, "profiles"));
            let result = store.create_record(profile, &parameters);

            assert!(matches!(
                result,
                Err(Error::ConstraintViolation {
                    kind: ConstraintKind::NotNull,
                    ..
                })
            ));

            Ok(())
        })
    }

    #[test]
    fn test_create_record_absent_optional_belongs_to_persists_null() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;

            let store = Store::new(manager, &connection);
            // posts.author_id is nullable: an absent `author` is accepted as a null key.
            let post = Record::from_attributes(
                schema(manager, "posts"),
                Attributes::from_iter([("title", Attribute::Text("no author".to_string()))]),
            );
            let parameters = QueryParameters::new(schema(manager, "posts"));
            store.create_record(post, &parameters)?;

            let persisted = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(persisted.len(), 1);
            assert_eq!(persisted[0]["author_id"], Attribute::Null);

            Ok(())
        })
    }

    #[test]
    fn test_create_record_honours_client_generated_id() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;

            let store = Store::new(manager, &connection);
            // 42 is an id the autoincrement would never assign the first row, so persisting it
            // proves the client-supplied id was written rather than generated.
            let user = Record::from_attributes(
                schema(manager, "users"),
                Attributes::from_iter([("name", Attribute::Text("alice".to_string()))]),
            )
            .with_id(Identifier::Integer(42).into());

            let parameters = QueryParameters::new(schema(manager, "users"));
            let created = store.create_record(user, &parameters)?;
            assert_eq!(created.content.require_id()?, &Identifier::Integer(42));

            let persisted = manager
                .table("users", &connection)?
                .query(&QueryParameters::new(schema(manager, "users")))?;
            assert_eq!(persisted.len(), 1);
            assert_eq!(persisted[0]["id"], Attribute::Integer(42));

            Ok(())
        })
    }

    // --- update_record -----------------------------------------------------

    #[test]
    fn test_update_record_updates_attributes() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "before")?;

            let store = Store::new(manager, &connection);
            let record = Record::from_attributes(
                schema(manager, "posts"),
                Attributes::from_iter([("title", Attribute::Text("after".to_string()))]),
            )
            .with_id(Identifier::Integer(1).into());

            let parameters = QueryParameters::new(schema(manager, "posts"));
            store.update_record(record, &parameters)?;

            let posts = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0]["title"], Attribute::Text("after".to_string()));

            Ok(())
        })
    }

    #[test]
    fn test_update_record_replaces_has_many() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 1, 1, "p1")?;
            seed_post(manager, &connection, 2, 1, "p2")?;
            seed_post(manager, &connection, 3, 2, "p3")?;

            let store = Store::new(manager, &connection);

            // Reassign bob's posts to exactly {p1}: p1 is adopted, p3 (bob's) is detached.
            let record = Record::from_relationships(
                schema(manager, "users"),
                Relationships::from([(
                    "posts",
                    Relationship::HasMany(vec![Identifier::Integer(1)]),
                )]),
            )
            .with_id(Some(Identifier::Integer(2)));
            store.update_record(record, &QueryParameters::new(schema(manager, "users")))?;

            let posts: HashMap<Attribute, Row> = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?
                .into_iter()
                .map(|row| (row["id"].clone(), row))
                .collect();

            assert_eq!(
                posts[&Attribute::Integer(1)]["author_id"],
                Attribute::Integer(2)
            );
            assert_eq!(
                posts[&Attribute::Integer(2)]["author_id"],
                Attribute::Integer(1)
            );
            assert_eq!(posts[&Attribute::Integer(3)]["author_id"], Attribute::Null);

            Ok(())
        })
    }

    #[test]
    fn test_update_record_invalid_belongs_to_is_related_not_found() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "before")?;

            let store = Store::new(manager, &connection);
            let record = Record::from_relationships(
                schema(manager, "posts"),
                Relationships::from([(
                    "author",
                    Relationship::BelongsTo(Identifier::Integer(999)),
                )]),
            )
            .with_id(Some(Identifier::Integer(1)));

            let parameters = QueryParameters::new(schema(manager, "posts"));
            let result = store.update_record(record, &parameters);

            assert!(matches!(result, Err(Error::RelatedRecordNotFound)));

            Ok(())
        })
    }

    // --- delete_record -----------------------------------------------------

    #[test]
    fn test_delete_record_removes_row() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "doomed")?;

            let store = Store::new(manager, &connection);
            store.delete_record(schema(manager, "posts"), Identifier::Integer(1))?;

            assert!(
                manager
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(schema(manager, "posts")))?
                    .is_empty()
            );

            Ok(())
        })
    }

    // --- create_collection -------------------------------------------------

    #[test]
    fn test_create_collection_inserts_records_with_belongs_to() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let created = store.create_collection(
                vec![
                    new_post(manager, "First", 1),
                    new_post(manager, "Second", 1),
                ],
                &QueryParameters::new(schema(manager, "posts")),
            )?;

            assert_eq!(created.content.len(), 2);
            for record in &created.content {
                assert_eq!(
                    record.require_related("author")?,
                    &Relationship::BelongsTo(Identifier::Integer(1))
                );
            }

            let mut titles = created
                .content
                .iter()
                .map(|record| record.require("title")?.as_string().map(String::as_str))
                .collect::<Result<Vec<_>, Error>>()?;
            titles.sort_unstable();
            assert_eq!(titles, ["First", "Second"]);

            assert_eq!(
                manager
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(schema(manager, "posts")))?
                    .len(),
                2
            );

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_assigns_distinct_belongs_to_per_record()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;

            let store = Store::new(manager, &connection);
            let posts: HashMap<String, Record> = store
                .create_collection(
                    vec![
                        new_post(manager, "alice-post", 1),
                        new_post(manager, "bob-post", 2),
                    ],
                    &QueryParameters::new(schema(manager, "posts")),
                )?
                .content
                .into_iter()
                .map(|post| -> Result<_, Box<dyn StdError>> {
                    Ok((post.require("title")?.as_string()?.clone(), post))
                })
                .try_collect()?;

            assert_eq!(
                posts
                    .get("alice-post")
                    .expect("Alice's post should be in the index")
                    .require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(1))
            );
            assert_eq!(
                posts
                    .get("bob-post")
                    .expect("Bob's post should be in the index")
                    .require_related("author")?,
                &Relationship::BelongsTo(Identifier::Integer(2))
            );

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_loads_includes() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let uri: Uri = "/posts?include=author".parse()?;
            let parameters =
                QueryParameters::parse(&uri, schema(manager, "posts"), manager.registry())?;
            let created = store.create_collection(
                vec![
                    new_post(manager, "First", 1),
                    new_post(manager, "Second", 1),
                ],
                &parameters,
            )?;

            assert_eq!(created.included.len(), 1);
            assert_eq!(created.included[0].schema.name(), "users");
            assert_eq!(created.included[0].id, Some(Identifier::Integer(1)));

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_empty_is_a_noop() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            let store = Store::new(manager, &connection);

            let parameters = QueryParameters::new(schema(manager, "posts"));
            let created = store.create_collection(vec![], &parameters)?;

            assert!(created.content.is_empty());
            assert!(created.included.is_empty());
            assert!(
                manager
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(schema(manager, "posts")))?
                    .is_empty()
            );

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_links_has_many() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "one")?;
            seed_post(manager, &connection, 2, 1, "two")?;

            let store = Store::new(manager, &connection);
            let user = Record::from((
                schema(manager, "users"),
                Attributes::from_iter([("name", Attribute::Text("dave".to_string()))]),
                Relationships::from_iter([(
                    "posts",
                    Relationship::HasMany(vec![Identifier::Integer(1), Identifier::Integer(2)]),
                )]),
            ));
            let created = store
                .create_collection(vec![user], &QueryParameters::new(schema(manager, "users")))?;
            let new_id = *created.content[0].require_id()?.as_i64()?;

            let posts = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(post["author_id"], Attribute::Integer(new_id));
            }

            Ok(())
        })
    }

    #[test]
    fn test_create_collection_links_has_one() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_profile(manager, &connection, 1, 1, "alice's profile")?;

            let store = Store::new(manager, &connection);
            let user = Record::from((
                schema(manager, "users"),
                Attributes::from_iter([("name", Attribute::Text("dave".to_string()))]),
                Relationships::from_iter([(
                    "profile",
                    Relationship::HasOne(Identifier::Integer(1)),
                )]),
            ));
            let created = store
                .create_collection(vec![user], &QueryParameters::new(schema(manager, "users")))?;
            let new_id = *created.content[0].require_id()?.as_i64()?;

            let profile = manager.table("profiles", &connection)?.find(
                Identifier::Integer(1),
                &QueryParameters::new(schema(manager, "profiles")),
            )?;
            assert_eq!(profile["user_id"], Attribute::Integer(new_id));

            Ok(())
        })
    }

    // --- update_collection -------------------------------------------------

    #[test]
    fn test_update_collection_uniform_attribute_patch_scoped_by_filter()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 1, 1, "alice-one")?;
            seed_post(manager, &connection, 2, 1, "alice-two")?;
            seed_post(manager, &connection, 3, 2, "bob-one")?;

            let store = Store::new(manager, &connection);

            let patch = RecordPatch::from_attributes(
                schema(manager, "posts"),
                Attributes::from_iter([("title", Attribute::Text("patched".to_string()))]),
            );

            let parameters = QueryParameters {
                filter: Some(FilterParameters::from([(
                    "author_id",
                    vec![FilterValue::Equal(Attribute::Integer(1))],
                )])),
                ..QueryParameters::new(schema(manager, "posts"))
            };

            let updated = store.update_collection(patch, &parameters)?;
            assert_eq!(updated.content.len(), 2);

            let posts = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            for post in &posts {
                let expected = if post["author_id"] == Attribute::Integer(1) {
                    "patched"
                } else {
                    "bob-one"
                };
                assert_eq!(post["title"], Attribute::Text(expected.to_string()));
            }

            Ok(())
        })
    }

    #[test]
    fn test_update_collection_bulk_reassigns_belongs_to() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 1, 1, "one")?;
            seed_post(manager, &connection, 2, 1, "two")?;

            let store = Store::new(manager, &connection);

            let patch = RecordPatch::from_relationships(
                schema(manager, "posts"),
                Relationships::from_iter([(
                    "author",
                    Relationship::BelongsTo(Identifier::Integer(2)),
                )]),
            );

            let parameters = QueryParameters::new(schema(manager, "posts"));
            store.update_collection(patch, &parameters)?;

            let posts = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(posts.len(), 2);
            for post in &posts {
                assert_eq!(post["author_id"], Attribute::Integer(2));
            }

            Ok(())
        })
    }

    // --- delete_collection -------------------------------------------------

    #[test]
    fn test_delete_collection_scoped_by_filter() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 1, 1, "alice-one")?;
            seed_post(manager, &connection, 2, 1, "alice-two")?;
            seed_post(manager, &connection, 3, 2, "bob-one")?;

            let store = Store::new(manager, &connection);

            let parameters = QueryParameters {
                filter: Some(FilterParameters::from([(
                    "author_id",
                    vec![FilterValue::Equal(Attribute::Integer(1))],
                )])),
                ..QueryParameters::new(schema(manager, "posts"))
            };

            store.delete_collection(schema(manager, "posts"), &parameters)?;

            let posts = manager
                .table("posts", &connection)?
                .query(&QueryParameters::new(schema(manager, "posts")))?;
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0]["title"], Attribute::Text("bob-one".to_string()));

            Ok(())
        })
    }

    #[test]
    fn test_delete_collection_unscoped_clears_table() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "one")?;
            seed_post(manager, &connection, 2, 1, "two")?;

            let store = Store::new(manager, &connection);
            let parameters = QueryParameters::new(schema(manager, "posts"));
            store.delete_collection(schema(manager, "posts"), &parameters)?;

            assert!(
                manager
                    .table("posts", &connection)?
                    .query(&QueryParameters::new(schema(manager, "posts")))?
                    .is_empty()
            );

            Ok(())
        })
    }

    // --- peek_related_collection -------------------------------------------

    #[test]
    fn test_peek_related_collection_has_many_returns_all_ids() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 1, 1, "one")?;
            seed_post(manager, &connection, 2, 1, "two")?;
            seed_post(manager, &connection, 3, 1, "three")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            let mut ids = store
                .peek_related_collection(&user, "posts")?
                .iter()
                .map(|id| id.to_i64())
                .collect::<Result<Vec<_>, _>>()?;
            ids.sort();

            assert_eq!(ids, vec![1, 2, 3]);

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_collection_has_many_empty_when_none() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(store.peek_related_collection(&user, "posts")?.is_empty());

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_collection_belongs_to_returns_single_id() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 7, "alice")?;
            seed_post(manager, &connection, 1, 7, "one")?;

            let store = Store::new(manager, &connection);
            let posts = schema(manager, "posts");
            let post = store
                .fetch_record(posts, Identifier::Integer(1), &QueryParameters::new(posts))?
                .content;

            assert_eq!(
                store.peek_related_collection(&post, "author")?,
                vec![Identifier::Integer(7)]
            );

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_collection_null_foreign_key_is_empty() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            let posts = schema(manager, "posts");
            manager.table("posts", &connection)?.insert(
                Row::from_iter([
                    ("id", Attribute::Integer(1)),
                    ("author_id", Attribute::Null),
                    ("title", Attribute::Text("orphan".to_string())),
                ]),
                &QueryParameters::new(posts),
            )?;

            let store = Store::new(manager, &connection);
            let post = store
                .fetch_record(posts, Identifier::Integer(1), &QueryParameters::new(posts))?
                .content;

            assert!(store.peek_related_collection(&post, "author")?.is_empty());

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_collection_unknown_relationship_is_error() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.peek_related_collection(&user, "ghost"),
                Err(Error::InvalidRelationshipAccess { .. })
            ));

            Ok(())
        })
    }

    // --- peek_related_record -----------------------------------------------

    #[test]
    fn test_peek_related_record_belongs_to_returns_id() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 7, "alice")?;
            seed_post(manager, &connection, 1, 7, "one")?;

            let store = Store::new(manager, &connection);
            let posts = schema(manager, "posts");
            let post = store
                .fetch_record(posts, Identifier::Integer(1), &QueryParameters::new(posts))?
                .content;

            assert_eq!(
                store.peek_related_record(&post, "author")?,
                Some(Identifier::Integer(7))
            );

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_record_has_one_returns_id() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_profile(manager, &connection, 5, 1, "hi")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert_eq!(
                store.peek_related_record(&user, "profile")?,
                Some(Identifier::Integer(5))
            );

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_record_empty_is_none() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert_eq!(store.peek_related_record(&user, "profile")?, None);

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_record_on_to_many_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.peek_related_record(&user, "posts"),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }

    #[test]
    fn test_peek_related_record_unknown_relationship_is_error() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.peek_related_record(&user, "ghost"),
                Err(Error::InvalidRelationshipAccess { .. })
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_belongs_to_sets_foreign_key_and_returns_related_id()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 10, 1, "hello")?;

            let store = Store::new(manager, &connection);
            let posts = schema(manager, "posts");
            let post = store
                .fetch_record(posts, Identifier::Integer(10), &QueryParameters::new(posts))?
                .content;

            let linked = store.link_record(post, "author", Identifier::Integer(2))?;
            let stored = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(posts))?;

            assert_eq!(linked, Some(Identifier::Integer(2)));
            assert_eq!(stored["author_id"], Attribute::Integer(2));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_has_one_sets_related_foreign_key_and_returns_related_id()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_profile(manager, &connection, 5, 2, "bob's page")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let profiles = schema(manager, "profiles");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            let linked = store.link_record(user, "profile", Identifier::Integer(5))?;
            let stored = manager
                .table("profiles", &connection)?
                .find(Identifier::Integer(5), &QueryParameters::new(profiles))?;

            assert_eq!(linked, Some(Identifier::Integer(5)));
            assert_eq!(stored["user_id"], Attribute::Integer(1));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_has_one_conflicting_owner_is_a_unique_violation()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_profile(manager, &connection, 5, 1, "alice's page")?;
            seed_profile(manager, &connection, 6, 2, "bob's page")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            // Alice already owns profile 5; claiming profile 6 collides on the UNIQUE user_id.
            assert!(matches!(
                store.link_record(user, "profile", Identifier::Integer(6)),
                Err(Error::ConstraintViolation {
                    kind: ConstraintKind::Unique,
                    ..
                })
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_on_to_many_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.link_record(user, "posts", Identifier::Integer(1)),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_unknown_relationship_is_error() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.link_record(user, "ghost", Identifier::Integer(1)),
                Err(Error::InvalidRelationshipAccess { .. })
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_belongs_to_non_primary_key_relates_by_code() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_org(manager, &connection, 1, "acme")?;
            seed_org(manager, &connection, 2, "globex")?;
            seed_member(manager, &connection, 10, "root", "acme")?;

            let store = Store::new(manager, &connection);
            let members = schema(manager, "members");
            let member = store
                .fetch_record(
                    members,
                    Identifier::Integer(10),
                    &QueryParameters::new(members),
                )?
                .content;

            // The relationship is keyed on `orgs.code`, not the org's primary key.
            let linked = store.link_record(member, "org", Identifier::Integer(2))?;
            let stored = manager
                .table("members", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(members))?;

            assert_eq!(linked, Some(Identifier::Integer(2)));
            assert_eq!(stored["org_code"], Attribute::Text("globex".to_string()));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_has_one_non_primary_key_relates_by_code() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_org(manager, &connection, 1, "acme")?;
            seed_org(manager, &connection, 2, "globex")?;
            seed_member(manager, &connection, 10, "root", "globex")?;

            let store = Store::new(manager, &connection);
            let orgs = schema(manager, "orgs");
            let members = schema(manager, "members");
            let org = store
                .fetch_record(orgs, Identifier::Integer(1), &QueryParameters::new(orgs))?
                .content;

            // Own key is the non-primary `orgs.code`, read off the fetched org.
            let linked = store.link_record(org, "member", Identifier::Integer(10))?;
            let stored = manager
                .table("members", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(members))?;

            assert_eq!(linked, Some(Identifier::Integer(10)));
            assert_eq!(stored["org_code"], Attribute::Text("acme".to_string()));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_belongs_to_missing_target_is_related_not_found()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "hello")?;

            let store = Store::new(manager, &connection);
            let posts = schema(manager, "posts");
            let post = store
                .fetch_record(posts, Identifier::Integer(10), &QueryParameters::new(posts))?
                .content;

            assert!(matches!(
                store.link_record(post, "author", Identifier::Integer(999)),
                Err(Error::RelatedRecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_belongs_to_non_primary_key_missing_target_is_related_not_found()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_org(manager, &connection, 1, "acme")?;
            seed_member(manager, &connection, 10, "root", "acme")?;

            let store = Store::new(manager, &connection);
            let members = schema(manager, "members");
            let member = store
                .fetch_record(
                    members,
                    Identifier::Integer(10),
                    &QueryParameters::new(members),
                )?
                .content;

            assert!(matches!(
                store.link_record(member, "org", Identifier::Integer(999)),
                Err(Error::RelatedRecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_has_one_missing_target_is_related_not_found()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.link_record(user, "profile", Identifier::Integer(999)),
                Err(Error::RelatedRecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_missing_primary_is_record_not_found() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            // A post that was never persisted: the target user exists, the primary does not.
            let post = Record::from_relationships(schema(manager, "posts"), Relationships::new())
                .with_id(Some(Identifier::Integer(999)));

            assert!(matches!(
                store.link_record(post, "author", Identifier::Integer(1)),
                Err(Error::RecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_record_has_one_missing_primary_is_record_not_found()
    -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_org(manager, &connection, 1, "acme")?;
            seed_member(manager, &connection, 10, "root", "acme")?;

            let store = Store::new(manager, &connection);
            // An org whose code is absent from the table: writing it onto the member's foreign
            // key violates, and for has-one that points back at a missing primary.
            let ghost = Record::from_attributes(
                schema(manager, "orgs"),
                Attributes::from_iter([("code", Attribute::Text("ghost".to_string()))]),
            );

            assert!(matches!(
                store.link_record(ghost, "member", Identifier::Integer(10)),
                Err(Error::RecordNotFound)
            ));

            Ok(())
        })
    }

    // --- relink_record -----------------------------------------------------

    #[test]
    fn test_relink_record_belongs_to_replaces_target() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 10, 1, "hello")?;

            let store = Store::new(manager, &connection);
            let posts = schema(manager, "posts");
            let post = store
                .fetch_record(posts, Identifier::Integer(10), &QueryParameters::new(posts))?
                .content;

            let linked = store.relink_record(post, "author", Identifier::Integer(2))?;
            let stored = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(posts))?;

            assert_eq!(linked, Some(Identifier::Integer(2)));
            assert_eq!(stored["author_id"], Attribute::Integer(2));

            Ok(())
        })
    }

    // Replacing a to-one detaches the prior owner before setting the new one, so the related side's
    // UNIQUE key never collides mid-write (where `link_record` would).
    #[test]
    fn test_relink_record_has_one_detaches_prior_owner() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_org(manager, &connection, 1, "acme")?;
            seed_org(manager, &connection, 2, "globex")?;
            seed_member(manager, &connection, 10, "root", "acme")?;
            seed_member(manager, &connection, 11, "admin", "globex")?;

            let store = Store::new(manager, &connection);
            let orgs = schema(manager, "orgs");
            let members = schema(manager, "members");
            let org = store
                .fetch_record(orgs, Identifier::Integer(1), &QueryParameters::new(orgs))?
                .content;

            let linked = store.relink_record(org, "member", Identifier::Integer(11))?;

            let member11 = manager
                .table("members", &connection)?
                .find(Identifier::Integer(11), &QueryParameters::new(members))?;
            let member10 = manager
                .table("members", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(members))?;

            assert_eq!(linked, Some(Identifier::Integer(11)));
            assert_eq!(member11["org_code"], Attribute::Text("acme".to_string()));
            assert_eq!(member10["org_code"], Attribute::Null);

            Ok(())
        })
    }

    #[test]
    fn test_relink_record_missing_target_is_related_not_found() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.relink_record(user, "profile", Identifier::Integer(999)),
                Err(Error::RelatedRecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_relink_record_on_to_many_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.relink_record(user, "posts", Identifier::Integer(1)),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }

    // --- unlink_record -----------------------------------------------------

    #[test]
    fn test_unlink_record_belongs_to_clears_foreign_key() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "hello")?;

            let store = Store::new(manager, &connection);
            let posts = schema(manager, "posts");
            let post = store
                .fetch_record(posts, Identifier::Integer(10), &QueryParameters::new(posts))?
                .content;

            let linked = store.unlink_record(post, "author")?;
            let stored = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(posts))?;

            assert_eq!(linked, None);
            assert_eq!(stored["author_id"], Attribute::Null);

            Ok(())
        })
    }

    #[test]
    fn test_unlink_record_has_one_detaches_owner() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_org(manager, &connection, 1, "acme")?;
            seed_member(manager, &connection, 10, "root", "acme")?;

            let store = Store::new(manager, &connection);
            let orgs = schema(manager, "orgs");
            let members = schema(manager, "members");
            let org = store
                .fetch_record(orgs, Identifier::Integer(1), &QueryParameters::new(orgs))?
                .content;

            let linked = store.unlink_record(org, "member")?;
            let stored = manager
                .table("members", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(members))?;

            assert_eq!(linked, None);
            assert_eq!(stored["org_code"], Attribute::Null);

            Ok(())
        })
    }

    // Clearing an already-empty to-one is a no-op success.
    #[test]
    fn test_unlink_record_when_empty_is_noop() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert_eq!(store.unlink_record(user, "profile")?, None);

            Ok(())
        })
    }

    #[test]
    fn test_unlink_record_on_to_many_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.unlink_record(user, "posts"),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }

    // --- link_collection ---------------------------------------------------

    // JSON:API POST to a to-many adds the specified members, leaving existing ones in place.
    #[test]
    fn test_link_collection_adds_without_disturbing_existing() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 10, 1, "one")?;
            seed_post(manager, &connection, 11, 2, "two")?;
            seed_post(manager, &connection, 12, 2, "three")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            let mut membership = store
                .link_collection(
                    user,
                    "posts",
                    vec![Identifier::Integer(11), Identifier::Integer(12)],
                )?
                .iter()
                .map(|id| id.to_i64())
                .collect::<Result<Vec<_>, _>>()?;
            membership.sort();

            assert_eq!(membership, vec![10, 11, 12]);

            Ok(())
        })
    }

    // JSON:API: "If a given type and id is already in the relationship, the server MUST NOT add it
    // again" — re-adding an existing member is a no-op success.
    #[test]
    fn test_link_collection_already_present_is_idempotent() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "one")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert_eq!(
                store.link_collection(user, "posts", vec![Identifier::Integer(10)])?,
                vec![Identifier::Integer(10)]
            );

            Ok(())
        })
    }

    #[test]
    fn test_link_collection_missing_target_is_related_not_found() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.link_collection(user, "posts", vec![Identifier::Integer(999)]),
                Err(Error::RelatedRecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_link_collection_on_to_one_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.link_collection(user, "profile", vec![Identifier::Integer(1)]),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }

    // --- relink_collection -------------------------------------------------

    // JSON:API PATCH to a to-many "completely replace[s] every member of the relationship".
    #[test]
    fn test_relink_collection_replaces_membership() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_user(manager, &connection, 2, "bob")?;
            seed_post(manager, &connection, 10, 1, "one")?;
            seed_post(manager, &connection, 11, 1, "two")?;
            seed_post(manager, &connection, 12, 2, "three")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let posts = schema(manager, "posts");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            let mut membership = store
                .relink_collection(
                    user,
                    "posts",
                    vec![Identifier::Integer(11), Identifier::Integer(12)],
                )?
                .iter()
                .map(|id| id.to_i64())
                .collect::<Result<Vec<_>, _>>()?;
            membership.sort();

            let detached = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(posts))?;

            assert_eq!(membership, vec![11, 12]);
            assert_eq!(detached["author_id"], Attribute::Null);

            Ok(())
        })
    }

    // Replacing with an empty set clears the relationship.
    #[test]
    fn test_relink_collection_to_empty_clears() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "one")?;
            seed_post(manager, &connection, 11, 1, "two")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let posts = schema(manager, "posts");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(store.relink_collection(user, "posts", vec![])?.is_empty());

            let post10 = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(10), &QueryParameters::new(posts))?;
            let post11 = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(11), &QueryParameters::new(posts))?;

            assert_eq!(post10["author_id"], Attribute::Null);
            assert_eq!(post11["author_id"], Attribute::Null);

            Ok(())
        })
    }

    // JSON:API: complete replacement MUST "return an appropriate error response if some resources
    // cannot be found".
    #[test]
    fn test_relink_collection_missing_target_is_related_not_found() -> Result<(), Box<dyn StdError>>
    {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "one")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.relink_collection(
                    user,
                    "posts",
                    vec![Identifier::Integer(10), Identifier::Integer(999)]
                ),
                Err(Error::RelatedRecordNotFound)
            ));

            Ok(())
        })
    }

    #[test]
    fn test_relink_collection_on_to_one_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.relink_collection(user, "profile", vec![Identifier::Integer(1)]),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }

    // --- unlink_collection -------------------------------------------------

    // JSON:API DELETE from a to-many removes the specified members, leaving the rest.
    #[test]
    fn test_unlink_collection_removes_specified() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "one")?;
            seed_post(manager, &connection, 11, 1, "two")?;
            seed_post(manager, &connection, 12, 1, "three")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let posts = schema(manager, "posts");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            let mut membership = store
                .unlink_collection(user, "posts", vec![Identifier::Integer(11)])?
                .iter()
                .map(|id| id.to_i64())
                .collect::<Result<Vec<_>, _>>()?;
            membership.sort();

            let removed = manager
                .table("posts", &connection)?
                .find(Identifier::Integer(11), &QueryParameters::new(posts))?;

            assert_eq!(membership, vec![10, 12]);
            assert_eq!(removed["author_id"], Attribute::Null);

            Ok(())
        })
    }

    // JSON:API: removing a member "already missing from the relationship" MUST succeed.
    #[test]
    fn test_unlink_collection_absent_member_is_idempotent() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;
            seed_post(manager, &connection, 10, 1, "one")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert_eq!(
                store.unlink_collection(user, "posts", vec![Identifier::Integer(99)])?,
                vec![Identifier::Integer(10)]
            );

            Ok(())
        })
    }

    #[test]
    fn test_unlink_collection_on_to_one_is_kind_mismatch() -> Result<(), Box<dyn StdError>> {
        with_manager(|manager| {
            let connection = manager.acquire()?;
            seed_user(manager, &connection, 1, "alice")?;

            let store = Store::new(manager, &connection);
            let users = schema(manager, "users");
            let user = store
                .fetch_record(users, Identifier::Integer(1), &QueryParameters::new(users))?
                .content;

            assert!(matches!(
                store.unlink_collection(user, "profile", vec![Identifier::Integer(1)]),
                Err(Error::MismatchedRelationshipKind { .. })
            ));

            Ok(())
        })
    }
}
