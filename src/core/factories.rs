use crate::{
    core::error::Error,
    database::{
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
    routing::UriGenerator,
};
use serde_json::Value;
use std::collections::HashMap;

pub enum Content<'sch: 'req, 'req> {
    Resource(&'req Record<'sch>),
    Collection(Vec<&'req Record<'sch>>),
    LinkageToOne(Option<Identifier>),
    LinkageToMany(Vec<Identifier>),
    Errors(Vec<JsonApiError>),
}

impl<'sch: 'req, 'req> From<&'req Record<'sch>> for Content<'sch, 'req> {
    fn from(resourceful: &'req Record<'sch>) -> Self {
        Content::Resource(resourceful)
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

pub fn make_record_resource(
    record: &Record,
    uri_generator: &dyn UriGenerator,
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

            let links = relationship::Links {
                this: Some(uri_generator.uri_for_relationship(&identifier, relationship)),
                related: Some(uri_generator.uri_for_related(&identifier, relationship))
            };

            Ok((relationship.to_string(), Relationship {
                data: Some(linkage),
                links: Some(links),
                meta: None
            }))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    let links = resource::Links {
        this: uri_generator.uri_for_resource(&identifier),
    };

    Ok(Resource {
        identifier,
        attributes: Some(attributes),
        relationships: Some(relationships),
        links: links.into(),
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

pub fn to_document<'sch: 'req, 'req>(
    content: impl Into<Content<'sch, 'req>>,
    included: Vec<Record>,
    uri: &Uri,
    uri_generator: &dyn UriGenerator,
) -> Result<Document, Error> {
    let content = content.into();
    // `included` MUST NOT accompany a document without primary `data` (i.e. an
    // errors document); a data document keeps it, even as an empty array.
    let carries_data = !matches!(content, Content::Errors(_));

    let content: PrimaryContent = match content {
        Content::Resource(record) => make_record_resource(record, uri_generator)?.into(),
        Content::Collection(collection) => collection
            .into_iter()
            .map(|record| make_record_resource(record, uri_generator))
            .collect::<Result<Vec<_>, _>>()?
            .into(),
        Content::LinkageToOne(Some(identifier)) => make_linkage_resource(identifier).into(),
        Content::LinkageToOne(None) => PrimaryContent::Empty { data: () },
        Content::LinkageToMany(identifiers) => identifiers
            .into_iter()
            .map(make_linkage_resource)
            .collect::<Vec<_>>()
            .into(),
        Content::Errors(errors) => errors.into(),
    };

    let included = included
        .into_iter()
        .map(|record| make_record_resource(&record, uri_generator))
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Document {
        content,
        meta: None,
        jsonapi: Some(implementation_info()),
        links: Some(document_links(uri)),
        included: carries_data.then_some(included),
    })
}
