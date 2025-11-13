use std::collections::HashMap;
use super::attributes::Identifier;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Relationship {
    BelongsTo(Identifier),
    HasOne(Identifier),
    HasMany(Vec<Identifier>),
}

pub type Relationships<'a> = HashMap<&'a str, Relationship>;