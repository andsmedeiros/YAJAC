use crate::http_wrappers::StatusCode;
use crate::routing::Error;
use crate::utils::MediaType;
use itertools::Itertools;
use std::fmt::Display;

/// The JSON:API media type, sans parameters.
pub(super) const JSONAPI_MEDIA_TYPE: &str = "application/vnd.api+json";

/// The set of media types accepted by the server as matches for JSON:API — the literal type and the
/// wildcards that subsume it.
pub(super) const ACCEPTED_MEDIA_TYPES: [&str; 3] = [JSONAPI_MEDIA_TYPE, "application/*", "*/*"];

/// A parsed JSON:API media type: the profiles and extensions applied to it, and its quality weight.
#[derive(Debug, Default, PartialEq)]
pub(super) struct JsonApiMediaType<'a> {
    pub(super) profiles: Vec<&'a str>,
    pub(super) extensions: Vec<&'a str>,
    pub(super) quality: Option<f64>,
}

impl<'a> JsonApiMediaType<'a> {
    /// Builds a JSON:API media type from a generic `MediaType`, accepting the literal type and the
    /// wildcards that subsume it. Errors on any media type parameter other than `ext`, `profile`,
    /// and the `q` weight; profile and extension URIs are collected verbatim, to be matched by
    /// equality. A media type that is not a JSON:API match is a caller invariant violation (callers
    /// pre-filter), hence the 500.
    pub(super) fn try_new(media_type: MediaType<'a>) -> Result<Self, Error> {
        ACCEPTED_MEDIA_TYPES
            .iter()
            .any(|value| media_type.essence.eq_ignore_ascii_case(value))
            .then(|| {
                let mut json_api_media_type = JsonApiMediaType::default();

                for (parameter, value) in media_type.parameters {
                    if parameter.eq_ignore_ascii_case("profile") {
                        json_api_media_type.profiles.append(&mut value.split(' ').collect());
                    } else if parameter.eq_ignore_ascii_case("ext") {
                        json_api_media_type.extensions.append(&mut value.split(' ').collect());
                    } else if parameter.eq_ignore_ascii_case("q") {
                        let value = value.parse().map_err(|_| {
                            Error::new(
                                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                                "UnsupportedQualityValue",
                                "A media type parameter contains a quality value in an unsupported format",
                            )
                        })?;
                        json_api_media_type.quality = Some(value);
                    } else {
                        return Err(Error::new(
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            "UnsupportedMediaTypeParameter",
                            "An unsupported media type parameter was specified",
                        ));
                    }
                }

                Ok(json_api_media_type)
            })
            .ok_or_else(|| {
                Error::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InvalidJsonApiMediaType",
                    format!(
                        "Attempted to build a JSON:API media type from a header of type {}",
                        media_type.essence
                    ),
                )
            })?
    }
}

impl<'a> Display for JsonApiMediaType<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{JSONAPI_MEDIA_TYPE}")?;

        if !self.profiles.is_empty() {
            write!(f, ";profile=\"{}\"", self.profiles.iter().join(" "))?;
        }

        if !self.extensions.is_empty() {
            write!(f, ";ext=\"{}\"", self.extensions.iter().join(" "))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::JsonApiMediaType;
    use crate::http_wrappers::StatusCode;
    use crate::routing::Error;
    use crate::utils::MediaType;

    /// The first media type parsed from a header value — the shape `try_new` consumes.
    fn media_type(header: &str) -> MediaType<'_> {
        MediaType::list_from(header)
            .next()
            .expect("a parsed media type")
    }

    #[test]
    fn parses_a_bare_media_type() -> Result<(), Error> {
        let parsed = JsonApiMediaType::try_new(media_type("application/vnd.api+json"))?;

        assert_eq!(parsed.profiles, Vec::<&str>::new());
        assert_eq!(parsed.extensions, Vec::<&str>::new());
        assert_eq!(parsed.quality, None);

        Ok(())
    }

    #[test]
    fn accepts_wildcards() -> Result<(), Error> {
        JsonApiMediaType::try_new(media_type("*/*"))?;
        JsonApiMediaType::try_new(media_type("application/*"))?;

        Ok(())
    }

    #[test]
    fn collects_space_separated_profiles() -> Result<(), Error> {
        let parsed = JsonApiMediaType::try_new(media_type(
            "application/vnd.api+json; profile=\"https://p/1 https://p/2\"",
        ))?;

        assert_eq!(parsed.profiles, vec!["https://p/1", "https://p/2"]);

        Ok(())
    }

    #[test]
    fn collects_space_separated_extensions() -> Result<(), Error> {
        let parsed = JsonApiMediaType::try_new(media_type(
            "application/vnd.api+json; ext=\"https://e/1 https://e/2\"",
        ))?;

        assert_eq!(parsed.extensions, vec!["https://e/1", "https://e/2"]);

        Ok(())
    }

    #[test]
    fn parses_quality() -> Result<(), Error> {
        let parsed = JsonApiMediaType::try_new(media_type("application/vnd.api+json; q=0.8"))?;

        assert_eq!(parsed.quality, Some(0.8));

        Ok(())
    }

    #[test]
    fn rejects_a_malformed_quality() {
        let error =
            JsonApiMediaType::try_new(media_type("application/vnd.api+json; q=high")).unwrap_err();

        assert_eq!(error.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn rejects_a_disallowed_parameter() {
        let error =
            JsonApiMediaType::try_new(media_type("application/vnd.api+json; charset=utf-8"))
                .unwrap_err();

        assert_eq!(error.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn rejects_a_foreign_media_type() {
        let error = JsonApiMediaType::try_new(media_type("text/html")).unwrap_err();

        assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn displays_a_bare_media_type() {
        assert_eq!(
            JsonApiMediaType::default().to_string(),
            "application/vnd.api+json"
        );
    }

    #[test]
    fn displays_profiles_before_extensions() {
        let media_type = JsonApiMediaType {
            profiles: vec!["https://p/1", "https://p/2"],
            extensions: vec!["https://e/1"],
            quality: None,
        };

        assert_eq!(
            media_type.to_string(),
            "application/vnd.api+json;profile=\"https://p/1 https://p/2\";ext=\"https://e/1\""
        );
    }
}
