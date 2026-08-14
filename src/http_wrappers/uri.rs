use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    convert::{AsMut, AsRef},
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri(http::Uri);

impl Display for Uri {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for Uri {
    type Target = http::Uri;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Uri {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<http::Uri> for Uri {
    fn as_ref(&self) -> &http::Uri {
        &self.0
    }
}

impl AsMut<http::Uri> for Uri {
    fn as_mut(&mut self) -> &mut http::Uri {
        &mut self.0
    }
}

impl From<http::Uri> for Uri {
    fn from(value: http::Uri) -> Self {
        Uri(value)
    }
}

impl From<Uri> for http::Uri {
    fn from(value: Uri) -> Self {
        value.0
    }
}

impl PartialEq<http::Uri> for Uri {
    fn eq(&self, other: &http::Uri) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Uri> for http::Uri {
    fn eq(&self, other: &Uri) -> bool {
        *self == other.0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equals_its_inner_uri_in_both_directions() -> Result<(), Box<dyn std::error::Error>> {
        let inner = "https://example.com/articles/1".parse::<http::Uri>()?;
        let wrapper = Uri::from(inner.clone());

        assert_eq!(wrapper, inner);
        assert_eq!(inner, wrapper);
        Ok(())
    }

    #[test]
    fn differs_from_another_inner_uri_in_both_directions() -> Result<(), Box<dyn std::error::Error>>
    {
        let wrapper = Uri::from("https://example.com/articles/1".parse::<http::Uri>()?);
        let other = "https://example.com/articles/2".parse::<http::Uri>()?;

        assert_ne!(wrapper, other);
        assert_ne!(other, wrapper);
        Ok(())
    }
}
