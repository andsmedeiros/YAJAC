use crate::{
    adapter::{Adapter, Parameters, UriGenerator},
    spec::{
        identifier::Identifier,
        resource::Resource,
    }
};

use serde_json::Value;
use std::collections::HashMap;

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

pub type Attributes = HashMap<String, Value>;
pub type Relationships = HashMap<String, RelatedData>;
pub type Meta = HashMap<String, Value>;

pub trait Resourceful {
    fn kind(&self) -> &'static str;
    fn identifier(&self) -> Identifier;

    fn attributes(&self, _params: &Parameters) -> Option<Attributes> { None }
    fn relationships<G: UriGenerator>(&self, _adapter: &mut Adapter<G>, _params: &Parameters)
        -> Option<Relationships> { None }
    fn meta(&self, _params: &Parameters) -> Option<Meta> { None }
}