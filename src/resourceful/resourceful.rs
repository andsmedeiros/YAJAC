use crate::{
    adapter::{Context, UriGenerator},
    spec::identifier::Identifier,
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

    fn attributes<G: UriGenerator>(&self, _context: &Context<G>)
        -> Option<Attributes> { None }
    fn relationships<G: UriGenerator>(&self, _context: &mut Context<G>)
        -> Option<Relationships> { None }
    fn meta<G: UriGenerator>(&self, _context: &Context<G>) -> Option<Meta> { None }
}