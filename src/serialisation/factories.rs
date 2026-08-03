use super::error::Error;
use super::uri_generator::UriGenerator;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::Identifier as DatabaseIdentifier,
        error::Error as DatabaseError,
        record::Record,
        relationships::Relationship as DatabaseRelationship,
        schema::{IdentifierType, RelationshipKind as SchemaRelationship, Schema},
    },
    http_wrappers::Uri,
    json_api::{
        document::{self, Document, ImplementationInfo},
        error::Error as JsonApiError,
        identifier::Identifier,
        links::Link,
        primary_content::PrimaryContent,
        relationship::{self, Linkage, Relationship},
        resource::{self, Resource},
    },
};
use serde_json::Value;
use std::collections::HashMap;

pub enum Content<'sch: 'req, 'req> {
    Resource(&'req Record<'sch>),
    Collection(Vec<&'req Record<'sch>>),
    LinkageToOne(Option<Identifier>),
    LinkageToMany(Vec<Identifier>),
    /// Null primary data standing for a resource that is absent rather than missing: an
    /// empty to-one related resource, which is served as `data: null` under a 200.
    Empty,
    Errors(Vec<JsonApiError>),
}

impl<'sch: 'req, 'req> From<&'req Record<'sch>> for Content<'sch, 'req> {
    fn from(resourceful: &'req Record<'sch>) -> Self {
        Content::Resource(resourceful)
    }
}

impl<'sch: 'req, 'req> From<Option<&'req Record<'sch>>> for Content<'sch, 'req> {
    fn from(resourceful: Option<&'req Record<'sch>>) -> Self {
        resourceful.map_or(Content::Empty, Content::Resource)
    }
}

impl<'sch: 'req, 'req> From<&'req Vec<Record<'sch>>> for Content<'sch, 'req> {
    fn from(collection: &'req Vec<Record<'sch>>) -> Self {
        Content::Collection(collection.iter().collect())
    }
}

impl<'sch: 'req, 'req> From<Option<Identifier>> for Content<'sch, 'req> {
    fn from(identifier: Option<Identifier>) -> Self {
        Content::LinkageToOne(identifier)
    }
}

impl<'sch: 'req, 'req> From<Vec<Identifier>> for Content<'sch, 'req> {
    fn from(identifiers: Vec<Identifier>) -> Self {
        Content::LinkageToMany(identifiers)
    }
}

impl<'sch: 'req, 'req> From<Vec<JsonApiError>> for Content<'sch, 'req> {
    fn from(errors: Vec<JsonApiError>) -> Self {
        Content::Errors(errors)
    }
}

impl<'sch> From<(DatabaseIdentifier, &'sch Schema<'sch>)> for Identifier {
    fn from((identifier, schema): (DatabaseIdentifier, &'sch Schema<'sch>)) -> Self {
        Identifier::Existing {
            kind: schema.name().to_string(),
            id: match identifier {
                DatabaseIdentifier::Integer(value) => value.to_string(),
                DatabaseIdentifier::Text(value) => value,
            },
        }
    }
}

impl<'sch> TryFrom<(Identifier, &'sch Schema<'sch>)> for DatabaseIdentifier {
    type Error = DatabaseError;

    fn try_from(value: (Identifier, &'sch Schema<'sch>)) -> Result<Self, DatabaseError> {
        let (identifier, schema) = value;

        let id = match identifier {
            Identifier::New { kind, .. } => Err(DatabaseError::MissingRecordId { schema: kind })?,
            Identifier::Existing { kind, id } if kind == schema.name() => {
                match schema.primary_key().kind {
                    IdentifierType::Integer => {
                        DatabaseIdentifier::Integer(id.parse().map_err(|_error| {
                            DatabaseError::InvalidAttributeConversion {
                                kind: "i64".to_string(),
                            }
                        })?)
                    }
                    IdentifierType::Text => DatabaseIdentifier::Text(id),
                }
            }

            _ => Err(DatabaseError::ResourceValidationFailure {
                schema: schema.name().to_string(),
                attribute: schema.primary_key().name.to_string(),
                message: "Resource identifier contains a mismatching schema".to_string(),
            })?,
        };

        Ok(id)
    }
}

pub(crate) fn make_record_resource<'sch, 'req, Adapter: AdapterInterface>(
    record: &Record<'sch>,
    generator: &UriGenerator<'sch, 'req, Adapter>,
) -> Result<Resource, Error> {
    let identifier = record.identifier();
    let attributes = record
        .attributes
        .iter()
        .map(|(name, value)| (name.to_string(), Value::from(value.clone())))
        .collect();

    let relationships = record.relationships
        .iter()
        .map(|(relationship, value)| -> Result<_, Error> {
            let descriptor = record.schema.relationship(relationship)
                .ok_or_else(|| Error::DocumentSerialisationError {
                    message: format!("Failed to describe relationship '{}' on model '{}'", relationship, record.kind())
                })?;
            let related = &descriptor.related;

            let linkage = match (descriptor.kind, value) {
                (SchemaRelationship::BelongsTo, DatabaseRelationship::BelongsTo(id)) |
                (SchemaRelationship::HasOne, DatabaseRelationship::HasOne(id)) =>
                    Linkage::ToOne(Identifier::Existing {
                        kind: related.resource.to_string(),
                        id: id.to_string()
                    }),
                (SchemaRelationship::HasMany, DatabaseRelationship::HasMany(ids)) =>
                    Linkage::ToMany(ids
                        .iter()
                        .map(|id| Identifier::Existing {
                            kind: related.resource.to_string(),
                            id: id.to_string()
                        })
                        .collect()
                    ),
                (SchemaRelationship::HasMany, DatabaseRelationship::Empty) =>
                    Linkage::ToMany(Vec::new()),
                (_, DatabaseRelationship::Empty) => Linkage::Empty,
                _ => Err(Error::DocumentSerialisationError {
                    message: format!(
                        "Relationship '{}' with value '{:?}' does not match schema definition of '{:?}'",
                        relationship, value, descriptor
                    )
                })?
            };

            let links = match (
                generator.uri_for_linkage(record, relationship)?,
                generator.uri_for_related(record, relationship)?,
            ) {
                (None, None) => None,
                (this, related) => Some(relationship::Links { this, related }),
            };

            Ok((relationship.to_string(), Relationship {
                data: Some(linkage),
                links,
                meta: None
            }))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    let links = generator
        .uri_for_resource(record)?
        .map(|this| resource::Links { this });

    Ok(Resource {
        identifier,
        attributes: Some(attributes),
        relationships: Some(relationships),
        links,
        meta: None,
    })
}

pub fn make_linkage_resource(identifier: Identifier) -> Resource {
    Resource {
        identifier,
        attributes: None,
        relationships: None,
        links: None,
        meta: None,
    }
}

fn implementation_info() -> ImplementationInfo {
    ImplementationInfo {
        version: Some("1.1".to_string()),
        ext: None,
        profile: None,
        meta: None,
    }
}

fn document_links(uri: &Uri) -> document::Links {
    document::Links {
        this: Link::Uri(uri.clone()).into(),
        related: None,
        described_by: None,
    }
}

pub(crate) fn to_document<'sch: 'req, 'req, Adapter: AdapterInterface>(
    content: impl Into<Content<'sch, 'req>>,
    included: Vec<Record<'sch>>,
    uri: &Uri,
    generator: &UriGenerator<'sch, 'req, Adapter>,
) -> Result<Document, Error> {
    let content = content.into();
    // `included` MUST NOT accompany a document without primary `data` (i.e. an
    // errors document); a data document keeps it, even as an empty array.
    let carries_data = !matches!(content, Content::Errors(_));

    let content: PrimaryContent = match content {
        Content::Resource(record) => make_record_resource(record, generator)?.into(),
        Content::Collection(collection) => collection
            .into_iter()
            .map(|record| make_record_resource(record, generator))
            .collect::<Result<Vec<_>, _>>()?
            .into(),
        Content::LinkageToOne(Some(identifier)) => make_linkage_resource(identifier).into(),
        Content::LinkageToOne(None) | Content::Empty => PrimaryContent::Empty { data: () },
        Content::LinkageToMany(identifiers) => identifiers
            .into_iter()
            .map(make_linkage_resource)
            .collect::<Vec<_>>()
            .into(),
        Content::Errors(errors) => errors.into(),
    };

    let included = included
        .into_iter()
        .map(|record| make_record_resource(&record, generator))
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Document {
        content,
        meta: None,
        jsonapi: Some(implementation_info()),
        links: Some(document_links(uri)),
        included: carries_data.then_some(included),
    })
}
