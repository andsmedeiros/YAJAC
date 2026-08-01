use crate::routing::error::{Error, LinkGenerationError};
use crate::{http_wrappers::Uri, json_api::identifier::Identifier};
use std::borrow::Cow;
use std::collections::HashMap;

const GENERATED_INVALID_MSG: &str =
    "Generated an invalid URI. This is a bug and should not happen!";

pub trait UriGenerator {
    fn base_url(&self) -> String {
        "".to_string()
    }

    fn uri_for_collection(&self, kind: &str) -> Uri {
        let base = self.base_url();
        format!("{}/{}", base, kind)
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }

    fn uri_for_resource(&self, identifier: &Identifier) -> Uri {
        let base = self.base_url();

        if let Identifier::Existing { kind, id } = identifier {
            format!("{base}/{kind}/{id}")
                .parse::<Uri>()
                .expect(GENERATED_INVALID_MSG)
        } else {
            panic!("Attempted to generate URI for unpersisted resource");
        }
    }

    fn uri_for_relationship(&self, identifier: &Identifier, relationship: &str) -> Uri {
        let resource = self.uri_for_resource(identifier);
        format!("{resource}/relationships/{relationship}")
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }

    fn uri_for_related(&self, identifier: &Identifier, relationship: &str) -> Uri {
        let resource = self.uri_for_resource(identifier);
        format!("{resource}/{relationship}")
            .parse::<Uri>()
            .expect(GENERATED_INVALID_MSG)
    }
}

pub struct DefaultUriGenerator<'a> {
    protocol: &'a str,
    host: &'a str,
    namespace: &'a str,
}

impl<'a> DefaultUriGenerator<'a> {
    pub fn new(protocol: &'a str, host: &'a str, namespace: &'a str) -> Self {
        assert!(
            !protocol.is_empty() && !host.is_empty() || protocol.is_empty() && host.is_empty(),
            "URL protocol and host must either be both absent or both present."
        );
        DefaultUriGenerator {
            protocol,
            host,
            namespace,
        }
    }
}

impl Default for DefaultUriGenerator<'_> {
    fn default() -> Self {
        DefaultUriGenerator::new("", "", "")
    }
}

impl<'a> UriGenerator for DefaultUriGenerator<'a> {
    fn base_url(&self) -> String {
        if self.protocol.is_empty() && self.host.is_empty() {
            self.namespace.to_string()
        } else {
            format!("{}://{}:{}", self.protocol, self.host, self.namespace)
        }
    }
}

/// How generated links are rooted. The sole public knob for link generation: passed to
/// `Router::try_new`, it decides whether every link the router mints is a bare path or an absolute
/// URL. The path *structure* is not configured here — it comes from where each resource is mounted.
pub enum BaseUri<'sch> {
    /// Path-only links (`/articles/1`), resolved by the client against the document's `self` link.
    Relative,
    /// Absolute links under a fixed origin (`https://api.example.test/articles/1`).
    Absolute(Cow<'sch, str>),
}

impl<'sch> BaseUri<'sch> {
    /// Renders a mounted path template into a concrete link. Static segments pass through unchanged;
    /// a `:name` segment is replaced by its resolved value, percent-encoded so an arbitrary id never
    /// breaks out of its path segment. An `Absolute` base prepends its origin. A dynamic segment
    /// with no resolution, or a result that fails to parse, is a framework fault (internal error).
    pub(crate) fn render(
        &self,
        template: &[Cow<str>],
        resolved: &HashMap<&str, Cow<str>>,
    ) -> Result<Uri, Error> {
        let base = match self {
            BaseUri::Relative => std::iter::once(Cow::Borrowed("")),
            BaseUri::Absolute(base) => std::iter::once(Cow::Borrowed(base.trim_end_matches("/"))),
        };

        let path = template.iter().cloned().map(|segment| {
            if let Some(name) = segment.strip_prefix(':') {
                resolved
                    .get(name)
                    .map(|value| urlencoding::encode(value))
                    .ok_or_else(|| LinkGenerationError {
                        message: format!("dynamic segment ':{name}' was not resolved"),
                    })
            } else {
                Ok(segment)
            }
        });

        let link = base
            .into_iter()
            .map(Ok)
            .chain(path)
            .collect::<Result<Vec<_>, _>>()?
            .join("/");

        link.parse::<Uri>().map_err(|error| {
            LinkGenerationError {
                message: format!("'{link}' is not a valid URI: {error}"),
            }
            .into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_wrappers::StatusCode;

    /// A borrowed path template, as the router captures it.
    fn template(segments: &[&'static str]) -> Vec<Cow<'static, str>> {
        segments.iter().copied().map(Cow::Borrowed).collect()
    }

    /// The resolution map a controller hands back for a template's dynamic segments.
    fn resolved(
        entries: &[(&'static str, &'static str)],
    ) -> HashMap<&'static str, Cow<'static, str>> {
        entries
            .iter()
            .map(|(name, value)| (*name, Cow::Borrowed(*value)))
            .collect()
    }

    fn absolute() -> BaseUri<'static> {
        BaseUri::Absolute(Cow::Borrowed("https://api.example.test"))
    }

    #[test]
    fn renders_a_relative_resource_path() -> Result<(), Error> {
        let uri =
            BaseUri::Relative.render(&template(&["articles", ":id"]), &resolved(&[("id", "1")]))?;
        assert_eq!(uri.to_string(), "/articles/1");
        Ok(())
    }

    #[test]
    fn renders_an_absolute_resource_path_under_the_origin() -> Result<(), Error> {
        let uri = absolute().render(&template(&["articles", ":id"]), &resolved(&[("id", "1")]))?;
        assert_eq!(uri.to_string(), "https://api.example.test/articles/1");
        Ok(())
    }

    #[test]
    fn renders_a_relationship_path() -> Result<(), Error> {
        let uri = absolute().render(
            &template(&["articles", ":id", "relationships", "comments"]),
            &resolved(&[("id", "1")]),
        )?;
        assert_eq!(
            uri.to_string(),
            "https://api.example.test/articles/1/relationships/comments"
        );
        Ok(())
    }

    #[test]
    fn renders_a_related_path() -> Result<(), Error> {
        let uri = absolute().render(
            &template(&["articles", ":id", "comments"]),
            &resolved(&[("id", "1")]),
        )?;
        assert_eq!(
            uri.to_string(),
            "https://api.example.test/articles/1/comments"
        );
        Ok(())
    }

    #[test]
    fn percent_encodes_a_substituted_segment() -> Result<(), Error> {
        let uri = BaseUri::Relative
            .render(&template(&["items", ":id"]), &resolved(&[("id", "a b/c")]))?;
        assert_eq!(uri.to_string(), "/items/a%20b%2Fc");
        Ok(())
    }

    #[test]
    fn an_unresolved_segment_is_an_internal_error() {
        let error = BaseUri::Relative
            .render(&template(&["articles", ":id"]), &resolved(&[]))
            .expect_err("an unresolved segment must fail");
        assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
