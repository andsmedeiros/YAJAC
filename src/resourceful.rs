use crate::{
    adapter::{Adapter, UriGenerator},
    spec::{
        identifier::Identifier,
        resource::Resource,
    }
};

use serde_json::Value;
use std::collections::HashMap;

pub enum RelatedData {
    None,
    One(Resource),
    Many(Vec<Resource>),
}

pub type PossibleAttributes = Option<HashMap<String, Value>>;
pub type PossibleRelationships = Option<HashMap<String, RelatedData>>;
pub type PossibleMeta = Option<HashMap<String, Value>>;

pub trait Resourceful {
    fn kind(&self) -> &'static str;
    fn identifier(&self) -> Identifier;

    fn attributes(&self) -> PossibleAttributes { None }
    fn relationships<G: UriGenerator>(&self, _adapter: &mut Adapter<G>) 
        -> PossibleRelationships { None }
    fn meta(&self) -> PossibleMeta { None }
}