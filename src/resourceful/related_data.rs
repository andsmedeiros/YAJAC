use crate::{
    spec::{
        identifier::Identifier,
        resource::Resource,
    }
};

pub enum RelatedRecord {
    Unloaded(Identifier),
    Loaded(Resource)
}

pub enum RelatedCollection {
    Unloaded(Vec<Identifier>),
    Loaded(Vec<Resource>)
}

pub enum RelatedData {
    None,
    One(RelatedRecord),
    Many(RelatedCollection),
}

impl Default for RelatedData {
    fn default() -> Self { RelatedData::None }
}

impl From<Identifier> for RelatedData {
    fn from(identifier: Identifier) -> Self {
        RelatedData::One(RelatedRecord::Unloaded(identifier))
    }
}

impl From<Vec<Identifier>> for RelatedData {
    fn from(identifiers: Vec<Identifier>) -> Self {
        RelatedData::Many(RelatedCollection::Unloaded(identifiers))
    }
}

impl From<Resource> for RelatedData {
    fn from(resource: Resource) -> Self {
        RelatedData::One(RelatedRecord::Loaded(resource))
    }
}

impl From<Vec<Resource>> for RelatedData {
    fn from(resources: Vec<Resource>) -> Self {
        RelatedData::Many(RelatedCollection::Loaded(resources))
    }
}