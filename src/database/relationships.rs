use super::attributes::Identifier;
use std::collections::HashMap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Relationship {
    BelongsTo(Identifier),
    HasOne(Identifier),
    HasMany(Vec<Identifier>),
    Empty,
}

pub type Relationships<'sch> = HashMap<&'sch str, Relationship>;
