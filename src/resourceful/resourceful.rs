use crate::{
    adapter::{Adapter, Parameters, UriGenerator},
    spec::{
        identifier::Identifier,
    }
};
use super::related_data::RelatedData;

use serde_json::Value;
use std::collections::HashMap;

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