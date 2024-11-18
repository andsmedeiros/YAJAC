use crate::{
    adapter::{
        context::Context,
        parameters::Parameters,
    },
    http_wrappers::Uri,
    resourceful::Resourceful,
    spec::{
        document::Document, 
        error::Error, 
        identifier::Identifier,
        primary_content::PrimaryContent, 
        resource::Resource
    }
};
use std::collections::HashMap;

enum Content<'a, R: Resourceful> {
    Resource(&'a R),
    Collection(Vec<&'a R>),
    Errors(Vec<Error>)
}

impl<'a, R: Resourceful> From<&'a R> for Content<'a, R> {
    fn from(resourceful: &'a R) -> Self {
        Content::Resource(resourceful)
    }
}

impl<'a, R: Resourceful> From<&'a Vec<R>> for Content<'a, R> {
    fn from(collection: &'a Vec<R>) -> Self {
        Content::Collection(collection.iter().collect())
    }
}

impl<'a, R: Resourceful> From<Vec<Error>> for Content<'a, R> {
    fn from(errors: Vec<Error>) -> Self {
        Content::Errors(errors)
    }
}

pub fn make_resource(model: &impl Resourceful, params: &Parameters, cache: &HashMap<Identifier, Resource>) -> Resource {
    if cache.contains_key(&model.identifier()) {
        return cache.get(&model.identifier()).unwrap().clone();
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

pub fn to_document<'a, R: Resourceful + 'a>(content: impl Into<Content<'a, R>>, uri: Uri) -> Document {
    let params = Parameters::new(&uri);
    let cache = HashMap::<Identifier, Resource>::new();
    let content: PrimaryContent = match Into::<Content<'a, R>>::into(content) {
        Content::Resource(model) =>
             make_resource(model, &params, &cache).into(),
        Content::Collection(collection) => collection
            .into_iter()
            .map(|model| make_resource(model, &params, &cache))
            .collect::<Vec<Resource>>()
            .into(),
        Content::Errors(errors) =>
            errors.into()
        
    };
    Document {
        content: self.,
        meta: None,
        jsonapi: self.implementation_info().into(),
        links: self.document_links().into(),
        included: self.included_resources(),
    }
}