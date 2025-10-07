use std::fmt::Display;

pub type DateTime = chrono::DateTime<chrono::Utc>;

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone)]
pub struct RelationshipColumns {
    pub own: &'static str,
    pub related: &'static str,
}

impl Display for RelationshipColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct RelatedTable {
    pub table: &'static str,
    pub columns: RelationshipColumns,
}

impl Display for RelatedTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub enum Relationship {
    BelongsTo(RelatedTable),
    HasMany(RelatedTable),
    HasOne(RelatedTable),
}

impl Display for Relationship {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: &'static str,
    pub columns: &'static[(&'static str, AttributeType)],
    pub relationships: &'static[(&'static str, Relationship)],
    pub text_index: bool
}

impl TableSchema {
    pub fn column(&self, column_name: &str) -> Option<&AttributeType> {
        self.columns
            .iter()
            .find(|column| column.0 == column_name)
            .map(|column| &column.1)
    }

    pub fn relationship(&self, relationship_name: &str) -> Option<&Relationship> {
        self.relationships
            .iter()
            .find(|relationship| relationship.0 == relationship_name)
            .map(|relationship| &relationship.1)
    }

    pub fn has_column(&self, column_name: &str) -> bool {
        self.columns
            .iter()
            .any(|column| column.0 == column_name)
    }

    pub fn has_relationship(&self, relationship_name: &str) -> bool {
        self.relationships
            .iter()
            .any(|relationship| relationship.0 == relationship_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AttributeType::*;

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
            columns: &[
                ("id", Integer),
                ("name", Text),
                ("price", Float),
            ],
            relationships: &[
                ("category", Relationship::BelongsTo(RelatedTable {
                    table: "categories",
                    columns: RelationshipColumns { own: "category_id", related: "id" }
                })),
                ("variants", Relationship::HasMany(RelatedTable {
                    table: "variants",
                    columns: RelationshipColumns { own: "id", related: "product_id" }
                })),
                ("position", Relationship::HasOne(RelatedTable {
                    table: "display_positions",
                    columns: RelationshipColumns { own: "id", related: "product_id" }
                }))
            ],
            text_index: true,
        };

        assert_eq!(schema.column("id"), Some(&Integer));
        assert_eq!(schema.column("name"), Some(&Text));
        assert_eq!(
            schema.relationship("category").unwrap().to_string(),
            Relationship::BelongsTo(RelatedTable {
                table: "categories",
                columns: RelationshipColumns { own: "category_id", related: "id" }
            }).to_string()
        );

        assert!(schema.column("nonexistent").is_none());
        assert!(schema.relationship("nonexistent").is_none());

        assert!(schema.has_column("id"));
        assert!(!schema.has_column("nonexistent"));
        assert!(schema.has_relationship("category"));
        assert!(!schema.has_relationship("nonexistent"));
    }

    #[test]
    fn test_empty_schema() {
        let schema = TableSchema {
            name: "empty",
            columns: &[],
            relationships: &[],
            text_index: false,
        };

        assert!(schema.column("anything").is_none());
        assert!(schema.relationship("anything").is_none());
        assert!(!schema.has_column("anything"));
        assert!(!schema.has_relationship("anything"));
    }
}