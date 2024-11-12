use crate::{
    adapter::{Context, Parameters, UriGenerator},
    http_wrappers::Uri,
    resourceful::{
        Relationships,
        Resourceful,
        related_data::{
            RelatedData,
            RelatedCollection,
            RelatedRecord,
        }
    },
    spec::{
        document::{self, Document, ImplementationInfo},
        error::Error,
        identifier::Identifier,
        links::Link,
        relationship::{self, Linkage, Relationship},
        resource::{self, Resource}
    }
};
use std::{
    borrow::Borrow,
    collections::HashMap
};
use serde_json::Value;

pub struct Adapter<G: UriGenerator> {
    cache: HashMap<Identifier, Resource>,
    uri: Uri,
    params: Parameters,
    uri_generator: G,
}

impl<G: UriGenerator> Adapter<G> {
    pub fn new(uri: Uri, uri_generator: G) -> Self {
        let params = Parameters::from(uri.borrow());
        Self {
            cache: HashMap::new(),
            uri,
            params,
            uri_generator
        }
    }

    pub fn make_resource(&mut self, model: &impl Resourceful, params: &Parameters) -> Resource {
        if self.cache.contains_key(&model.identifier()) {
            return self.cache.get(&model.identifier()).unwrap().clone();
        }

        let mut context = Context::new(self, params.clone());

        let attributes = model.attributes(&context);
        let relationships = model.relationships(&mut context);
        let meta = model.meta(&context);
        let links = resource::Links {
            this: self.uri_generator.uri_for_resource(&model.identifier())
        };

        Resource {
            identifier: model.identifier(),
            attributes,
            relationships: relationships.map(|r|
                self.link_relationships(model.identifier(), r)
            ),
            links: links.into(),
            meta
        }
    }

    pub fn into_resource_document(mut self, model: &impl Resourceful) -> Document {
        Document {
            content: self.make_resource(model, &self.params).into(),
            meta: None,
            jsonapi: self.implementation_info().into(),
            links: self.document_links().into(),
            included: self.included_resources(),
        }
    }

    pub fn into_collection_document<'a, I, R>(mut self, models: I) -> Document
    where
        I: IntoIterator<Item=&'a R>,
        R: Resourceful + 'a
    {
        Document {
            content: models
                .into_iter()
                .map(|model|
                    self.make_resource(model, &self.params)
                )
                .collect::<Vec<Resource>>()
                .into(),
            meta: None,
            jsonapi: self.implementation_info().into(),
            links: self.document_links().into(),
            included: self.included_resources()
        }
    }

    pub fn into_errors_document<I>(self, errors: Vec<Error>) -> Document {
        Document {
            content: errors.into(),
            meta: None,
            jsonapi: self.implementation_info().into(),
            links: self.document_links().into(),
            included: None
        }
    }

    fn implementation_info(&self) -> ImplementationInfo {
        ImplementationInfo {
            version: Some("1.1".to_string()),
            ext: None,
            profile: None,
            meta: None
        }
    }

    fn document_links(&self) -> document::Links {
        document::Links {
            this: Link::Uri(self.uri.clone()).into(),
            related: None,
            described_by: None,
        }
    }

    fn included_resources(self) -> Option<Vec<Value>> {
        if self.cache.is_empty() {
            None
        } else {
            self.cache.values()
                .into_iter()
                .map(|resource|
                    serde_json::to_value(&resource).unwrap()
                )
                .collect::<Vec<_>>()
                .into()
        }
    }

    pub(crate) fn link_related_data(&mut self, related_data: RelatedData) -> Linkage {
        match related_data {
            RelatedData::None => Linkage::Empty,
            RelatedData::One(record) => {
                match record {
                    RelatedRecord::Unloaded(id) => Linkage::ToOne(id),
                    RelatedRecord::Loaded(record) => {
                        let id = record.identifier.clone();
                        self.cache.insert(record.identifier.clone(), record);
                        Linkage::ToOne(id)
                    }
                }

            },
            RelatedData::Many(collection) => {
                match collection {
                    RelatedCollection::Unloaded(ids) => Linkage::ToMany(ids),
                    RelatedCollection::Loaded(records) => {
                        let ids = records.into_iter().map(|model| {
                            let id = model.identifier.clone();
                            self.cache.insert(model.identifier.clone(), model);
                            id
                        }).collect();
                        Linkage::ToMany(ids)
                    }
                }

            }
        }
    }
    
    pub(crate) fn link_relationships(&mut self, identifier: Identifier, relationships: Relationships)
        -> HashMap<String, Relationship> 
    {
        relationships
            .into_iter()
            .map(|(relationship_name, related_data)| {
                let linkage = self.link_related_data(related_data);
                let relationship = Relationship {
                    links: relationship::Links {
                        this: self.uri_generator
                            .uri_for_relationship(&identifier, relationship_name.as_str())
                            .into(),
                        related: self.uri_generator
                            .uri_for_related(&identifier, relationship_name.as_str())
                            .into()
                    }.into(),
                    data: Some(linkage),
                    meta: None
                };
                (relationship_name, relationship)
            })
            .collect()
    }
}
