use crate::database::attributes::{date_time_from_millis, date_time_from_rfc3339};
use crate::database::schema::{AttributeType, Schema};
use crate::database::{
    attributes::{Attribute, Attributes},
    connection::Connection as ConnectionInterface,
    error::Error,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use log::{debug, error};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{
    self, Row,
    types::{ToSql, ToSqlOutput, Value as DatabaseValue, ValueRef},
};
use std::cell::Cell;
use std::fmt::Display;

/// A pooled SQLite connection together with the current depth of transaction nesting open on it.
/// Owns the pooled handle and returns it to the pool when dropped.
pub struct Connection {
    handle: PooledConnection<SqliteConnectionManager>,
    depth: Cell<usize>,
}

impl Connection {
    pub(super) fn new(handle: PooledConnection<SqliteConnectionManager>) -> Self {
        Self {
            handle,
            depth: Cell::new(0),
        }
    }

    /// Runs a batch of `;`-separated statements with no bindings. The setup path for DDL and seeding.
    pub fn execute_batch(&self, sql: &str) -> Result<(), Error> {
        self.handle.execute_batch(sql)?;
        Ok(())
    }

    /// Opens a transaction level: `BEGIN` at the outermost, a savepoint below it.
    fn begin_transaction(&self) -> Result<(), Error> {
        let sql = match self.depth.get() {
            0 => "BEGIN".to_string(),
            level => format!("SAVEPOINT sp{level}"),
        };
        self.handle.execute_batch(&sql)?;
        self.depth.update(|level| level + 1);
        Ok(())
    }

    /// Closes the current transaction level successfully: `COMMIT` at the outermost, `RELEASE` below.
    fn commit_transaction(&self) -> Result<(), Error> {
        let sql = match self.depth.get() - 1 {
            0 => "COMMIT".to_string(),
            level => format!("RELEASE sp{level}"),
        };
        self.handle.execute_batch(&sql)?;
        self.depth.update(|level| level - 1);
        Ok(())
    }

    /// Discards the current transaction level: `ROLLBACK` at the outermost, `ROLLBACK TO`/`RELEASE`
    /// below.
    fn rollback_transaction(&self) -> Result<(), Error> {
        let sql = match self.depth.get() - 1 {
            0 => "ROLLBACK".to_string(),
            level => format!("ROLLBACK TO sp{level}; RELEASE sp{level}"),
        };
        self.handle.execute_batch(&sql)?;
        self.depth.update(|level| level - 1);
        Ok(())
    }
}

impl Drop for Connection {
    /// Rolls back a transaction still open at drop — the panic path, where neither arm of
    /// `transaction` ran — so the connection rejoins the pool clean instead of poisoning the next
    /// checkout.
    fn drop(&mut self) {
        if self.depth.get() > 0 {
            if let Err(error) = self.handle.execute_batch("ROLLBACK") {
                error!(
                    "Failed to roll back a dangling transaction before returning the \
                     connection to the pool: {error}"
                );
            }
        }
    }
}

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
    schema: &Schema,
    attribute: &str,
    from: T,
    to: U,
) -> Result<Attribute, Error>
where
    T: Display,
    U: Display,
{
    Err(Error::InconsistentSchema {
        schema: schema.name().to_string(),
        attribute: attribute.to_string(),
        message: format!("Attribute stored as {from} cannot be converted to {to}",),
    })
}

fn materialise_attributes<'sch>(
    schema: &'sch Schema<'sch>,
    row: &Row,
) -> Result<Attributes<'sch>, Error> {
    let entries = row
        .as_ref()
        .columns()
        .iter()
        .enumerate()
        .map(|(index, column)| -> Result<_, Error> {
            let name = column.name();
            let value = row.get_ref_unwrap(index);

            let column = schema
                .column(name)
                .ok_or_else(|| Error::InconsistentSchema {
                    schema: schema.name().to_string(),
                    attribute: name.to_string(),
                    message: "Database returned an unknown column".to_string(),
                })?;
            let attribute_type = column.kind;

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
                            schema: schema.name().to_string(),
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
            Ok((column.name, value))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Attributes::from_iter(entries))
}

fn build_bindings(bindings: &[Attribute]) -> Vec<&dyn ToSql> {
    bindings.iter().map(|b| b as &dyn ToSql).collect()
}

impl ConnectionInterface for Connection {
    fn query<'sch>(
        &self,
        query: String,
        bindings: Vec<Attribute>,
        schema: &'sch Schema<'sch>,
    ) -> Result<Vec<Attributes<'sch>>, Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.handle.prepare(&query)?;
        let rows = statement
            .query_and_then(bindings.as_slice(), |row| {
                materialise_attributes(schema, row)
            })?
            .collect::<Result<Vec<Attributes<'sch>>, _>>()?;

        debug!("Returned {} rows", rows.len());
        Ok(rows)
    }

    fn execute(&self, query: String, bindings: Vec<Attribute>) -> Result<(), Error> {
        debug!("{}, {:?}", query, bindings);

        let bindings = build_bindings(&bindings);
        let mut statement = self.handle.prepare(&query)?;
        let row_count = statement.execute(bindings.as_slice())?;

        debug!("Affected {} rows", row_count);
        Ok(())
    }

    /// Runs `operation` inside a transaction level: commits it on `Ok`, rolls it back on `Err`. The
    /// level nests — the outermost is a real transaction, inner ones savepoints — so composing store
    /// calls stays atomic. A panic inside `operation` leaves the level open; `Drop` clears it.
    fn transaction<R>(&self, operation: impl FnOnce() -> Result<R, Error>) -> Result<R, Error> {
        self.begin_transaction()?;

        match operation() {
            Ok(value) => {
                self.commit_transaction()?;
                Ok(value)
            }
            Err(error) => {
                self.rollback_transaction()?;
                Err(error)
            }
        }
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
