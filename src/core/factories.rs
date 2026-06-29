use crate::{
    core::error::Error,
    database::{
        record::Record, relationships::Relationship as DatabaseRelationship,
        schema::Relationship as SchemaRelationship,
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

pub enum Content<'a> {
    Resource(&'a Record<'a>),
    Collection(Vec<&'a Record<'a>>),
    Errors(Vec<JsonApiError>),
}

impl<'a> From<&'a Record<'a>> for Content<'a> {
    fn from(resourceful: &'a Record) -> Self {
        Content::Resource(resourceful)
    }
}

impl<'a> From<&'a Vec<Record<'a>>> for Content<'a> {
    fn from(collection: &'a Vec<Record<'a>>) -> Self {
        Content::Collection(collection.iter().collect())
    }
}

impl<'a> From<Vec<JsonApiError>> for Content<'a> {
    fn from(errors: Vec<JsonApiError>) -> Self {
        Content::Errors(errors)
    }
}

pub fn make_resource(record: &Record, uri_generator: &dyn UriGenerator) -> Result<Resource, Error> {
    let identifier = record.identifier();
    let attributes = record
        .attributes
        .iter()
        .map(|(name, value)| (name.clone(), Value::from(value.clone())))
        .collect();

    let relationships = record.relationships
        .iter()
        .map(|(relationship, value)| -> Result<_, Error> {
            let descriptor = record.schema.relationship(relationship)
                .ok_or_else(|| Error::DocumentSerialisationError {
                    message: format!("Failed to describe relationship '{}' on model '{}'", relationship, record.kind())
                })?;

            let linkage = match (descriptor, value) {
                (SchemaRelationship::BelongsTo(def), DatabaseRelationship::BelongsTo(id)) |
                (SchemaRelationship::HasOne(def), DatabaseRelationship::HasOne(id)) =>
                    Linkage::ToOne(Identifier::Existing {
                        kind: def.resource.to_string(),
                        id: id.to_string()
                    }),
                (SchemaRelationship::HasMany(def), DatabaseRelationship::HasMany(ids)) =>
                    Linkage::ToMany(ids
                        .iter()
                        .map(|id| Identifier::Existing {
                            kind: def.resource.to_string(),
                            id: id.to_string()
                        })
                        .collect()
                    ),
                (SchemaRelationship::HasMany(_), DatabaseRelationship::Empty) =>
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

pub fn to_document<'a>(
    content: impl Into<Content<'a>>,
    included: Vec<Record>,
    uri: &Uri,
    uri_generator: &dyn UriGenerator,
) -> Result<Document, Error> {
    let content: PrimaryContent = match content.into() {
        Content::Resource(record) => make_resource(record, uri_generator)?.into(),
        Content::Collection(collection) => collection
            .into_iter()
            .map(|record| make_resource(record, uri_generator))
            .collect::<Result<Vec<_>, _>>()?
            .into(),
        Content::Errors(errors) => errors.into(),
    };

    let included = included
        .into_iter()
        .map(|record| make_resource(&record, uri_generator))
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Document {
        content,
        meta: None,
        jsonapi: Some(implementation_info()),
        links: Some(document_links(uri)),
        included: Some(included),
    })
}
