mod media_type;
mod negotiation;

#[cfg(test)]
mod tests;

use super::{ResourceHandler, ResourceMiddleware};
use crate::database::adapters::Adapter as AdapterInterface;
use crate::error::Error;
use crate::http_wrappers::StatusCode;
use crate::json_api::error::Error as JsonApiError;
use crate::json_api::primary_content::PrimaryContent;
use crate::routing::context::ResourceContext;
use crate::routing::{Error as RoutingError, ResourceResult, respond_with};
use crate::serialisation::factories::to_document;
use crate::serialisation::uri_generator::NullUriGenerator;
use http::HeaderValue;
use http::header::{CONTENT_TYPE, LOCATION};
use log::error;
use media_type::{JSONAPI_MEDIA_TYPE, JsonApiMediaType};
use negotiation::ContentNegotiator;
use std::borrow::Cow;

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
        let uri = context.uri();

        let uses_filter_profile = context
            .query_parameters()
            .map(|parameters| parameters.filter.is_some());

        ContentNegotiator::negotiate(&mut context)
            .map_err(Error::from)
            .and_then(|()| next(context))
            .and_then(|mut response| {
                let mut content_type = JsonApiMediaType::default();

                if uses_filter_profile.is_ok_and(|value| value) {
                    content_type.profiles.push(FILTER_PROFILE);
                }

                if let Some(document) = response.body() {
                    if let Some(ref links) = document.links
                        && let Some(ref pagination) = links.pagination
                        && [
                            &pagination.first,
                            &pagination.last,
                            &pagination.prev,
                            &pagination.next,
                        ]
                        .into_iter()
                        .any(Option::is_some)
                    {
                        content_type.profiles.push(PAGINATION_PROFILE);
                    }

                    if response.status() == StatusCode::CREATED
                        && let PrimaryContent::Record { ref data } = document.content
                        && let Some(ref links) = data.links
                    {
                        let location =
                            HeaderValue::try_from(links.this.to_string()).map_err(|error| {
                                RoutingError::GeneratedInvalidHeader {
                                    header: LOCATION.to_string(),
                                    message: error.to_string(),
                                }
                            })?;
                        response.headers_mut().insert(LOCATION, location);
                    }
                }

                let content_type =
                    HeaderValue::try_from(content_type.to_string()).map_err(|error| {
                        RoutingError::GeneratedInvalidHeader {
                            header: CONTENT_TYPE.to_string(),
                            message: error.to_string(),
                        }
                    })?;
                response.headers_mut().insert(CONTENT_TYPE, content_type);
                Ok(response)
            })
            .or_else(|mut error| {
                // Render any resource-tier error into an error document. A 5xx is logged whole and
                // then stripped, so the detail reaches the operator and never the client.
                let status = error.status.clone();

                if status.is_server_error() {
                    error!("{uri} failed: {error:?}");

                    let is_development = cfg!(debug_assertions);
                    if !is_development {
                        redact_error(&mut error);
                    }
                }

                // An errors document renders no per-record links, so it needs no request-bound
                // generator: the null generator refuses any link, asserting exactly that.
                let document = to_document(
                    vec![JsonApiError::from(error)],
                    Vec::new(),
                    uri,
                    &NullUriGenerator,
                )?;
                respond_with(status, Some(document)).map(|mut response| {
                    response
                        .headers_mut()
                        .insert(CONTENT_TYPE, HeaderValue::from_static(JSONAPI_MEDIA_TYPE));
                    response
                })
            })
    }
}

/// Strips an error bound for a client down to its status. A `5xx` names broken internals — schema
/// and column names, adapter messages, unmet invariants — none of which is the client's to see, and
/// all of which the boundary has already logged.
fn redact_error(error: &mut Error) {
    *error = Error {
        status: error.status.clone(),
        code: Cow::Borrowed("InternalServerError"),
        title: Cow::Borrowed("An unexpected error occurred"),
        detail: "The server failed to process this request".to_string(),
        source: None,
        meta: None,
    };
}
