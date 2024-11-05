use crate::{
    http_wrappers::Uri, 
    resourceful::{RelatedData, Resourceful}, 
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

pub struct Adapter<G: UriGenerator> {    
    cache: HashMap<Identifier, Resource>,
    uri_generator: G,
}

impl<G: UriGenerator> Adapter<G> {
    
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
            RelatedData::One(model) => {
                let id = model.identifier.clone();
                self.cache.insert(model.identifier.clone(), model);
                Linkage::ToOne(id)
            },
            RelatedData::Many(collection) => {
                let ids = collection.into_iter().map(|model| {
                    let id = model.identifier.clone();
                    self.cache.insert(model.identifier.clone(), model);
                    id
                }).collect();
                Linkage::ToMany(ids)
            }
        }
    }
    
    fn link_relationships(&mut self, identifier: Identifier, relationships: HashMap<String, RelatedData>)
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

    pub fn make_resource(&mut self, model: &impl Resourceful) -> Resource {
         Resource {
            identifier: model.identifier(),
            attributes: model.attributes(),
            relationships: model.relationships(self)
                .map(|relationships| 
                    self.link_relationships(model.identifier(), relationships)
                ),
            links: resource::Links {
                this: self.uri_generator.uri_for_resource(&model.identifier())
            }.into(),
            meta: model.meta()
        }
    }

    pub fn make_resource_document(&mut self, model: &impl Resourceful) -> Document {
        Document {
            content: self.make_resource(model).into(),
            meta: None,
            jsonapi: Self::implementation_info().into(),
            links: None,
            included: None,
        }
    }

    pub fn make_collection_document<'a, I, R>(&mut self, models: I) -> Document 
    where
        I: IntoIterator<Item=&'a R>,
        R: Resourceful + 'a
    {
        Document {
            content: models
                .into_iter()
                .map(|model| self.make_resource(model))
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