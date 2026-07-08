use super::{
    error::Error,
    schema::{
        AttributeType, RelatedResource, RelationshipKind, Schema, SchemaBuilder, SchemaParts,
    },
};
use std::collections::HashMap;

/// An immutable, validated collection of schemas keyed by resource type. Holds no
/// storage: binding a registry to a connection pool is `ConnectionManager`'s job.
pub struct Registry<'sch> {
    schemas: HashMap<&'sch str, Schema<'sch>>,
}

impl<'sch> Registry<'sch> {
    /// Builds the registry from schema builders: decomposes them into their raw
    /// parts and hands them to `try_build`, which validates the set and mints the
    /// schemas — the sole path by which a `Schema` comes into being.
    pub fn try_new(builders: impl IntoIterator<Item = SchemaBuilder<'sch>>) -> Result<Self, Error> {
        let parts = builders
            .into_iter()
            .map(SchemaBuilder::into_parts)
            .collect();

        Ok(Self {
            schemas: try_build(parts)?,
        })
    }

    pub fn schema(&self, name: &str) -> Result<&Schema<'sch>, Error> {
        self.schemas.get(name).ok_or_else(|| Error::UnknownSchema {
            schema: name.to_string(),
            message: "The requested table is not registered".to_string(),
        })
    }
}

/// Validates the schema set and mints it. Returns the built schemas iff every
/// invariant holds, so an inconsistent set never yields a `Schema`.
fn try_build<'sch>(
    parts: Vec<SchemaParts<'sch>>,
) -> Result<HashMap<&'sch str, Schema<'sch>>, Error> {
    let mut registry: HashMap<&'sch str, SchemaParts<'sch>> = HashMap::with_capacity(parts.len());

    // Resource types are unique across the set.
    for schema in parts {
        let name = schema.name;
        if registry.insert(name, schema).is_some() {
            return Err(Error::InconsistentSchema {
                schema: name.to_string(),
                attribute: name.to_string(),
                message: "Resource type is declared more than once".to_string(),
            });
        }
    }

    for schema in registry.values() {
        validate_schema(schema)?;
        validate_relationships(schema, &registry)?;
    }

    Ok(registry
        .into_iter()
        .map(|(name, schema)| (name, Schema::new(schema)))
        .collect())
}

/// Intra-schema invariants: a column name denotes at most one of the primary
/// key, an attribute, or a foreign key; attributes and relationships share the
/// JSON:API "fields" namespace; and `type`/`id` are reserved field names.
fn validate_schema(schema: &SchemaParts) -> Result<(), Error> {
    let primary_key = schema.primary_key.name;
    if schema.attributes.contains_key(primary_key) || schema.foreign_keys.contains_key(primary_key)
    {
        return Err(Error::InconsistentSchema {
            schema: schema.name.to_string(),
            attribute: primary_key.to_string(),
            message: "Primary key name is also declared as a column".to_string(),
        });
    }

    for &name in schema.attributes.keys() {
        if schema.foreign_keys.contains_key(name) {
            return Err(Error::InconsistentSchema {
                schema: schema.name.to_string(),
                attribute: name.to_string(),
                message: "Column is declared as both an attribute and a foreign key".to_string(),
            });
        }
    }

    for &name in schema.relationships.keys() {
        if schema.attributes.contains_key(name) {
            return Err(Error::InconsistentSchema {
                schema: schema.name.to_string(),
                attribute: name.to_string(),
                message: "Name is declared as both an attribute and a relationship".to_string(),
            });
        }
    }

    for &name in schema.attributes.keys().chain(schema.relationships.keys()) {
        if name == "type" || name == "id" {
            return Err(Error::InconsistentSchema {
                schema: schema.name.to_string(),
                attribute: name.to_string(),
                message: "'type' and 'id' are reserved and cannot name a field".to_string(),
            });
        }
    }

    Ok(())
}

/// Cross-schema invariants: each relationship's owning and referenced keys exist
/// on their respective tables (the primary key matched by its declared name, not
/// a hardcoded "id"), the related resource is registered, and the two join
/// columns share a type.
fn validate_relationships<'sch>(
    schema: &SchemaParts<'sch>,
    registry: &HashMap<&'sch str, SchemaParts<'sch>>,
) -> Result<(), Error> {
    for (&relationship, descriptor) in &schema.relationships {
        let RelatedResource { resource, keys } = &descriptor.related;
        match descriptor.kind {
            RelationshipKind::BelongsTo => {
                let Some(own_column) = schema.foreign_keys.get(keys.own) else {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship refers to non-existent foreign key '{}'",
                            keys.own
                        ),
                    });
                };

                let Some(related) = registry.get(resource) else {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship refers to non-existent resource '{resource}'"
                        ),
                    });
                };

                let related_type = if keys.related == related.primary_key.name {
                    AttributeType::from(related.primary_key.kind)
                } else if let Some(column) = related.attributes.get(keys.related) {
                    column.kind
                } else {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship refers to non-existent related column '{}' at table '{resource}'",
                            keys.related
                        ),
                    });
                };

                if own_column.kind != related_type {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship join columns have mismatched types: '{}' is {} but '{}' is {related_type}",
                            keys.own, own_column.kind, keys.related
                        ),
                    });
                }
            }
            RelationshipKind::HasOne | RelationshipKind::HasMany => {
                let own_type = if keys.own == schema.primary_key.name {
                    AttributeType::from(schema.primary_key.kind)
                } else if let Some(column) = schema.attributes.get(keys.own) {
                    column.kind
                } else {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship refers to non-existent attribute '{}'",
                            keys.own
                        ),
                    });
                };

                let Some(related) = registry.get(resource) else {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship refers to non-existent resource '{resource}'"
                        ),
                    });
                };

                let Some(related_column) = related.foreign_keys.get(keys.related) else {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship refers to non-existent foreign key '{}' at table '{resource}'",
                            keys.related
                        ),
                    });
                };

                if own_type != related_column.kind {
                    return Err(Error::InconsistentSchema {
                        schema: schema.name.to_string(),
                        attribute: relationship.to_string(),
                        message: format!(
                            "Relationship join columns have mismatched types: '{}' is {own_type} but '{}' is {}",
                            keys.own, keys.related, related_column.kind
                        ),
                    });
                }
            }
        }
    }

    Ok(())
}
