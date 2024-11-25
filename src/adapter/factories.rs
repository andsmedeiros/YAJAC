use crate::{
    adapter::{
        Cache,
        Context,
        Parameters,
        UriGenerator
    },
    http_wrappers::Uri,
    resourceful::{
        Relationships,
        Resourceful,
        related_data::{RelatedData,
                       RelatedCollection,
                       RelatedRecord,
        }
    },
    spec::{
        document::{self, ImplementationInfo, Document},
        error::Error, 
        identifier::Identifier,
        links::Link,
        primary_content::PrimaryContent,
        relationship::{self, Linkage, Relationship},
        resource::{self, Resource}
    }
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc
};

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

fn register_at_cache(resource: Resource, cache: &mut Cache) {
    cache.register(resource);
}

pub(crate) fn link_related_data(related_data: RelatedData, cache: Rc<RefCell<Cache>>) -> Linkage {
    match related_data {
        RelatedData::None => Linkage::Empty,
        RelatedData::One(record) =>
            match record {
                RelatedRecord::Unloaded(id) => Linkage::ToOne(id),
                RelatedRecord::Loaded(record) => {
                    let mut cache = cache.borrow_mut();
                    let id = cache.register(record);
                    Linkage::ToOne(id)
                }
            },
        RelatedData::Many(collection) =>
            match collection {
                RelatedCollection::Unloaded(ids) => Linkage::ToMany(ids),
                RelatedCollection::Loaded(records) => {
                    let ids = records.into_iter().map(|model| {
                        let mut cache = cache.borrow_mut();
                        let id = cache.register(model);
                        id
                    }).collect();
                    Linkage::ToMany(ids)
                }
            }
    }
}

pub(crate) fn link_relationships<G: UriGenerator>(
    identifier: Identifier,
    relationships: Relationships,
    context: &Context<G>
)
    -> HashMap<String, Relationship>
{
    relationships
        .into_iter()
        .map(|(relationship_name, related_data)| {
            let linkage = link_related_data(related_data, context.cache.clone());
            let this = context.uri_generator
                .uri_for_relationship(&identifier, relationship_name.as_str());
            let related = context.uri_generator
                .uri_for_related(&identifier, relationship_name.as_str());

            let relationship = Relationship {
                links: relationship::Links {
                    this: this.into(),
                    related: related.into()
                }.into(),
                data: Some(linkage),
                meta: None
            };

            (relationship_name, relationship)
        })
        .collect()
}

pub fn make_resource<G: UriGenerator>(model: &impl Resourceful, context: &Context<G>) -> Resource {
    if context.cache.borrow().has(&model.identifier()) {
        return context.cache.borrow().get(&model.identifier()).unwrap().clone();
    }

    let attributes = model.attributes(&context);
    let relationships = model.relationships(context);
    let meta = model.meta(&context);
    let links = resource::Links {
        this: context.uri_generator.uri_for_resource(&model.identifier())
    };

    Resource {
        identifier: model.identifier(),
        attributes,
        relationships: relationships.map(|r|
            link_relationships(model.identifier(), r, context)
        ),
        links: links.into(),
        meta
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

fn document_links(uri: &Uri) -> document::Links {
    document::Links {
        this: Link::Uri(uri.clone()).into(),
        related: None,
        described_by: None,
    }
}

fn included_resources<G: UriGenerator>(context: Context<G>) -> Option<Vec<Value>> {
    let cache = context.cache.borrow();
    if cache.is_empty() {
        None
    } else {
        cache.values()
            .into_iter()
            .map(|resource|
                serde_json::to_value(&resource).unwrap()
            )
            .collect::<Vec<_>>()
            .into()
    }
}

pub fn to_document<'a, R, G>(content: impl Into<Content<'a, R>>, uri_generator: G, uri: Uri)
    -> Document
where
    R: Resourceful + 'a,
    G: UriGenerator + 'a
{
    let params = Parameters::new(&uri);
    let cache = Rc::new(RefCell::new(Cache::new()));
    let context = Context::new(cache, params, uri_generator);

    let content: PrimaryContent = match Into::<Content<'a, R>>::into(content) {
        Content::Resource(model) =>
             make_resource(model, &context).into(),
        Content::Collection(collection) => collection
            .into_iter()
            .map(|model| make_resource(model, &context))
            .collect::<Vec<Resource>>()
            .into(),
        Content::Errors(errors) =>
            errors.into()
        
    };

    Document {
        content,
        meta: None,
        jsonapi: implementation_info().into(),
        links: document_links(&uri).into(),
        included: included_resources(context),
    }
}