use crate::http_wrappers::Uri;
use crate::serialisation::error::Error;
use std::borrow::Cow;
use std::collections::HashMap;

/// How generated links are rooted. The sole public knob for link generation: passed to
/// `Router::try_new`, it decides whether every link the router mints is a bare path or an absolute
/// URL. The path *structure* is not configured here — it comes from where each resource is mounted.
#[derive(Clone)]
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
                    .ok_or_else(|| Error::LinkGenerationError {
                        message: format!("dynamic segment ':{name}' was not resolved"),
                    })
            } else if segment.starts_with('*') {
                Err(Error::LinkGenerationError {
                    message: "a wildcard segment cannot appear in a generated link".to_string(),
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

        link.parse::<Uri>()
            .map_err(|error| Error::LinkGenerationError {
                message: format!("'{link}' is not a valid URI: {error}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn an_unresolved_segment_is_a_link_generation_error() {
        let error = BaseUri::Relative
            .render(&template(&["articles", ":id"]), &resolved(&[]))
            .expect_err("an unresolved segment must fail");
        assert!(matches!(error, Error::LinkGenerationError { .. }));
    }

    #[test]
    fn a_wildcard_segment_is_a_link_generation_error() {
        let error = BaseUri::Relative
            .render(&template(&["files", "*"]), &resolved(&[]))
            .expect_err("a wildcard cannot appear in a generated link");
        assert!(matches!(error, Error::LinkGenerationError { .. }));
    }
}
