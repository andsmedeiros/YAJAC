use super::{
    adapters::Adapter as AdapterInterface,
    error::Error,
    schema::{RelatedResource, Relationship, TableSchema},
    table::Table,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct Registry<'a, Adapter: AdapterInterface> {
    contents: HashMap<&'a str, Adapter::Table<'a>>,
}

impl<'sch, Adapter: AdapterInterface> Registry<'sch, Adapter> {
    pub fn try_new(
        connection: Adapter::Connection,
        schema: &'sch [&'sch TableSchema],
    ) -> Result<Self, Error> {
        let connection = Arc::new(Mutex::new(connection));
        let registry = Self {
            contents: Self::validate_schema(schema)?
                .into_iter()
                .map(|(name, schema)| (name, Table::new(schema, connection.clone())))
                .collect(),
        };

        Ok(registry)
    }

    pub fn table(&self, name: &str) -> Result<&Adapter::Table<'sch>, Error> {
        self.contents.get(name).ok_or_else(|| Error::UnknownSchema {
            schema: name.to_string(),
            message: "The requested table is not registered".to_string(),
        })
    }

    fn validate_schema(
        registry_schema: &'sch [&'sch TableSchema],
    ) -> Result<HashMap<&'sch str, &'sch TableSchema<'sch>>, Error> {
        use Relationship::*;

        let schema_registry: HashMap<&'sch str, &'sch TableSchema> = registry_schema
            .iter()
            .map(|schema| (schema.name, *schema))
            .collect();

        for schema in registry_schema {
            for (relationship, descriptor) in schema.relationships {
                match descriptor {
                    BelongsTo(RelatedResource { resource, keys }) => {
                        if !schema.has_foreign_key(keys.own) {
                            Err(Error::SchemaValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: relationship.to_string(),
                                message: format!(
                                    "Relationship refers to non-existent foreign key '{}'",
                                    keys.own
                                ),
                            })?
                        }

                        if let Some(related_schema) = schema_registry.get(resource) {
                            if keys.related != "id" && !related_schema.has_attribute(keys.related) {
                                Err(Error::SchemaValidationFailure {
                                    schema: schema.name.to_string(),
                                    attribute: relationship.to_string(),
                                    message: format!(
                                        "Relationship refers to non-existent related column '{}' at table '{}'",
                                        keys.own, resource
                                    ),
                                })?
                            }
                        } else {
                            Err(Error::SchemaValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: relationship.to_string(),
                                message: format!(
                                    "Relationship refers to non-existent resource '{}'",
                                    resource
                                ),
                            })?
                        }
                    }
                    HasOne(RelatedResource { resource, keys })
                    | HasMany(RelatedResource { resource, keys }) => {
                        if keys.own != "id" && !schema.has_attribute(keys.own) {
                            Err(Error::SchemaValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: relationship.to_string(),
                                message: format!(
                                    "Relationship refers to non-existent attribute '{}'",
                                    keys.own
                                ),
                            })?
                        }

                        if let Some(related_schema) = schema_registry.get(resource) {
                            if !related_schema.has_foreign_key(keys.related) {
                                Err(Error::SchemaValidationFailure {
                                    schema: schema.name.to_string(),
                                    attribute: relationship.to_string(),
                                    message: format!(
                                        "Relationship refers to non-existent foreign key '{}' at table '{}'",
                                        keys.related, resource
                                    ),
                                })?
                            }
                        } else {
                            Err(Error::SchemaValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: relationship.to_string(),
                                message: format!(
                                    "Relationship refers to non-existent resource '{}'",
                                    resource
                                ),
                            })?
                        }
                    }
                }
            }
        }

        Ok(schema_registry)
    }
}
