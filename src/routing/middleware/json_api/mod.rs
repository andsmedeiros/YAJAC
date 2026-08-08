mod media_type;
mod negotiation;

#[cfg(test)]
mod tests;

use super::{ResourceHandler, ResourceMiddleware};
use crate::database::adapters::Adapter as AdapterInterface;
use crate::http_wrappers::StatusCode;
use crate::json_api::error::Error as JsonApiError;
use crate::routing::controller::ResourceContext;
use crate::routing::{Error, ResourceResult, RouteParameters, respond_with};
use crate::serialisation::factories::to_document;
use crate::serialisation::uri_generator::UriGenerator;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue};
use log::error;
use media_type::{JSONAPI_MEDIA_TYPE, JsonApiMediaType};
use negotiation::ContentNegotiator;

/// The profiles the server applies natively.
/// TODO: Spec and publish those profiles.
const PAGINATION_PROFILE: &str = "https://example.com/profiles/pagination";
const FILTER_PROFILE: &str = "https://example.com/profiles/filter";

/// The framework boundary at every resourceful route: the outermost resource middleware, seeded at
/// registration. It negotiates content (`415`/`406`), catches a resource-tier error and renders it
/// into an error document, and stamps the JSON:API `Content-Type` on the response coming back — so
/// the tier is JSON:API whether it succeeds or fails. A stateless ZST, shared across every route.
#[derive(Default)]
pub(crate) struct JsonApi;

impl<'sch, Adapter: AdapterInterface + 'sch> ResourceMiddleware<'sch, Adapter> for JsonApi {
    fn handle<'req>(
        &self,
        mut context: ResourceContext<'sch, 'req, Adapter>,
        next: &ResourceHandler<'sch, 'req, Adapter>,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        // Captured before `next` consumes the context, for rendering an error document afterwards;
        // each is a request-scoped view over router-owned data, so it outlives the context.
        let uri = context.uri();
        let base_uri = context.base_uri();
        let mount_table = context.mount_table();

        let uses_filter_profile = context
            .query_parameters()
            .map(|parameters| parameters.filter.is_some());

        ContentNegotiator::negotiate(&mut context)
            .and_then(|()| next(context))
            .and_then(|mut response| {
                let mut content_type = JsonApiMediaType::default();

                if uses_filter_profile.is_ok_and(|value| value) {
                    content_type.profiles.push(FILTER_PROFILE);
                }

                if response.body().as_ref().is_some_and(|document| {
                    document.links.as_ref().is_some_and(|links| {
                        links.pagination.as_ref().is_some_and(|pagination| {
                            pagination.first.is_some()
                                || pagination.last.is_some()
                                || pagination.prev.is_some()
                                || pagination.next.is_some()
                        })
                    })
                }) {
                    content_type.profiles.push(PAGINATION_PROFILE);
                }

                let content_type =
                    HeaderValue::try_from(content_type.to_string()).map_err(|error| {
                        Error::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "GeneratedInvalidContentType",
                            format!(
                                "The server generated an invalid 'Content-Type' header: {error}"
                            ),
                        )
                    })?;
                response.headers_mut().insert(CONTENT_TYPE, content_type);
                Ok(response)
            })
            .or_else(|error| {
                // The error boundary: render any resource-tier error into an error document,
                // redacting (and logging) a 5xx so nothing internal leaks. The generator is never
                // driven — an errors document carries no resource links — so lend it a bare view.
                let status = error.status_code();
                let error = if status.is_server_error() {
                    error!("{uri} failed: {error:?}");
                    if cfg!(debug_assertions) {
                        error
                    } else {
                        Error::new(
                            status.clone(),
                            "InternalServerError",
                            "Internal server error",
                        )
                    }
                } else {
                    error
                };

                let route = RouteParameters::new();
                let headers = HeaderMap::new();
                let generator = UriGenerator::new(base_uri, mount_table, &route, &headers);
                let document =
                    to_document(vec![JsonApiError::from(error)], Vec::new(), uri, &generator)?;
                respond_with(status, Some(document)).map(|mut response| {
                    response
                        .headers_mut()
                        .insert(CONTENT_TYPE, HeaderValue::from_static(JSONAPI_MEDIA_TYPE));
                    response
                })
            })
    }
}
