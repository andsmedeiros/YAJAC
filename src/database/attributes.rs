use indexmap::IndexMap;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use super::{
    error::Error,
    schema::{AttributeType, DateTime, TableSchema},
};
use std::fmt::{Display};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Attribute {
    Null,
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    DateTime(DateTime),
}

impl PartialEq for Attribute {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Attribute::Null, Attribute::Null) => true,
            (Attribute::Text(a), Attribute::Text(b)) => a == b,
            (Attribute::Integer(a), Attribute::Integer(b)) => a == b,
            (Attribute::Float(a), Attribute::Float(b)) =>
                format!("{:.12}", a) == format!("{:.12}", b),
            (Attribute::Boolean(a), Attribute::Boolean(b)) => a == b,
            (Attribute::DateTime(a), Attribute::DateTime(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Attribute {}

impl Hash for Attribute {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Attribute::Null => (),
            Attribute::Text(value) => value.hash(state),
            Attribute::Integer(value) => value.hash(state),
            Attribute::Float(value) => format!("{:.12}", value).hash(state),
            Attribute::Boolean(value) => value.hash(state),
            Attribute::DateTime(value) => value.hash(state),
        }
    }
}

impl Attribute {
    pub fn as_string(&self) -> Result<&String, Error> {
        match self {
            Attribute::Text(s) => Ok(s),
            _ => Err(Error::InvalidAttributeConversion { kind: "&String".to_string() }),
        }
    }

    pub fn to_string(self) -> Result<String, Error> {
        match self {
            Attribute::Text(s) => Ok(s),
            _ => Err(Error::InvalidAttributeConversion { kind: "String".to_string() }),
        }
    }

    pub fn as_i64(&self) -> Result<&i64, Error> {
        match self {
            Attribute::Integer(i) => Ok(i),
            _ => Err(Error::InvalidAttributeConversion { kind: "&i64".to_string() }),
        }
    }

    pub fn to_i64(self) -> Result<i64, Error> {
        match self {
            Attribute::Integer(i) => Ok(i),
            _ => Err(Error::InvalidAttributeConversion { kind: "i64".to_string() }),
        }
    }

    pub fn as_f64(&self) -> Result<&f64, Error> {
        match self {
            Attribute::Float(f) => Ok(f),
            _ => Err(Error::InvalidAttributeConversion { kind: "&f64".to_string() }),
        }
    }

    pub fn to_f64(self) -> Result<f64, Error> {
        match self {
            Attribute::Float(f) => Ok(f),
            _ => Err(Error::InvalidAttributeConversion { kind: "f64".to_string() }),
        }
    }

    pub fn as_bool(&self) -> Result<&bool, Error> {
        match self {
            Attribute::Boolean(b) => Ok(b),
            _ => Err(Error::InvalidAttributeConversion { kind: "&bool".to_string() }),
        }
    }

    pub fn to_bool(self) -> Result<bool, Error> {
        match self {
            Attribute::Boolean(b) => Ok(b),
            _ => Err(Error::InvalidAttributeConversion { kind: "bool".to_string() }),
        }
    }

    pub fn as_datetime(&self) -> Result<&DateTime, Error> {
        match self {
            Attribute::DateTime(d) => Ok(d),
            _ => Err(Error::InvalidAttributeConversion { kind: "&DateTime".to_string() }),
        }
    }

    pub fn to_datetime(self) -> Result<DateTime, Error> {
        match self {
            Attribute::DateTime(d) => Ok(d),
            _ => Err(Error::InvalidAttributeConversion { kind: "DateTime".to_string() }),
        }
    }
}

impl Display for Attribute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Attribute::Null => f.write_str("null"),
            Attribute::Text(text) => f.write_str(text),
            Attribute::Integer(int) => f.write_str(int.to_string().as_str()),
            Attribute::Float(float) => f.write_str(float.to_string().as_str()),
            Attribute::Boolean(boolean) => f.write_str(boolean.to_string().as_str()),
            Attribute::DateTime(datetime) => f.write_str(datetime.to_string().as_str()),
        }
    }
}

impl From<Attribute> for Value {
    fn from(value: Attribute) -> Self {
        match value {
            Attribute::Null => Value::Null,
            Attribute::Text(value) => Value::String(value),
            Attribute::Integer(value) => Value::Number(value.into()),
            Attribute::Float(value) => serde_json::Number::from_f64(value)
                .and_then(|value| Some(Value::Number(value)))
                .unwrap_or(Value::Null),
            Attribute::Boolean(value) => Value::Bool(value),
            Attribute::DateTime(value) => Value::String(value.to_rfc3339()),
        }
    }
}

pub type Attributes = IndexMap<String, Attribute>;

pub fn date_time_from_millis(millis: i64, attribute: &str) -> Result<DateTime, Error> {
    if let Some(date_time) = DateTime::from_timestamp_millis(millis) {
        Ok(date_time)
    } else {
        Err(Error::InvalidAttribute {
            attribute: attribute.to_string(),
            kind: "DateTime".to_string(),
            message: format!("Value {millis} is out of bounds")
        })
    }
}

pub fn date_time_from_rfc3339(date_time: &str, attribute: &str) -> Result<DateTime, Error> {
    if let Ok(date_time) = chrono::DateTime::parse_from_rfc3339(date_time) {
        Ok(date_time.to_utc())
    } else {
        Err(Error::InvalidAttribute {
            attribute: attribute.to_string(),
            kind: "DateTime".to_string(),
            message: format!("DateTime string '{date_time}' is invalid")
        })
    }
}

fn attribute_from_value(value: Value, attribute: &str, attribute_type: &AttributeType) -> Result<Attribute, Error> {
    match value {
        Value::Null => Ok(Attribute::Null),
        Value::String(value) => match attribute_type {
            AttributeType::Text => Ok(Attribute::Text(value)),
            AttributeType::DateTime =>
                Ok(Attribute::DateTime(date_time_from_rfc3339(value.as_str(), attribute)?)),
            AttributeType::Boolean =>
                match value.to_lowercase().as_str() {
                    "true" => Ok(Attribute::Boolean(true)),
                    "false" => Ok(Attribute::Boolean(false)),
                    _ => Err(Error::InvalidAttribute {
                        attribute: attribute.to_string(),
                        kind: "bool".to_string(),
                        message: "Provided value does not represent a valid boolean".to_string()
                    }),
                }
            _ => Err(Error::InvalidAttribute {
                attribute: attribute.to_string(),
                kind: "DateTime".to_string(),
                message: "Provided value does not represent a valid DateTime".to_string()
            })
        },
        Value::Number(number) => match attribute_type {
            AttributeType::Integer => match number.as_i64() {
                Some(value) => Ok(Attribute::Integer(value)),
                None => Err(Error::InvalidAttribute {
                    attribute: attribute.to_string(),
                    kind: "i64".to_string(),
                    message: "Provided value does not represent an integer".to_string()
                })
            },
            AttributeType::Float => match number.as_f64() {
                Some(value) => Ok(Attribute::Float(value)),
                None => Err(Error::InvalidAttribute {
                    attribute: attribute.to_string(),
                    kind: "f64".to_string(),
                    message: "Provided value does not represent an float".to_string()
                })
            },
            AttributeType::DateTime => match number.as_i64() {
                Some(value) =>
                    Ok(Attribute::DateTime(date_time_from_millis(value, attribute)?)),
                None => Err(Error::InvalidAttribute {
                    attribute: attribute.to_string(),
                    kind: "DateTime".to_string(),
                    message: "Provided value does not represent a valid DateTime".to_string()
                })
            },
            AttributeType::Boolean => match number.as_i64() {
                Some(value) => match value {
                    0 => Ok(Attribute::Boolean(false)),
                    1 => Ok(Attribute::Boolean(true)),
                    _ => Err(Error::InvalidAttribute {
                        attribute: attribute.to_string(),
                        kind: "boolean".to_string(),
                        message: "Provided value does not represent a valid boolean".to_string()
                    }),
                },
                None => Err(Error::InvalidAttribute {
                    attribute: attribute.to_string(),
                    kind: "boolean".to_string(),
                    message: "Provided value does not represent a valid boolean".to_string()
                })
            },
            _ => Err(Error::InvalidAttribute {
                attribute: attribute.to_string(),
                kind: attribute_type.to_string(),
                message: format!("Provided value does not represent a valid {}", attribute_type)
            })
        },
        Value::Bool(value) => match attribute_type {
            AttributeType::Boolean => Ok(Attribute::Boolean(value)),
            _ => Err(Error::InvalidAttribute {
                attribute: attribute.to_string(),
                kind: attribute_type.to_string(),
                message: format!("Provided value does not represent a valid {}", attribute_type)
            })
        },
        _ => Err(Error::InvalidAttribute {
            attribute: attribute.to_string(),
            kind: attribute_type.to_string(),
            message: format!("Provided value does not represent a valid {}", attribute_type)
        })
    }
}

pub fn from_value(schema: &TableSchema, value: Value) -> Result<Attributes, Error> {
    let schema_name = schema.name;
    let entries = match value {
        Value::Object(object) => object
            .into_iter()
            .map(|(attribute, value)|
                match schema.column(attribute.as_str()) {
                    Some(attribute_type) =>
                        Ok((attribute, attribute_from_value(value, schema_name, attribute_type)?)),
                    None =>
                        Err(Error::SchemaValidationFailure {
                            schema: schema_name.to_string(),
                            attribute,
                            message: "Unknown attribute".to_string()
                        })
                }
            ),
        _ => Err(Error::InvalidAttributeSet)?
    }.collect::<Result<Vec<_>, Error>>()?;

    Ok(Attributes::from_iter(entries.into_iter()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::database::schema::{AttributeType, TableSchema};
    use chrono::Utc;

    #[test]
    fn test_attribute_conversions() {
        let text = Attribute::Text("hello".to_string());
        let integer = Attribute::Integer(42);
        let float = Attribute::Float(3.14);
        let boolean = Attribute::Boolean(true);
        let datetime = Attribute::DateTime(Utc::now());
        let null = Attribute::Null;

        // Test successful as_* conversions
        assert_eq!(text.as_string().unwrap(), "hello");
        assert_eq!(*integer.as_i64().unwrap(), 42);
        assert_eq!(*float.as_f64().unwrap(), 3.14);
        assert_eq!(*boolean.as_bool().unwrap(), true);
        assert!(datetime.as_datetime().is_ok());

        // Test successful to_* conversions
        assert_eq!(text.clone().to_string().unwrap(), "hello");
        assert_eq!(integer.clone().to_i64().unwrap(), 42);
        assert_eq!(float.clone().to_f64().unwrap(), 3.14);
        assert_eq!(boolean.clone().to_bool().unwrap(), true);
        assert!(datetime.clone().to_datetime().is_ok());

        // Test failed conversions
        assert!(integer.as_string().is_err());
        assert!(text.as_i64().is_err());
        assert!(text.as_f64().is_err());
        assert!(text.as_bool().is_err());
        assert!(text.as_datetime().is_err());
        assert!(null.clone().to_string().is_err());
        assert!(null.as_i64().is_err());
    }

    #[test]
    fn test_from_value_success_and_failures() {
        let schema = TableSchema {
            name: "test",
            columns: &[
                ("name", AttributeType::Text),
                ("age", AttributeType::Integer),
                ("score", AttributeType::Float),
                ("active", AttributeType::Boolean),
            ],
            relationships: &[],
            text_index: false,
        };

        // Test successful conversion
        let json = json!({
            "name": "John",
            "age": 30,
            "score": 85.5,
            "active": true
        });
        let attributes = from_value(&schema, json).unwrap();
        assert_eq!(attributes.len(), 4);
        assert_eq!(attributes["name"].as_string().unwrap(), "John");
        assert_eq!(*attributes["age"].as_i64().unwrap(), 30);
        assert_eq!(*attributes["score"].as_f64().unwrap(), 85.5);
        assert_eq!(*attributes["active"].as_bool().unwrap(), true);

        // Test failures
        let json_unknown = json!({"unknown_field": "value"});
        assert!(from_value(&schema, json_unknown).is_err());

        let json_wrong_type = json!({"age": "not_a_number"});
        assert!(from_value(&schema, json_wrong_type).is_err());

        let json_not_object = json!("just a string");
        assert!(from_value(&schema, json_not_object).is_err());
    }

    #[test]
    fn test_datetime_conversions() {
        let schema = TableSchema {
            name: "test",
            columns: &[
                ("timestamp", AttributeType::DateTime),
            ],
            relationships: &[],
            text_index: false,
        };

        // Test milliseconds timestamp
        let json_millis = json!({"timestamp": 1609459200000i64});
        assert!(from_value(&schema, json_millis).is_ok());

        // Test ISO string
        let json_iso = json!({"timestamp": "2021-01-01T00:00:00Z"});
        assert!(from_value(&schema, json_iso).is_ok());

        // Test invalid datetime
        let json_invalid = json!({"timestamp": "invalid-date"});
        assert!(from_value(&schema, json_invalid).is_err());
    }

    #[test]
    fn test_boolean_conversions() {
        let schema = TableSchema {
            name: "test",
            columns: &[
                ("flag", AttributeType::Boolean),
            ],
            relationships: &[],
            text_index: false,
        };

        // Test string conversions
        let json_true = json!({"flag": "true"});
        let attributes = from_value(&schema, json_true).unwrap();
        assert_eq!(*attributes["flag"].as_bool().unwrap(), true);

        let json_false = json!({"flag": "FALSE"});
        let attributes = from_value(&schema, json_false).unwrap();
        assert_eq!(*attributes["flag"].as_bool().unwrap(), false);

        let json_invalid_str = json!({"flag": "maybe"});
        assert!(from_value(&schema, json_invalid_str).is_err());

        // Test number conversions
        let json_one = json!({"flag": 1});
        let attributes = from_value(&schema, json_one).unwrap();
        assert_eq!(*attributes["flag"].as_bool().unwrap(), true);

        let json_zero = json!({"flag": 0});
        let attributes = from_value(&schema, json_zero).unwrap();
        assert_eq!(*attributes["flag"].as_bool().unwrap(), false);

        let json_invalid_num = json!({"flag": 2});
        assert!(from_value(&schema, json_invalid_num).is_err());
    }
}