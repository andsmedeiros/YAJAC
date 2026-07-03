use super::{
    AttributeType, Column, IdentifierType, PrimaryKey, RelatedResource, Relationship,
    RelationshipKeys, SchemaParts,
};
use indexmap::IndexMap;

/// Fluent construction of a relationship's target. `to` names the related
/// resource; the join columns follow, labelled by which side carries the
/// foreign key: `pointing_own`/`to_related` when the key is on our table,
/// `pointing_related`/`to_own` when it is on theirs. Both keys are mandatory.
pub struct Related<'sch> {
    resource: &'sch str,
}

pub struct PointingOwn<'sch> {
    resource: &'sch str,
    own: &'sch str,
}

pub struct PointingRelated<'sch> {
    resource: &'sch str,
    related: &'sch str,
}

impl<'sch> Related<'sch> {
    pub fn to(resource: &'sch str) -> Self {
        Self { resource }
    }

    pub fn pointing_own(self, own: &'sch str) -> PointingOwn<'sch> {
        PointingOwn {
            resource: self.resource,
            own,
        }
    }

    pub fn pointing_related(self, related: &'sch str) -> PointingRelated<'sch> {
        PointingRelated {
            resource: self.resource,
            related,
        }
    }
}

impl<'sch> PointingOwn<'sch> {
    pub fn to_related(self, related: &'sch str) -> RelatedResource<'sch> {
        RelatedResource {
            resource: self.resource,
            keys: RelationshipKeys {
                own: self.own,
                related,
            },
        }
    }
}

impl<'sch> PointingRelated<'sch> {
    pub fn to_own(self, own: &'sch str) -> RelatedResource<'sch> {
        RelatedResource {
            resource: self.resource,
            keys: RelationshipKeys {
                own,
                related: self.related,
            },
        }
    }
}

/// Fluent, insertion-ordered collection of a table's schema. Defaults to an
/// integer `id` primary key and no text index. It only accumulates: `into_parts`
/// hands the raw content to the registry, which validates and builds the schema.
pub struct SchemaBuilder<'sch> {
    parts: SchemaParts<'sch>,
}

impl<'sch> SchemaBuilder<'sch> {
    pub fn table(name: &'sch str) -> Self {
        Self {
            parts: SchemaParts {
                name,
                primary_key: PrimaryKey {
                    name: "id",
                    kind: IdentifierType::Integer,
                },
                attributes: IndexMap::new(),
                foreign_keys: IndexMap::new(),
                relationships: IndexMap::new(),
                text_index: false,
            },
        }
    }

    pub fn primary_key(mut self, name: &'sch str, kind: IdentifierType) -> Self {
        self.parts.primary_key = PrimaryKey { name, kind };
        self
    }

    pub fn attribute(mut self, name: &'sch str, kind: AttributeType) -> Self {
        self.parts.attributes.insert(name, Column { kind });
        self
    }

    pub fn foreign_key(mut self, name: &'sch str, kind: AttributeType) -> Self {
        self.parts.foreign_keys.insert(name, Column { kind });
        self
    }

    pub fn belongs_to(mut self, name: &'sch str, related: RelatedResource<'sch>) -> Self {
        self.parts
            .relationships
            .insert(name, Relationship::BelongsTo(related));
        self
    }

    pub fn has_one(mut self, name: &'sch str, related: RelatedResource<'sch>) -> Self {
        self.parts
            .relationships
            .insert(name, Relationship::HasOne(related));
        self
    }

    pub fn has_many(mut self, name: &'sch str, related: RelatedResource<'sch>) -> Self {
        self.parts
            .relationships
            .insert(name, Relationship::HasMany(related));
        self
    }

    pub fn text_index(mut self) -> Self {
        self.parts.text_index = true;
        self
    }

    pub(crate) fn into_parts(self) -> SchemaParts<'sch> {
        self.parts
    }
}

#[cfg(test)]
mod tests {
    use crate::database::schema::*;
    use AttributeType::*;
    use indexmap::IndexMap;

    #[test]
    fn test_schema_builder_collects_parts() {
        let parts = products().into_parts();

        assert_eq!(parts.name, "products");
        assert_eq!(
            parts.primary_key,
            PrimaryKey {
                name: "id",
                kind: IdentifierType::Integer,
            }
        );
        assert_eq!(
            parts.attributes,
            IndexMap::from([
                ("name", Column { kind: Text }),
                ("price", Column { kind: Float })
            ])
        );
        assert_eq!(
            parts.foreign_keys,
            IndexMap::from([("category_id", Column { kind: Integer })])
        );
        assert_eq!(
            parts.relationships,
            IndexMap::from([
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
            ])
        );
        assert!(parts.text_index);
    }
}
