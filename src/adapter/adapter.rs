use crate::{
    adapter::{Context, Parameters},
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
        document::{Document, ImplementationInfo},
        error::Error,
        identifier::Identifier, 
        relationship::{self, Linkage, Relationship},
        resource::{self, Resource}
    }
};
use std::{
    borrow::Borrow,
    collections::HashMap
};
use serde_json::Value;

const GENERATED_INVALID_MSG: &'static str = "Generated an invalid URI";

pub trait UriGenerator {
    fn base_url(&self) -> String { "".to_string() }

    fn uri_for_resource(&self, identifier: &Identifier) -> Uri {
        let base = self.base_url();
        
        if let Identifier::Existing { kind, id } = identifier  {
            format!("{base}/{kind}/{id}")
                .parse::<Uri>()
                .expect(GENERATED_INVALID_MSG)
        } else {
            panic!("Attempted to generate URI for unpersisted resource");
        }
    }

    fn uri_for_relationship(&self, identifier: &Identifier, relationship: &str) -> Uri {
        let resource = self.uri_for_resource(identifier);
        format!("{resource}/relationships/{relationship}")
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }

    fn uri_for_related(&self, identifier: &Identifier, relationship: &str) -> Uri {
        let resource = self.uri_for_resource(identifier);
        format!("{resource}/{relationship}")
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }
}

pub struct DefaultUriGenerator<'a> {
    protocol: &'a str,
    host: &'a str,
    namespace: &'a str
}

impl<'a> DefaultUriGenerator<'a> {
    pub fn new(protocol: &'a str, host: &'a str, namespace: &'a str) -> Self {
        assert!(
            !protocol.is_empty() && !host.is_empty() ||
            protocol.is_empty() && host.is_empty(),
            "URL protocol and host must either be both absent of both present."
        );
        DefaultUriGenerator { protocol, host, namespace }
    }
}

impl Default for DefaultUriGenerator<'_> {
    fn default() -> Self {
        DefaultUriGenerator::new("", "", "")
    }
}

impl<'a> UriGenerator for DefaultUriGenerator<'a> {
    fn base_url(&self) -> String {
        format!("{}://{}:{}", self.protocol, self.host, self.namespace)
    }
}

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

    pub fn make_resource_document(mut self, model: &impl Resourceful, params: &Parameters) -> Document {
        Document {
            content: self.make_resource(model, params).into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: self.included_resources(),
        }
    }

    pub fn make_collection_document<'a, I, R>(mut self, models: I, params: &Parameters) -> Document
    where
        I: IntoIterator<Item=&'a R>,
        R: Resourceful + 'a
    {
        Document {
            content: models
                .into_iter()
                .map(|model|
                    self.make_resource(model, params)
                )
                .collect::<Vec<Resource>>()
                .into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: self.included_resources()
        }
    }

    pub fn make_errors_document<I>(self, errors: Vec<Error>) -> Document {
        Document {
            content: errors.into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: None
        }
    }

    fn implementation_info() -> ImplementationInfo {
        ImplementationInfo {
            version: Some("1.1".to_string()),
            ext: None,
            profile: None,
            meta: None
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