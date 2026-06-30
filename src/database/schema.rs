use crate::database::error::Error;
use std::fmt::Display;

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TableSchema<'sch> {
    pub name: &'sch str,
    pub primary_key: PrimaryKey<'sch>,
    pub attributes: &'sch [(&'sch str, AttributeType)],
    pub foreign_keys: &'sch [(&'sch str, AttributeType)],
    pub relationships: &'sch [(&'sch str, Relationship<'sch>)],
    pub text_index: bool,
}

fn find<'sch, 'req, T: 'sch>(
    collection: &'sch [(&'sch str, T)],
    name: &'req str,
) -> Option<&'sch T> {
    collection
        .iter()
        .find_map(|(key, value)| if *key == name { Some(value) } else { None })
}

impl<'sch> TableSchema<'sch> {
    pub fn attribute(&self, attribute_name: &str) -> Option<AttributeType> {
        find(self.attributes, attribute_name).copied()
    }

    pub fn foreign_key(&self, foreign_key_name: &str) -> Option<AttributeType> {
        find(self.foreign_keys, foreign_key_name).copied()
    }

    pub fn relationship(&self, relationship_name: &str) -> Option<&Relationship<'sch>> {
        find(self.relationships, relationship_name)
    }

    pub fn is_primary_key(&self, attribute_name: &str) -> bool {
        self.primary_key.name == attribute_name
    }

    pub fn has_attribute(&self, column_name: &str) -> bool {
        self.attributes.iter().any(|(name, _)| *name == column_name)
    }

    pub fn has_foreign_key(&self, foreign_key_name: &str) -> bool {
        self.foreign_keys
            .iter()
            .any(|(name, _)| *name == foreign_key_name)
    }

    pub fn has_relationship(&self, relationship_name: &str) -> bool {
        self.relationships
            .iter()
            .any(|(name, _)| *name == relationship_name)
    }

    pub fn fields(&self) -> impl Iterator<Item = &'sch str> {
        let columns = self.attributes.iter().map(|(name, _)| *name);
        let relationships = self.relationships.iter().map(|(name, _)| *name);

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
        let schema = TableSchema {
            name: "products",
            primary_key: PrimaryKey {
                name: "id",
                kind: IdentifierType::Integer,
            },
            attributes: &[("name", Text), ("price", Float)],
            foreign_keys: &[("category_id", Integer)],
            relationships: &[
                (
                    "category",
                    Relationship::BelongsTo(RelatedResource {
                        resource: "categories",
                        keys: RelationshipKeys {
                            own: "category_id",
                            related: "id",
                        },
                    }),
                ),
                (
                    "variants",
                    Relationship::HasMany(RelatedResource {
                        resource: "variants",
                        keys: RelationshipKeys {
                            own: "id",
                            related: "product_id",
                        },
                    }),
                ),
                (
                    "position",
                    Relationship::HasOne(RelatedResource {
                        resource: "display_positions",
                        keys: RelationshipKeys {
                            own: "id",
                            related: "product_id",
                        },
                    }),
                ),
            ],
            text_index: true,
        };

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
