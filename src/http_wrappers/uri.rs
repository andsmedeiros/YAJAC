use std::{
    convert::{AsMut, AsRef},
    fmt::{Display,Formatter},
    ops::{Deref, DerefMut},
    str::FromStr
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde::de::{Error, Visitor};

#[derive(Debug, Clone)]
pub struct Uri(http::Uri);

impl Display for Uri {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.to_string())
    }
}

impl Deref for Uri {
    type Target = http::Uri;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl DerefMut for Uri {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 }
}

impl AsRef<http::Uri> for Uri {
    fn as_ref(&self) -> &http::Uri { &self.0 }
}

impl AsMut<http::Uri> for Uri {
    fn as_mut(&mut self) -> &mut http::Uri { &mut self.0 }
}

impl From<http::Uri> for Uri {
    fn from(value: http::Uri) -> Self {
        Uri(value)
    }
}

impl FromStr for Uri {
    type Err = <http::Uri as FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<http::Uri>().map(|u| u.into())
    }
}

impl Serialize for Uri {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.to_string().as_str())
    }
}

struct UriVisitor;
impl<'de> Visitor<'de> for UriVisitor {
    type Value = http::Uri;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("a string containing a valid URI")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        http::Uri::from_str(v).map_err(Error::custom)
    }
}

impl<'de> Deserialize<'de> for Uri {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Uri(deserializer.deserialize_string(UriVisitor)?))
    }
}
