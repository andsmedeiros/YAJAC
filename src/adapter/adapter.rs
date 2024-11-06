use crate::{
    adapter::Parameters,
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
use std::collections::HashMap;

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
    uri_generator: G,
}

impl<G: UriGenerator> Adapter<G> {
    pub fn new(uri_generator: G) -> Self {
        Self {
            cache: HashMap::new(),
            uri_generator
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

    fn link_related_data(&mut self, related_data: RelatedData) -> Linkage {
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
    
    fn link_relationships(&mut self, identifier: Identifier, relationships: Relationships)
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

    pub fn make_resource(&mut self, model: &impl Resourceful, params: &Parameters) -> Resource {
         Resource {
            identifier: model.identifier(),
            attributes: model.attributes(&params),
            relationships: model.relationships(self, &params)
                .map(|relationships| 
                    self.link_relationships(model.identifier(), relationships)
                ),
            links: resource::Links {
                this: self.uri_generator.uri_for_resource(&model.identifier())
            }.into(),
            meta: model.meta(&params)
        }
    }

    pub fn make_resource_document(&mut self, model: &impl Resourceful, params: &Parameters) -> Document {
        Document {
            content: self.make_resource(model, params).into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: None,
        }
    }

    pub fn make_collection_document<'a, I, R>(&mut self, models: I, params: &Parameters) -> Document
    where
        I: IntoIterator<Item=&'a R>,
        R: Resourceful + 'a
    {
        Document {
            content: models
                .into_iter()
                .map(|model| self.make_resource(model, params))
                .collect::<Vec<Resource>>()
                .into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: None,
        }
    }

    pub fn make_errors_document<I>(&self, errors: Vec<Error>) -> Document {
        Document {
            content: errors.into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: None
        }
    }
}