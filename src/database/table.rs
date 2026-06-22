use super::{
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error,
    query_builder::QueryBuilder as QueryBuilderInterface,
    query_parameters::QueryParameters,
    record::Record,
    schema::TableSchema,
};
use crate::database::attributes::{ForeignKeys, Identifier};
use crate::database::schema::IdentifierType;

pub trait Table<
    'sch,
    'req,
    Connection: ConnectionInterface + 'req,
    QueryBuilder: QueryBuilderInterface<'sch>,
>
{
    fn new(table_schema: &'sch TableSchema<'sch>, connection: &'req Connection) -> Self;

    fn schema(&self) -> &'sch TableSchema<'sch>;

    fn connection(&self) -> &'req Connection;

    fn is_attribute(&self, name: &str) -> bool {
        self.schema().attribute(name).is_some()
    }

    fn is_foreign_key(&self, name: &str) -> bool {
        self.schema().foreign_key(name).is_some()
    }

    fn query(&self, parameters: &QueryParameters) -> Result<Vec<Record<'sch>>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).query(parameters)?;
        self.run_fetch(query, bindings)
    }

    fn first(&self, parameters: &QueryParameters) -> Result<Option<Record<'sch>>, Error> {
        self.query(parameters).map(|rows| rows.into_iter().next())
    }

    fn find(&self, id: Identifier, parameters: &QueryParameters) -> Result<Record<'sch>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).find(id, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn insert(
        &self,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<Record<'sch>, Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).insert(attributes, parameters)?;

        self.run_fetch_single(query, bindings)
    }

    fn update(
        &self,
        id: Identifier,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<Record<'sch>, Error> {
        self.require_attributes(&attributes)?;
        let (query, bindings) =
            QueryBuilder::new(self.schema()).update(id, attributes, parameters)?;
        self.run_fetch_single(query, bindings)
    }

    fn update_batch(
        &self,
        attributes: Attributes,
        parameters: &QueryParameters,
    ) -> Result<Vec<Record<'sch>>, Error> {
        self.require_attributes(&attributes)?;
        let (query, bindings) =
            QueryBuilder::new(self.schema()).update_batch(attributes, parameters)?;
        self.run_fetch(query, bindings)
    }

    fn insert_batch(
        &self,
        rows: Vec<Attributes>,
        parameters: &QueryParameters,
    ) -> Result<Vec<Record<'sch>>, Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let (query, bindings) = QueryBuilder::new(self.schema()).insert_batch(rows, parameters)?;
        self.run_fetch(query, bindings)
    }

    fn require_attributes(&self, attributes: &Attributes) -> Result<(), Error> {
        if attributes.is_empty() {
            return Err(Error::InvalidOperation {
                schema: self.schema().name.to_string(),
                operation: "UPDATE".to_string(),
                message: "no attributes to update".to_string(),
            });
        }

        Ok(())
    }

    fn delete(&self, id: Identifier) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete(id);
        self.connection().execute(query, bindings)
    }

    fn delete_batch(&self, parameters: &QueryParameters) -> Result<(), Error> {
        let (query, bindings) = QueryBuilder::new(self.schema()).delete_batch(parameters)?;
        self.connection().execute(query, bindings)
    }

    fn run_fetch(
        &self,
        query: String,
        bindings: Vec<Attribute>,
    ) -> Result<Vec<Record<'sch>>, Error> {
        self.connection()
            .query(query, bindings, self.schema())?
            .into_iter()
            .map(|mut raw_attributes| {
                let schema = self.schema();
                let id = raw_attributes
                    .shift_remove(schema.primary_key.name)
                    .ok_or_else(|| Error::SchemaValidationFailure {
                        schema: schema.name.to_string(),
                        attribute: schema.primary_key.name.to_string(),
                        message: "Primary key was not loaded".to_string(),
                    })?;

                let id = match (schema.primary_key.kind, id) {
                    (IdentifierType::Integer, Attribute::Integer(value)) => {
                        Identifier::Integer(value)
                    }
                    (IdentifierType::Text, Attribute::Text(value)) => Identifier::Text(value),
                    (_, id) => Err(Error::SchemaValidationFailure {
                        schema: schema.name.to_string(),
                        attribute: schema.primary_key.name.to_string(),
                        message: format!(
                            "Expected primary key '{:?}' to be of type '{}'",
                            id, schema.primary_key.kind
                        ),
                    })?,
                };

                let mut attributes = Attributes::new();
                let mut foreign_keys = ForeignKeys::new();

                for (name, attribute) in raw_attributes {
                    if self.is_attribute(&name) {
                        attributes.insert(name, attribute);
                    } else if self.is_foreign_key(&name)
                        && let Some((name, _)) =
                            schema.foreign_keys.iter().find(|(fk, _)| fk == &name)
                    {
                        foreign_keys.insert(name, attribute);
                    } else {
                        Err(Error::SchemaValidationFailure {
                            schema: schema.name.to_string(),
                            attribute: name,
                            message: "Database returned an unknown attribute".to_string(),
                        })?;
                    }
                }

                Ok(Record::new(self.schema(), id, attributes, foreign_keys))
            })
            .collect::<Result<Vec<_>, _>>()
    }

    fn run_fetch_single(
        &self,
        query: String,
        bindings: Vec<Attribute>,
    ) -> Result<Record<'sch>, Error> {
        self.run_fetch(query, bindings)?
            .into_iter()
            .next()
            .ok_or(Error::RecordNotFound)
    }
}
