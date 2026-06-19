use crate::database::attributes::{date_time_from_millis, date_time_from_rfc3339};
use crate::database::schema::{AttributeType, TableSchema};
use crate::database::{
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use log::debug;
use rusqlite::{
    self, Row,
    types::{ToSql, ToSqlOutput, Value as DatabaseValue, ValueRef},
};
use std::fmt::Display;

pub type Connection = rusqlite::Connection;

impl ToSql for Attribute {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'static>> {
        match self {
            Attribute::Null => Ok(ToSqlOutput::Owned(DatabaseValue::Null)),
            Attribute::Text(value) => Ok(ToSqlOutput::Owned(DatabaseValue::Text(value.clone()))),
            Attribute::Integer(value) => Ok(ToSqlOutput::Owned(DatabaseValue::Integer(*value))),
            Attribute::Float(value) => Ok(ToSqlOutput::Owned(DatabaseValue::Real(*value))),
            Attribute::Boolean(value) => {
                Ok(ToSqlOutput::Owned(DatabaseValue::Integer(if *value {
                    1
                } else {
                    0
                })))
            }
            Attribute::DateTime(value) => {
                Ok(ToSqlOutput::Owned(DatabaseValue::Text(value.to_rfc3339())))
            }
        }
    }
}

fn inconsistent_schema_error<T, U>(
    schema: &TableSchema,
    attribute: &str,
    from: T,
    to: U,
) -> Result<Attribute, Error>
where
    T: Display,
    U: Display,
{
    Err(Error::InconsistentSchema {
        schema: schema.name.to_string(),
        attribute: attribute.to_string(),
        message: format!("Attribute stored as {from} cannot be converted to {to}",),
    })
}

fn materialise_attributes(schema: &TableSchema, row: &Row) -> Result<Attributes, Error> {
    let entries = row
        .as_ref()
        .columns()
        .iter()
        .enumerate()
        .map(|(index, column)| -> Result<_, Error> {
            let name = column.name();
            let value = row.get_ref_unwrap(index);

            let attribute_type = schema.attribute_type(name)?;

            let value = match value {
                ValueRef::Null => Attribute::Null,
                ValueRef::Integer(value) => match attribute_type {
                    AttributeType::Integer => Attribute::Integer(value),
                    AttributeType::DateTime => {
                        Attribute::DateTime(date_time_from_millis(value, name)?)
                    }
                    AttributeType::Boolean => Attribute::Boolean(match value {
                        0 => false,
                        1 => true,
                        _ => Err(Error::InconsistentSchema {
                            schema: schema.name.to_string(),
                            attribute: name.to_string(),
                            message: format!(
                                "Integer value '{}' cannot be converted to Boolean",
                                value
                            ),
                        })?,
                    }),
                    kind => inconsistent_schema_error(schema, name, "Integer", kind)?,
                },
                ValueRef::Real(value) => match attribute_type {
                    AttributeType::Float => Attribute::Float(value),
                    kind => inconsistent_schema_error(schema, name, "Float", kind)?,
                },
                ValueRef::Text(value) => {
                    let text = String::from_utf8_lossy(value);
                    match attribute_type {
                        AttributeType::Text => Attribute::Text(text.to_string()),
                        AttributeType::DateTime => {
                            Attribute::DateTime(date_time_from_rfc3339(text.as_ref(), name)?)
                        }
                        kind => inconsistent_schema_error(schema, name, "Text", kind)?,
                    }
                }
                ValueRef::Blob(value) => match attribute_type {
                    AttributeType::Text => Attribute::Text(b64.encode(value)),
                    kind => inconsistent_schema_error(schema, name, "Blob", kind)?,
                },
            };
            Ok((name.to_string(), value))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Attributes::from_iter(entries))
}

fn build_bindings(bindings: &[Attribute]) -> Vec<&dyn ToSql> {
    bindings.iter().map(|b| b as &dyn ToSql).collect()
}

impl ConnectionInterface for Connection {
    fn query(
        &mut self,
        query: String,
        bindings: Vec<Attribute>,
        table_schema: &TableSchema,
    ) -> Result<Vec<Attributes>, Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.prepare(&query)?;
        let rows = statement
            .query_and_then(bindings.as_slice(), |row| {
                materialise_attributes(table_schema, row)
            })?
            .collect::<Result<Vec<Attributes>, _>>()?;

        debug!("Returned {} rows", rows.len());
        Ok(rows)
    }

    fn execute(&mut self, query: String, bindings: Vec<Attribute>) -> Result<(), Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.prepare(&query)?;
        let row_count = statement.execute(bindings.as_slice())?;

        debug!("Affected {} rows", row_count);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::attributes::Attribute;
    use chrono::Utc;

    #[test]
    fn test_sqlite_conversions() {
        use rusqlite::types::{ToSqlOutput, Value as DatabaseValue};

        let null = Attribute::Null;
        assert!(
            matches!(null.to_sql(), Ok(ToSqlOutput::Owned(DatabaseValue::Null))),
            "Expected null database value"
        );

        let text = Attribute::Text("test".to_string());
        assert!(
            matches!(
                text.to_sql(),
                Ok(ToSqlOutput::Owned(DatabaseValue::Text(ref value))) if value == "test"
            ),
            "Expected text database value"
        );

        let integer = Attribute::Integer(123);
        assert!(
            matches!(
                integer.to_sql(),
                Ok(ToSqlOutput::Owned(DatabaseValue::Integer(123)))
            ),
            "Expected integer database value"
        );

        let float = Attribute::Float(1.5);
        assert!(
            matches!(
                float.to_sql(),
                Ok(ToSqlOutput::Owned(DatabaseValue::Real(value))) if value == 1.5
            ),
            "Expected real database value"
        );

        let bool_true = Attribute::Boolean(true);
        assert!(
            matches!(
                bool_true.to_sql(),
                Ok(ToSqlOutput::Owned(DatabaseValue::Integer(1)))
            ),
            "Expected integer database value for true"
        );

        let bool_false = Attribute::Boolean(false);
        assert!(
            matches!(
                bool_false.to_sql(),
                Ok(ToSqlOutput::Owned(DatabaseValue::Integer(0)))
            ),
            "Expected integer database value for false"
        );

        let dt = Utc::now();
        let datetime = Attribute::DateTime(dt);
        let expected = dt.to_rfc3339();
        assert!(
            matches!(
                datetime.to_sql(),
                Ok(ToSqlOutput::Owned(DatabaseValue::Text(ref value))) if value == &expected
            ),
            "Expected text database value for datetime"
        );
    }
}
