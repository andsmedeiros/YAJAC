use crate::database::error::Error;
use indexmap::IndexMap;
use std::fmt::Display;

pub mod builder;

pub use builder::{PointingOwn, PointingRelated, Related, SchemaBuilder};

pub type DateTime = chrono::DateTime<chrono::Utc>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IdentifierType {
    Text,
    Integer,
}

impl Display for IdentifierType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeType {
    Text,
    Integer,
    Float,
    Boolean,
    DateTime,
}

impl Display for AttributeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<IdentifierType> for AttributeType {
    fn from(kind: IdentifierType) -> Self {
        match kind {
            IdentifierType::Integer => AttributeType::Integer,
            IdentifierType::Text => AttributeType::Text,
        }
    }
}

/// A stored column's metadata. The sole extension point for per-column facts;
/// today it carries only its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Column {
    pub kind: AttributeType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimaryKey<'sch> {
    pub name: &'sch str,
    pub kind: IdentifierType,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelationshipKeys<'sch> {
    pub own: &'sch str,
    pub related: &'sch str,
}

impl Display for RelationshipKeys<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelatedResource<'sch> {
    pub resource: &'sch str,
    pub keys: RelationshipKeys<'sch>,
}

impl Display for RelatedResource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relationship<'sch> {
    BelongsTo(RelatedResource<'sch>),
    HasMany(RelatedResource<'sch>),
    HasOne(RelatedResource<'sch>),
}

impl<'sch> Relationship<'sch> {
    pub fn related_resource(&self) -> &RelatedResource<'_> {
        match self {
            Relationship::BelongsTo(related_resource)
            | Relationship::HasMany(related_resource)
            | Relationship::HasOne(related_resource) => related_resource,
        }
    }
}

impl Display for Relationship<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// The inert, unvalidated extract of a `SchemaBuilder`. The registry reads it to
/// validate cross-schema, then mints a `Schema` from it; it is the only
/// path to a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaParts<'sch> {
    pub name: &'sch str,
    pub primary_key: PrimaryKey<'sch>,
    pub attributes: IndexMap<&'sch str, Column>,
    pub foreign_keys: IndexMap<&'sch str, Column>,
    pub relationships: IndexMap<&'sch str, Relationship<'sch>>,
    pub text_index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema<'sch> {
    name: &'sch str,
    primary_key: PrimaryKey<'sch>,
    attributes: IndexMap<&'sch str, Column>,
    foreign_keys: IndexMap<&'sch str, Column>,
    relationships: IndexMap<&'sch str, Relationship<'sch>>,
    text_index: bool,
}

impl<'sch> Schema<'sch> {
    /// Mints a validated schema from a builder's extract. Restricted to the
    /// crate so construction always flows through the registry's validation.
    pub(crate) fn new(parts: SchemaParts<'sch>) -> Self {
        Self {
            name: parts.name,
            primary_key: parts.primary_key,
            attributes: parts.attributes,
            foreign_keys: parts.foreign_keys,
            relationships: parts.relationships,
            text_index: parts.text_index,
        }
    }

    pub fn name(&self) -> &'sch str {
        self.name
    }

    pub fn primary_key(&self) -> PrimaryKey<'sch> {
        self.primary_key
    }

    pub fn text_index(&self) -> bool {
        self.text_index
    }

    // The `&'sch self` receiver lends the borrowed values out of the owned maps
    // for `'sch`; every caller holds the schema behind a `&'sch` reference.
    pub fn attributes(&'sch self) -> impl Iterator<Item = (&'sch str, &'sch AttributeType)> {
        self.attributes
            .iter()
            .map(|(name, column)| (*name, &column.kind))
    }

    pub fn foreign_keys(&'sch self) -> impl Iterator<Item = (&'sch str, &'sch AttributeType)> {
        self.foreign_keys
            .iter()
            .map(|(name, column)| (*name, &column.kind))
    }

    pub fn relationships(
        &'sch self,
    ) -> impl Iterator<Item = (&'sch str, &'sch Relationship<'sch>)> {
        self.relationships
            .iter()
            .map(|(name, relationship)| (*name, relationship))
    }

    pub fn attribute(&self, attribute_name: &str) -> Option<AttributeType> {
        self.attributes
            .get(attribute_name)
            .map(|column| column.kind)
    }

    pub fn foreign_key(&self, foreign_key_name: &str) -> Option<AttributeType> {
        self.foreign_keys
            .get(foreign_key_name)
            .map(|column| column.kind)
    }

    pub fn relationship(&self, relationship_name: &str) -> Option<&Relationship<'sch>> {
        self.relationships.get(relationship_name)
    }

    pub fn is_primary_key(&self, attribute_name: &str) -> bool {
        self.primary_key.name == attribute_name
    }

    pub fn has_attribute(&self, column_name: &str) -> bool {
        self.attributes.contains_key(column_name)
    }

    pub fn has_foreign_key(&self, foreign_key_name: &str) -> bool {
        self.foreign_keys.contains_key(foreign_key_name)
    }

    pub fn has_relationship(&self, relationship_name: &str) -> bool {
        self.relationships.contains_key(relationship_name)
    }

    pub fn fields(&self) -> impl Iterator<Item = &'sch str> {
        let columns = self.attributes.keys().copied();
        let relationships = self.relationships.keys().copied();

        columns.chain(relationships)
    }

    pub fn attribute_type(&self, name: &str) -> Result<AttributeType, Error> {
        if self.is_primary_key(name) {
            Ok(AttributeType::from(self.primary_key.kind))
        } else {
            self.attribute(name)
                .or_else(|| self.foreign_key(name))
                .ok_or_else(|| Error::InvalidAttributeAccess {
                    schema: self.name.to_string(),
                    attribute: name.to_string(),
                })
        }
    }
}

/// Shared fixture: a `products` schema exercising every builder facility.
#[cfg(test)]
pub(crate) fn products() -> SchemaBuilder<'static> {
    SchemaBuilder::table("products")
        .attribute("name", AttributeType::Text)
        .attribute("price", AttributeType::Float)
        .foreign_key("category_id", AttributeType::Integer)
        .belongs_to(
            "category",
            Related::to("categories")
                .pointing_own("category_id")
                .to_related("id"),
        )
        .has_many(
            "variants",
            Related::to("variants")
                .pointing_related("product_id")
                .to_own("id"),
        )
        .has_one(
            "position",
            Related::to("display_positions")
                .pointing_related("product_id")
                .to_own("id"),
        )
        .text_index()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use AttributeType::*;
    use std::collections::HashSet;

    #[test]
    fn test_attribute_type_display() {
        assert_eq!(Text.to_string(), "Text");
        assert_eq!(Integer.to_string(), "Integer");
        assert_eq!(Float.to_string(), "Float");
        assert_eq!(Boolean.to_string(), "Boolean");
        assert_eq!(DateTime.to_string(), "DateTime");
    }

    #[test]
    fn test_table_schema_column_operations() {
        let schema = Schema::new(products().into_parts());

        assert_eq!(schema.attribute("name"), Some(Text));
        assert_eq!(schema.attribute("price"), Some(Float));
        assert_eq!(schema.foreign_key("category_id"), Some(Integer));
        assert_eq!(
            schema.relationship("category"),
            Some(&Relationship::BelongsTo(RelatedResource {
                resource: "categories",
                keys: RelationshipKeys {
                    own: "category_id",
                    related: "id"
                }
            }))
        );

        assert_eq!(schema.attribute("nonexistent"), None);
        assert_eq!(schema.foreign_key("nonexistent"), None);
        assert_eq!(schema.relationship("nonexistent"), None);

        assert!(!schema.has_attribute("id"));
        assert!(!schema.has_attribute("nonexistent"));
        assert!(schema.has_foreign_key("category_id"));
        assert!(!schema.has_foreign_key("nonexistent"));
        assert!(schema.has_relationship("category"));
        assert!(!schema.has_relationship("nonexistent"));

        assert_eq!(
            schema.fields().collect::<HashSet<_>>(),
            HashSet::from_iter(["name", "price", "category", "variants", "position"])
        );
    }
}
