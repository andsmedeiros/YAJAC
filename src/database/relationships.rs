use std::collections::HashMap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Relationship {
    BelongsTo(i32),
    HasOne(i32),
    HasMany(Vec<i32>),
}

pub type Relationships<'a> = HashMap<&'a str, Relationship>;