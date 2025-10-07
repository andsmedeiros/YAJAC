use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Display,
    str::FromStr
};
use super::error::Error;

#[derive(Debug, Clone)]
pub struct RouteParameters(HashMap<String, String>);

impl RouteParameters {
    pub fn new() -> RouteParameters {
        RouteParameters(HashMap::new())
    }

    pub fn insert<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.0.insert(key.into(), value.into());
    }

    pub fn has<K>(&self, key: K) -> bool
    where
        K: Borrow<str>,
    {
        self.0.contains_key(key.borrow())
    }

    pub fn get<K>(&self, key: K) -> Option<&String>
    where
        K: Borrow<str>,
    {
        self.0.get(key.borrow())
    }

    pub fn require<K>(&self, key: K) -> Result<&String, Error>
    where
        K: Borrow<str> + Display,
    {
        self.get(key.borrow())
            .ok_or_else(|| Error::RequiredParameterMissing {
                parameter: key.borrow().into(),
            })
    }

    pub fn get_as<T, K>(&self, key: K) -> Result<Option<T>, Error>
    where
        T: FromStr,
        K: Borrow<str> + Display,
    {
        self.get(key.borrow())
            .map(|value|
                value
                    .parse::<T>()
                    .map_err(|_| Error::ParseParameterFailure {
                        parameter: key.borrow().into(),
                        message: "Provided parameter contains an unexpected value".to_string(),
                    })
            )
            .transpose()
    }

    pub fn require_as<T, K>(&self, key: K) -> Result<T, Error>
    where
        T: FromStr,
        K: Borrow<str> + Display,
    {
        self.get_as(key.borrow())?
            .ok_or_else(|| Error::RequiredParameterMissing {
                parameter: key.borrow().into(),
            })
    }
}