use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use super::{
    adapters::Adapter as AdapterInterface,
    error::Error,
    table::Table,
    schema::{RelatedTable, Relationship, TableSchema},
};

pub struct Registry<'a, Adapter : AdapterInterface> {
    connection: Arc<Mutex<Adapter::Connection>>,
    contents: HashMap<&'a str, Adapter::Table<'a>>
}

impl<'a, Adapter : AdapterInterface> Registry<'a, Adapter> {
    pub fn try_new(connection: Adapter::Connection, schema: &'a [&'a TableSchema]) -> Result<Self, Error> {
        let mut connection = Arc::new(Mutex::new(connection));
        let registry = Self {
            connection: connection.clone(),
            contents: Self::validate_schema(schema)?
                .into_iter()
                .map(|(name, schema)| {
                    (name, Table::new(schema, connection.clone()))
                })
                .collect()
        };

        Ok(registry)
    }

    pub fn table(&self, name: &str) -> Result<&Adapter::Table<'a>, Error> {
        self.contents
            .get(name)
            .ok_or_else(|| Error::UnknownSchema {
                schema: name.to_string(),
                message: "The requested table is not registered".to_string()
            })
    }

    fn validate_schema(schema: &'a [&'a TableSchema]) -> Result<HashMap<&'a str, &'a TableSchema>, Error> {
        use Relationship::*;

        let validated_tables : HashMap<&'a str, &'a TableSchema> = schema
            .iter()
            .map(|schema| (schema.name, *schema))
            .collect();

        for table_schema in schema {
            for (relationship_name, relationship) in table_schema.relationships {
                let (related_table_name, relationship_columns) = match relationship {
                    HasOne(RelatedTable { table, columns }) |
                    HasMany(RelatedTable { table, columns }) |
                    BelongsTo(RelatedTable { table, columns })
                        => (table, columns)
                };

                if !table_schema.columns
                    .iter()
                    .any(|(name, _)| *name == relationship_columns.own)
                {
                    return Err(Error::SchemaValidationFailure {
                        schema: table_schema.name.to_string(),
                        attribute: relationship_name.to_string(),
                        message: format!(
                            "Relationship refers to non-existent own column '{}'",
                            relationship_columns.own
                        )
                    })
                }

                if let Some(related_schema) = validated_tables.get(related_table_name) {
                    if ! related_schema.columns
                        .iter()
                        .any(|(name, _)| *name == relationship_columns.related)
                    {
                        return Err(Error::SchemaValidationFailure {
                            schema: table_schema.name.to_string(),
                            attribute: relationship_name.to_string(),
                            message: format!(
                                "Relationship refers to non-existent related column '{}' at table '{}'",
                                relationship_columns.own,
                                related_table_name
                            )
                        })
                    }
                } else {
                    return Err(Error::SchemaValidationFailure {
                        schema: table_schema.name.to_string(),
                        attribute: relationship_name.to_string(),
                        message: format!(
                            "Relationship refers to non-existent table '{}'",
                            related_table_name
                        )
                    })
                }
            }
        }

        Ok(validated_tables)
    }
}