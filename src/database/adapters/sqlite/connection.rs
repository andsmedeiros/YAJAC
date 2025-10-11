use log::debug;
use base64::{engine::general_purpose::STANDARD as b64, Engine as _};
use rusqlite::{
    self,
    Row,
    Statement,
    types::{ToSql, ToSqlOutput, Value as DatabaseValue, ValueRef},
};
use crate::database::{
    attributes,
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error
};
use crate::database::attributes::{date_time_from_millis, date_time_from_rfc3339};
use crate::database::schema::{AttributeType, TableSchema};
use std::fmt::Display;

pub type Connection = rusqlite::Connection;

impl ToSql for Attribute {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'static>> {
        match self {
            Attribute::Null =>
                Ok(ToSqlOutput::Owned(DatabaseValue::Null)),
            Attribute::Text(value) =>
                Ok(ToSqlOutput::Owned(DatabaseValue::Text(value.clone()))),
            Attribute::Integer(value) =>
                Ok(ToSqlOutput::Owned(DatabaseValue::Integer(value.clone()))),
            Attribute::Float(value) =>
                Ok(ToSqlOutput::Owned(DatabaseValue::Real(value.clone()))),
            Attribute::Boolean(value) =>
                Ok(ToSqlOutput::Owned(DatabaseValue::Integer(if *value { 1 } else { 0 }))),
            Attribute::DateTime(value) =>
                Ok(ToSqlOutput::Owned(DatabaseValue::Text(value.to_rfc3339())))
        }
    }
}


fn inconsistent_schema_error<T, U>(schema: &TableSchema, attribute: &str, from: T, to: U)
                                   -> Result<Attribute, Error>
where
    T: Display,
    U: Display,
{
    Err(Error::InconsistentSchema {
        schema: schema.name.to_string(),
        attribute: attribute.to_string(),
        message: format!("Attribute stored as {from} cannot be converted to {to}", )
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

            let attribute_type = schema.column(name)
                .expect("Column not found in schema");

            let value = match value {
                ValueRef::Null =>
                    Attribute::Null,
                ValueRef::Integer(value) => match attribute_type {
                    AttributeType::Integer =>
                        Attribute::Integer(value),
                    AttributeType::DateTime =>
                        Attribute::DateTime(date_time_from_millis(value, name)?),
                    kind =>
                        inconsistent_schema_error(schema, name, "Integer", kind)?
                },
                ValueRef::Real(value) => match attribute_type {
                    AttributeType::Float =>
                        Attribute::Float(value),
                    kind =>
                        inconsistent_schema_error(schema, name, "Float", kind)?
                },
                ValueRef::Text(value) => {
                    let text = String::from_utf8_lossy(value);
                    match attribute_type {
                        AttributeType::Text =>
                            Attribute::Text(text.to_string()),
                        AttributeType::DateTime =>
                            Attribute::DateTime(date_time_from_rfc3339(text.as_ref(), name)?),
                        kind =>
                            inconsistent_schema_error(schema, name, "Text", kind)?
                    }
                },
                ValueRef::Blob(value) => match attribute_type {
                    AttributeType::Text =>
                        Attribute::Text(b64.encode(value)),
                    kind =>
                        inconsistent_schema_error(schema, name, "Blob", kind)?
                }
            };
            Ok((name.to_string(), value))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Attributes::from_iter(entries.into_iter()))
}

fn build_bindings(bindings: &Vec<Attribute>) -> Vec<&dyn ToSql> {
    bindings
        .iter()
        .map(|b| b as &dyn ToSql)
        .collect()
}

impl ConnectionInterface for Connection {
    fn query(&mut self, query: String, bindings: Vec<Attribute>, table_schema: &TableSchema) -> Result<Vec<Attributes>, Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.prepare(&query)?;
        let rows = statement
            .query_and_then(bindings.as_slice(), |row|
                materialise_attributes(table_schema, row)
            )?
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
    use chrono::Utc;
    use crate::database::attributes::Attribute;

    #[test]
    fn test_sqlite_conversions() {
        use rusqlite::types::{ToSqlOutput, Value as DatabaseValue};

        todo!("Write adequate tests here!");

        let null = Attribute::Null;
        match null.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Null) => {},
            _ => panic!("Expected null database value"),
        }

        let text = Attribute::Text("test".to_string());
        match text.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Text(value)) => assert_eq!(value, "test"),
            _ => panic!("Expected text database value"),
        }

        let integer = Attribute::Integer(123);
        match integer.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Integer(value)) => assert_eq!(value, 123),
            _ => panic!("Expected integer database value"),
        }

        let float = Attribute::Float(1.5);
        match float.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Real(value)) => assert_eq!(value, 1.5),
            _ => panic!("Expected real database value"),
        }

        let bool_true = Attribute::Boolean(true);
        match bool_true.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Integer(value)) => assert_eq!(value, 1),
            _ => panic!("Expected integer database value for true"),
        }

        let bool_false = Attribute::Boolean(false);
        match bool_false.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Integer(value)) => assert_eq!(value, 0),
            _ => panic!("Expected integer database value for false"),
        }

        let dt = Utc::now();
        let datetime = Attribute::DateTime(dt);
        match datetime.to_sql().unwrap() {
            ToSqlOutput::Owned(DatabaseValue::Text(value)) => assert_eq!(value, dt.to_rfc3339()),
            _ => panic!("Expected text database value for datetime"),
        }
    }
}