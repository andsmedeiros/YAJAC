use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::attributes::{Attribute, Attributes, ForeignKeys, Identifier, from_value};
use crate::database::composite::{Composite, CompositeCollection, CompositeRecord};
use crate::database::connection::Connection as ConnectionInterface;
use crate::database::data_loader::DataLoader;
use crate::database::error::Error;
use crate::database::query_parameters::{FilterParameters, FilterValue, QueryParameters};
use crate::database::record::{NewRecord, Record};
use crate::database::registry::Registry;
use crate::database::relationships::{Relationship as DatabaseRelationship, Relationships};
use crate::database::schema::{
    IdentifierType, RelatedResource, Relationship as SchemaRelationship, TableSchema,
};
use crate::database::table::Table as TableInterface;
use crate::json_api::identifier::Identifier as ResourceIdentifier;
use crate::json_api::relationship::Linkage;
use crate::json_api::resource::Resource;
use indexmap::IndexSet;
use serde_json::Value;

pub struct Store<'sch, 'req, Adapter: AdapterInterface> {
    registry: &'sch Registry<'sch, Adapter>,
    connection: &'req Adapter::Connection,
}

impl<'sch, 'req, Adapter: AdapterInterface> Store<'sch, 'req, Adapter> {
    pub fn new(
        registry: &'sch Registry<'sch, Adapter>,
        connection: &'req Adapter::Connection,
    ) -> Self {
        Store {
            registry,
            connection,
        }
    }

    pub fn fetch_record(
        &self,
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let mut content = self.table(schema)?.find(id, parameters)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn fetch_collection(
        &self,
        schema: &'sch TableSchema<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeCollection<'sch>, Error> {
        self.connection.transaction(|| {
            let mut content = self.table(schema)?.query(parameters)?;
            let included = self
                .loader()
                .load_for_collection(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn create_record(
        &self,
        new_record: NewRecord<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let mut content = self
                .table(new_record.schema)?
                .insert(new_record.attributes, parameters)?;
            self.replace_relationships(&content, &new_record.relationships)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn update_record(
        &self,
        record: Record<'sch>,
        parameters: &QueryParameters<'sch, 'req>,
    ) -> Result<CompositeRecord<'sch>, Error> {
        self.connection.transaction(|| {
            let table = self.table(record.schema)?;
            let mut content = if record.attributes.is_empty() {
                table.find(record.id.clone(), parameters)?
            } else {
                table.update(record.id.clone(), record.attributes, parameters)?
            };
            self.replace_relationships(&content, &record.relationships)?;
            let included = self.loader().load_for_record(&mut content, parameters)?;

            Ok(Composite { content, included })
        })
    }

    pub fn delete_record(
        &self,
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
    ) -> Result<(), Error> {
        self.connection
            .transaction(|| self.table(schema)?.delete(id))
    }

    pub fn materialise_new(
        &self,
        resource: &Resource,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<NewRecord<'sch>, Error> {
        let (attributes, relationships) = self.hydrate(resource, schema)?;

        Ok(NewRecord {
            schema,
            attributes,
            relationships,
        })
    }

    pub fn materialise(
        &self,
        resource: &Resource,
        schema: &'sch TableSchema<'sch>,
        id: Identifier,
    ) -> Result<Record<'sch>, Error> {
        let (attributes, relationships) = self.hydrate(resource, schema)?;
        let mut record = Record::new(schema, id, attributes, ForeignKeys::new());
        record.relationships = relationships;

        Ok(record)
    }

    fn hydrate(
        &self,
        resource: &Resource,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<(Attributes, Relationships<'sch>), Error> {
        let mut attributes = match &resource.attributes {
            Some(map) => from_value(schema, Value::Object(map.clone().into_iter().collect()))?,
            None => Attributes::new(),
        };
        let mut relationships = Relationships::new();

        for (name, relationship) in resource.relationships.iter().flatten() {
            let (key, descriptor) = self.relationship(schema, name)?;
            let related = descriptor.related_resource();
            let linkage = relationship.data.clone().unwrap_or(Linkage::Empty);

            match descriptor {
                SchemaRelationship::BelongsTo(_) => {
                    let value = match linkage {
                        Linkage::Empty => Attribute::Null,
                        Linkage::ToOne(identifier) => {
                            self.belongs_to_value(related, &identifier)?
                        }
                        Linkage::ToMany(_) => return Err(Self::mismatch(schema, name)),
                    };
                    attributes.insert(related.keys.own.to_string(), value);
                }
                SchemaRelationship::HasOne(_) | SchemaRelationship::HasMany(_) => {
                    relationships.insert(
                        key,
                        DatabaseRelationship::HasMany(self.members(related, linkage)?),
                    );
                }
            }
        }

        Ok((attributes, relationships))
    }

    fn replace_relationships(
        &self,
        owner: &Record<'sch>,
        relationships: &Relationships<'sch>,
    ) -> Result<(), Error> {
        for (name, relationship) in relationships {
            let DatabaseRelationship::HasMany(members) = relationship else {
                continue;
            };

            let related = self.relationship(owner.schema, name)?.1.related_resource();
            self.replace_relationship(owner, related, members)?;
        }

        Ok(())
    }

    fn replace_relationship(
        &self,
        owner: &Record<'sch>,
        related: &RelatedResource<'sch>,
        members: &[Identifier],
    ) -> Result<(), Error> {
        let owner_value = Self::value_at(owner, related.keys.own)?;
        let related_schema = self.registry.schema(related.resource)?;
        let table = self.table(related_schema)?;
        let foreign_key = related.keys.related;
        let primary_key = related_schema.primary_key.name;

        let mut detach =
            FilterParameters::from([(foreign_key, vec![FilterValue::Equal(owner_value.clone())])]);
        if !members.is_empty() {
            detach.insert(primary_key, vec![FilterValue::NotIn(member_set(members))]);
        }
        table.update_batch(
            Attributes::from_iter([(foreign_key.to_string(), Attribute::Null)]),
            &scope(related_schema, detach),
        )?;

        if !members.is_empty() {
            let adopt =
                FilterParameters::from([(primary_key, vec![FilterValue::In(member_set(members))])]);
            table.update_batch(
                Attributes::from_iter([(foreign_key.to_string(), owner_value)]),
                &scope(related_schema, adopt),
            )?;
        }

        Ok(())
    }

    fn members(
        &self,
        related: &RelatedResource<'sch>,
        linkage: Linkage,
    ) -> Result<Vec<Identifier>, Error> {
        let kind = self.registry.schema(related.resource)?.primary_key.kind;

        match linkage {
            Linkage::Empty => Ok(Vec::new()),
            Linkage::ToOne(identifier) => Ok(vec![target_identifier(&identifier, kind)?]),
            Linkage::ToMany(identifiers) => identifiers
                .iter()
                .map(|identifier| target_identifier(identifier, kind))
                .collect(),
        }
    }

    fn belongs_to_value(
        &self,
        related: &RelatedResource<'sch>,
        linkage: &ResourceIdentifier,
    ) -> Result<Attribute, Error> {
        let related_schema = self.registry.schema(related.resource)?;
        let target = target_identifier(linkage, related_schema.primary_key.kind)?;

        if related_schema.is_primary_key(related.keys.related) {
            Ok(Attribute::from(target))
        } else {
            let record = self
                .table(related_schema)?
                .find(target, &QueryParameters::new(related_schema))?;
            Self::value_at(&record, related.keys.related)
        }
    }

    fn relationship(
        &self,
        schema: &'sch TableSchema<'sch>,
        name: &str,
    ) -> Result<(&'sch str, &'sch SchemaRelationship<'sch>), Error> {
        schema
            .relationships
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(key, descriptor)| (*key, descriptor))
            .ok_or_else(|| Error::SchemaValidationFailure {
                schema: schema.name.to_string(),
                attribute: name.to_string(),
                message: "Unknown relationship".to_string(),
            })
    }

    fn value_at(record: &Record, column: &str) -> Result<Attribute, Error> {
        if record.schema.is_primary_key(column) {
            Ok(Attribute::from(record.id.clone()))
        } else {
            record
                .attributes
                .get(column)
                .or_else(|| record.foreign_keys.get(column))
                .map(ToOwned::to_owned)
                .ok_or_else(|| Error::DataLoadingError {
                    message: format!(
                        "Column '{column}' was not loaded on '{}'",
                        record.schema.name
                    ),
                })
        }
    }

    fn mismatch(schema: &TableSchema, name: &str) -> Error {
        Error::InconsistentSchema {
            schema: schema.name.to_string(),
            attribute: name.to_string(),
            message: "Relationship kind does not match schema".to_string(),
        }
    }

    fn table(&self, schema: &'sch TableSchema<'sch>) -> Result<Adapter::Table<'sch, 'req>, Error> {
        self.registry.table(schema.name, self.connection)
    }

    fn loader(&self) -> DataLoader<'sch, 'req, Adapter> {
        DataLoader::new(self.registry, self.connection)
    }
}

fn target_identifier(
    identifier: &ResourceIdentifier,
    kind: IdentifierType,
) -> Result<Identifier, Error> {
    let ResourceIdentifier::Existing { id, .. } = identifier else {
        return Err(Error::InvalidAttributeSet);
    };

    match kind {
        IdentifierType::Integer => {
            id.parse()
                .map(Identifier::Integer)
                .map_err(|_| Error::InvalidAttribute {
                    attribute: id.clone(),
                    kind: "Integer".to_string(),
                    message: "linkage id is not a valid integer".to_string(),
                })
        }
        IdentifierType::Text => Ok(Identifier::Text(id.clone())),
    }
}

fn member_set(members: &[Identifier]) -> IndexSet<Attribute> {
    members.iter().cloned().map(Attribute::from).collect()
}

fn scope<'sch>(
    schema: &'sch TableSchema<'sch>,
    filter: FilterParameters<'sch>,
) -> QueryParameters<'sch, 'sch> {
    let mut parameters = QueryParameters::new(schema);
    parameters.filter = Some(filter);
    parameters
}
