use super::error::Error;
use std::{borrow::Borrow, borrow::Cow, collections::HashMap, fmt::Display, str::FromStr};

/// What a matched route captured: its named single-segment parameters (`:name`), and — kept
/// distinct — the optional trailing glob (`*`). A name borrows the route template (`'sch`) when the
/// template segment is borrowed and owns it when it was computed; a value borrows the request path
/// (`'req`) when it needed no decoding and owns a decoded string otherwise. The glob is a raw,
/// multi-segment tail handed over verbatim for the handler to interpret, never decoded.
#[derive(Debug, Clone, Default)]
pub struct RouteParameters<'sch, 'req> {
    named: HashMap<Cow<'sch, str>, Cow<'req, str>>,
    glob: Option<Cow<'req, str>>,
}

impl<'sch, 'req> RouteParameters<'sch, 'req> {
    pub fn new() -> RouteParameters<'sch, 'req> {
        RouteParameters::default()
    }

    pub fn insert<K, V>(&mut self, key: K, value: V)
    where
        K: Into<Cow<'sch, str>>,
        V: Into<Cow<'req, str>>,
    {
        self.named.insert(key.into(), value.into());
    }

    /// Records the trailing glob's captured tail — the remaining segments joined by `/`, verbatim.
    pub fn set_glob<V>(&mut self, tail: V)
    where
        V: Into<Cow<'req, str>>,
    {
        self.glob = Some(tail.into());
    }

    /// The trailing glob's captured tail, if the route matched one — a raw, unencoded path.
    pub fn get_glob(&self) -> Option<&Cow<'req, str>> {
        self.glob.as_ref()
    }

    /// The trailing glob's captured tail, erroring if the route has no glob (a handler misuse).
    pub fn require_glob(&self) -> Result<&Cow<'req, str>, Error> {
        self.get_glob()
            .ok_or_else(|| Error::RequiredRouteParameterMissing {
                parameter: "glob".to_string(),
            })
    }

    pub fn has<K>(&self, key: K) -> bool
    where
        K: Borrow<str>,
    {
        self.named.contains_key(key.borrow())
    }

    pub fn get<K>(&self, key: K) -> Option<&Cow<'req, str>>
    where
        K: Borrow<str>,
    {
        self.named.get(key.borrow())
    }

    pub fn require<K>(&self, key: K) -> Result<&Cow<'req, str>, Error>
    where
        K: Borrow<str> + Display,
    {
        self.get(key.borrow())
            .ok_or_else(|| Error::RequiredRouteParameterMissing {
                parameter: key.borrow().to_string(),
            })
    }

    pub fn get_as<T, K>(&self, key: K) -> Result<Option<T>, Error>
    where
        T: FromStr,
        K: Borrow<str> + Display,
    {
        self.get(key.borrow())
            .map(|value| {
                value
                    .parse::<T>()
                    .map_err(|_| Error::FailedToParseRouteParameter {
                        parameter: key.borrow().to_string(),
                        message: "The provided value has an unexpected format".to_string(),
                    })
            })
            .transpose()
    }

    pub fn require_as<T, K>(&self, key: K) -> Result<T, Error>
    where
        T: FromStr,
        K: Borrow<str> + Display,
    {
        self.get_as(key.borrow())?
            .ok_or_else(|| Error::RequiredRouteParameterMissing {
                parameter: key.borrow().to_string(),
            })
    }
}
