use super::media_type::{ACCEPTED_MEDIA_TYPES, JSONAPI_MEDIA_TYPE, JsonApiMediaType};
use crate::database::adapters::Adapter as AdapterInterface;
use crate::routing::Error;
use crate::routing::context::ResourceContext;
use crate::utils::MediaType;
use http::header::{ACCEPT, CONTENT_TYPE};
use http::{HeaderMap, HeaderName};
use itertools::Itertools;
use std::collections::HashSet;
use std::sync::LazyLock;

/// The JSON:API extensions the server supports — none, so any `ext` is unsupported.
static SUPPORTED_EXTENSIONS: LazyLock<HashSet<&str>> = LazyLock::new(HashSet::new);

/// Aggregates behaviour for inspecting and validating the `Content-Type` and `Accept` headers of a
/// request.
pub(super) struct ContentNegotiator;

impl ContentNegotiator {
    /// Checks the request's `Accept` and `Content-Type` for standard conformity.
    ///
    /// # `Content-Type`
    ///
    /// Any body-carrying request must have a proper JSON:API `Content-Type`. Bodyless requests are
    /// not required to carry one, but if they do, it gets validated also.
    ///
    /// # `Accept`
    ///
    /// The header is never mandatory, but if provided, it must carry at least one valid instance
    /// of the JSON:API media type.
    ///
    /// On validation, a media type's extensions are matched against a list of supported extensions,
    /// raising an error naming the offenders on failure. Profiles are informative and ignored.
    pub(super) fn negotiate<'sch: 'req, 'req, Adapter: AdapterInterface>(
        context: &mut ResourceContext<'sch, 'req, Adapter>,
    ) -> Result<(), Error> {
        let content_type_required = context.contains_body()?;
        if let Some(extensions) =
            Self::extract_json_api_content_type(context.headers(), content_type_required)?
                .map(|content_type| {
                    content_type
                        .extensions
                        .iter()
                        .filter(|extension| !SUPPORTED_EXTENSIONS.contains(*extension))
                        .map(|extension| extension.to_string())
                        .collect_vec()
                })
                .filter(|extensions| !extensions.is_empty())
        {
            return Err(Error::UnsupportedJsonApiExtension { extensions });
        }

        if let Some(media_types) = Self::extract_json_api_accept(context.headers())?
            && media_types.iter().all(|accept| {
                accept
                    .extensions
                    .iter()
                    .any(|extension| !SUPPORTED_EXTENSIONS.contains(extension))
            })
        {
            return Err(Error::UnsatisfiableJsonApiExtension {
                extensions: media_types
                    .iter()
                    .flat_map(|accept| accept.extensions.iter())
                    .filter(|extension| !SUPPORTED_EXTENSIONS.contains(*extension))
                    .map(|extension| extension.to_string())
                    .unique()
                    .collect(),
            });
        }

        Ok(())
    }

    /// Attempts to read a header value from the supplied header map.
    /// If the header is absent, returns `Ok(None)` and if its value is not valid ASCII, returns an
    /// error.
    fn read_header(headers: &HeaderMap, header: HeaderName) -> Result<Option<&str>, Error> {
        headers
            .get(&header)
            .map(|header_value| header_value.to_str())
            .transpose()
            .map_err(|error| Error::InvalidHeaderValue {
                header: header.to_string(),
                message: error.to_string(),
            })
    }

    /// Checks whether the header map contains a `Content-Type` header and attempts to extract the
    /// media type it describes, coercing it to a `JsonApiMediaType` in the process.
    ///
    /// If `required` is `false`, the function will be tolerant towards a missing `Content-Type`
    /// in the header map and will return `Ok(None)`. Otherwise, header absence will be translated
    /// into an error.
    fn extract_json_api_content_type(
        headers: &HeaderMap,
        required: bool,
    ) -> Result<Option<JsonApiMediaType<'_>>, Error> {
        let Some(header) = Self::read_header(headers, CONTENT_TYPE)? else {
            return required
                .then_some(Err(Error::MissingContentType))
                .transpose();
        };

        match MediaType::list_from(header).collect_array() {
            Some([media_type]) if media_type.essence.eq_ignore_ascii_case(JSONAPI_MEDIA_TYPE) => {
                JsonApiMediaType::try_new(media_type)
                    .and_then(|media_type| match media_type.quality {
                        None => Ok(media_type),
                        Some(_) => Err(Error::ContentTypeCarriesQuality),
                    })
                    .map(Some)
            }
            Some([_]) => Err(Error::UnsupportedContentType),
            _ => Err(Error::InvalidContentType),
        }
    }

    /// Checks whether the header map contains an `Accept` header and attempts to extract the
    /// media types described by it, coercing them to `JsonApiMediaType` in the process.
    /// An `Accept` header with multiple non-JSON:API media types is accepted, as long as one media
    /// type is `application/vnd.api+json`.
    ///
    /// If the header is absent, returns `Ok(None)`.
    fn extract_json_api_accept(
        headers: &HeaderMap,
    ) -> Result<Option<Vec<JsonApiMediaType<'_>>>, Error> {
        let Some(header) = Self::read_header(headers, ACCEPT)? else {
            return Ok(None);
        };

        let media_types = MediaType::list_from(header).collect_vec();
        if media_types.is_empty() {
            return Err(Error::InvalidAcceptHeader);
        }

        let matching_media_types = media_types
            .into_iter()
            .filter(|media_type| {
                ACCEPTED_MEDIA_TYPES
                    .iter()
                    .any(|value| media_type.essence.eq_ignore_ascii_case(value))
            })
            .collect_vec();

        if matching_media_types.is_empty() {
            return Err(Error::NoAcceptableMediaType);
        }

        let json_api_media_types: Vec<JsonApiMediaType> = matching_media_types
            .into_iter()
            .map(JsonApiMediaType::try_new)
            .filter(|result| {
                result
                    .as_ref()
                    .is_ok_and(|media_type| media_type.quality != Some(0.0))
            })
            .try_collect()?;

        if json_api_media_types.is_empty() {
            return Err(Error::UnusableAcceptMediaTypes);
        }

        Ok(Some(json_api_media_types))
    }
}

#[cfg(test)]
mod tests {
    use super::super::media_type::JsonApiMediaType;
    use super::ContentNegotiator;
    use crate::routing::Error;
    use http::header::{ACCEPT, CONTENT_TYPE};
    use http::{HeaderMap, HeaderName, HeaderValue};

    /// A header map carrying a single named header.
    fn with_header(name: HeaderName, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            name,
            HeaderValue::from_str(value).expect("a valid header value"),
        );
        headers
    }

    // --- Content-Type ------------------------------------------------------

    #[test]
    fn content_type_absent_and_optional_is_none() -> Result<(), Error> {
        let headers = HeaderMap::new();
        let extracted = ContentNegotiator::extract_json_api_content_type(&headers, false)?;

        assert_eq!(extracted, None);

        Ok(())
    }

    #[test]
    fn content_type_absent_but_required_is_unsupported() {
        let error =
            ContentNegotiator::extract_json_api_content_type(&HeaderMap::new(), true).unwrap_err();

        assert_eq!(error, Error::MissingContentType);
    }

    #[test]
    fn content_type_bare_is_extracted() -> Result<(), Error> {
        let headers = with_header(CONTENT_TYPE, "application/vnd.api+json");
        let extracted = ContentNegotiator::extract_json_api_content_type(&headers, true)?;

        assert_eq!(extracted, Some(JsonApiMediaType::default()));

        Ok(())
    }

    #[test]
    fn content_type_with_a_disallowed_parameter_is_unsupported() {
        let error = ContentNegotiator::extract_json_api_content_type(
            &with_header(CONTENT_TYPE, "application/vnd.api+json; charset=utf-8"),
            true,
        )
        .unwrap_err();

        assert_eq!(
            error,
            Error::UnsupportedMediaTypeParameter {
                parameter: "charset".to_string()
            }
        );
    }

    #[test]
    fn content_type_with_a_quality_is_unsupported() {
        let error = ContentNegotiator::extract_json_api_content_type(
            &with_header(CONTENT_TYPE, "application/vnd.api+json; q=1"),
            true,
        )
        .unwrap_err();

        assert_eq!(error, Error::ContentTypeCarriesQuality);
    }

    #[test]
    fn content_type_of_a_foreign_type_is_unsupported() {
        let error = ContentNegotiator::extract_json_api_content_type(
            &with_header(CONTENT_TYPE, "text/html"),
            true,
        )
        .unwrap_err();

        assert_eq!(error, Error::UnsupportedContentType);
    }

    #[test]
    fn content_type_of_a_wildcard_is_unsupported() {
        let error = ContentNegotiator::extract_json_api_content_type(
            &with_header(CONTENT_TYPE, "*/*"),
            true,
        )
        .unwrap_err();

        assert_eq!(error, Error::UnsupportedContentType);
    }

    #[test]
    fn content_type_with_multiple_values_is_unsupported() {
        let error = ContentNegotiator::extract_json_api_content_type(
            &with_header(CONTENT_TYPE, "application/vnd.api+json, text/html"),
            true,
        )
        .unwrap_err();

        assert_eq!(error, Error::InvalidContentType);
    }

    // --- Accept ------------------------------------------------------------

    #[test]
    fn accept_absent_is_none() -> Result<(), Error> {
        let headers = HeaderMap::new();
        let extracted = ContentNegotiator::extract_json_api_accept(&headers)?;

        assert_eq!(extracted, None);

        Ok(())
    }

    #[test]
    fn accept_of_the_json_api_type_is_extracted() -> Result<(), Error> {
        let headers = with_header(ACCEPT, "application/vnd.api+json");
        let extracted = ContentNegotiator::extract_json_api_accept(&headers)?;

        assert_eq!(extracted, Some(vec![JsonApiMediaType::default()]));

        Ok(())
    }

    #[test]
    fn accept_of_a_wildcard_matches() -> Result<(), Error> {
        let any = with_header(ACCEPT, "*/*");
        let application = with_header(ACCEPT, "application/*");

        assert_eq!(
            ContentNegotiator::extract_json_api_accept(&any)?,
            Some(vec![JsonApiMediaType::default()])
        );
        assert_eq!(
            ContentNegotiator::extract_json_api_accept(&application)?,
            Some(vec![JsonApiMediaType::default()])
        );

        Ok(())
    }

    #[test]
    fn accept_ignores_a_bad_instance_beside_a_clean_one() -> Result<(), Error> {
        let headers = with_header(
            ACCEPT,
            "application/vnd.api+json; charset=utf-8, application/vnd.api+json",
        );
        let extracted = ContentNegotiator::extract_json_api_accept(&headers)?;

        assert_eq!(extracted, Some(vec![JsonApiMediaType::default()]));

        Ok(())
    }

    #[test]
    fn accept_of_a_foreign_type_only_is_not_acceptable() {
        let error = ContentNegotiator::extract_json_api_accept(&with_header(ACCEPT, "text/html"))
            .unwrap_err();

        assert_eq!(error, Error::NoAcceptableMediaType);
    }

    #[test]
    fn accept_of_a_disallowed_parameter_only_is_not_acceptable() {
        let error = ContentNegotiator::extract_json_api_accept(&with_header(
            ACCEPT,
            "application/vnd.api+json; charset=utf-8",
        ))
        .unwrap_err();

        assert_eq!(error, Error::UnusableAcceptMediaTypes);
    }

    #[test]
    fn accept_with_zero_quality_only_is_not_acceptable() {
        let error = ContentNegotiator::extract_json_api_accept(&with_header(
            ACCEPT,
            "application/vnd.api+json; q=0",
        ))
        .unwrap_err();

        assert_eq!(error, Error::UnusableAcceptMediaTypes);
    }

    #[test]
    fn accept_of_an_unparseable_value_is_not_acceptable() {
        let error =
            ContentNegotiator::extract_json_api_accept(&with_header(ACCEPT, "")).unwrap_err();

        assert_eq!(error, Error::InvalidAcceptHeader);
    }
}
