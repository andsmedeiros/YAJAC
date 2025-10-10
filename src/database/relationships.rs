use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Relationship {
    Unloaded,
    BelongsTo(i32),
    HasOne(i32),
    HasMany(Vec<i32>),
}

impl Default for Relationship {
    fn default() -> Self { Relationship::Unloaded }
}

pub type Relationships<'a> = HashMap<&'a str, Relationship>;