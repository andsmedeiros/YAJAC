use super::{
    adapters::Adapter as AdapterInterface,
    error::Error,
    pool::Pool as PoolInterface,
    schema::{RelatedResource, Relationship, TableSchema},
    table::Table as TableInterface,
};
use std::collections::HashMap;

pub struct Registry<'sch, Adapter: AdapterInterface> {
    schemas: HashMap<&'sch str, &'sch TableSchema<'sch>>,
    pool: Adapter::Pool,
}

impl<'sch, Adapter: AdapterInterface> Registry<'sch, Adapter> {
    pub fn try_new(pool: Adapter::Pool, schema: &'sch [&'sch TableSchema]) -> Result<Self, Error> {
        let schemas = Self::validate_schema(schema)?;

        Ok(Self { schemas, pool })
    }

    pub fn schema(&self, name: &str) -> Result<&'sch TableSchema<'sch>, Error> {
        self.schemas
            .get(name)
            .copied()
            .ok_or_else(|| Error::UnknownSchema {
                schema: name.to_string(),
                message: "The requested table is not registered".to_string(),
            })
    }

    /// Acquires a connection from the pool, held for the request.
    pub fn acquire(&self) -> Result<<Adapter::Pool as PoolInterface>::Handle<'_>, Error> {
        self.pool.acquire()
    }

    /// Builds a request-scoped table bound to `connection`.
    pub fn table<'req>(
        &self,
        name: &str,
        connection: &'req Adapter::Connection,
    ) -> Result<Adapter::Table<'sch, 'req>, Error> {
        Ok(Adapter::Table::new(self.schema(name)?, connection))
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
