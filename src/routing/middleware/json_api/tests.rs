use super::{FILTER_PROFILE, JsonApi, PAGINATION_PROFILE};
use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::connection_manager::ConnectionManager;
use crate::database::registry::Registry;
use crate::database::schema::{AttributeType, Schema, SchemaBuilder};
use crate::http_wrappers::{StatusCode, Uri};
use crate::json_api::document::{Document, Links, Pagination};
use crate::json_api::links::Link;
use crate::json_api::primary_content::PrimaryContent;
use crate::routing::controller::ResourceContext;
use crate::routing::middleware::ResourceMiddleware;
use crate::routing::mount_table::MountTable;
use crate::routing::{BaseUri, Error, PrimaryContext, PrimaryRequest, RouteParameters};
use crate::routing::{ResourceResult, respond_with};
use crate::serialisation::ByteStream;
use http::header::CONTENT_TYPE;
use http::{HeaderName, Response};
use std::error::Error as StdError;
use std::io::Cursor;

type Manager = ConnectionManager<'static, SqliteAdapter>;
type TestResult = Result<(), Box<dyn StdError>>;

fn schemas() -> [SchemaBuilder<'static>; 1] {
    [SchemaBuilder::table("articles").attribute("title", AttributeType::Text)]
}

/// A manager over a single registered schema — no DDL, as the boundary never touches the database.
fn manager() -> Result<Manager, Box<dyn StdError>> {
    Ok(ConnectionManager::new(
        Registry::try_new(schemas())?,
        Pool::memory()?,
    ))
}

fn schema<'sch>(manager: &'sch Manager, name: &str) -> &'sch Schema<'sch> {
    manager
        .registry()
        .schema(name)
        .expect("a registered schema")
}

/// A primary request with a streamed body and the given headers.
fn request(
    method: &str,
    uri: &str,
    body: &str,
    headers: &[(HeaderName, &str)],
) -> Result<PrimaryRequest, Box<dyn StdError>> {
    let stream: ByteStream = Box::new(Cursor::new(body.as_bytes().to_vec()));
    let request = headers
        .iter()
        .fold(
            http::Request::builder().method(method).uri(uri),
            |builder, (name, value)| builder.header(name, *value),
        )
        .body(stream)?;
    Ok(request)
}

/// Builds a `ResourceContext` bound to the `articles` schema and runs the `JsonApi` boundary over it
/// with `next` as the wrapped handler.
fn handle<N>(manager: &Manager, request: PrimaryRequest, next: N) -> ResourceResult
where
    N: for<'req> Fn(ResourceContext<'_, 'req, SqliteAdapter>) -> ResourceResult,
{
    let base = BaseUri::Relative;
    let mounts = MountTable::default();
    let uri: Uri = request.uri().clone().into();
    let context = PrimaryContext::from_request(
        manager,
        &base,
        &mounts,
        &uri,
        RouteParameters::new(),
        request,
    );

    JsonApi.handle(
        ResourceContext::new(schema(manager, "articles"), context),
        &next,
    )
}

/// The `Content-Type` of a response, if present and readable.
fn content_type(response: &Response<Option<Document>>) -> Option<String> {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[test]
fn stamps_the_json_api_content_type_on_success() -> TestResult {
    let manager = manager()?;
    let response = handle(
        &manager,
        request("GET", "/articles", "", &[])?,
        |_context| Ok(Response::new(None)),
    )?;

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        content_type(&response).as_deref(),
        Some("application/vnd.api+json")
    );

    Ok(())
}

#[test]
fn stamps_the_json_api_content_type_on_a_rendered_error() -> TestResult {
    let manager = manager()?;
    let response = handle(
        &manager,
        request("GET", "/articles", "", &[])?,
        |_context| {
            Err(Error::new(
                StatusCode::NOT_FOUND,
                "NotFound",
                "The article does not exist",
            ))
        },
    )?;

    assert_eq!(response.status().as_u16(), 404);
    assert_eq!(
        content_type(&response).as_deref(),
        Some("application/vnd.api+json")
    );

    let document = serde_json::to_value(response.body())?;
    assert!(document["errors"].is_array());

    Ok(())
}

#[test]
fn rejects_a_bad_content_type_without_running_the_handler() -> TestResult {
    let manager = manager()?;
    let request = request(
        "POST",
        "/articles",
        "{}",
        &[(CONTENT_TYPE, "application/vnd.api+json; charset=utf-8")],
    )?;

    let response = handle(&manager, request, |_context| {
        panic!("the handler must not run when negotiation fails")
    })?;

    assert_eq!(response.status().as_u16(), 415);
    assert_eq!(
        content_type(&response).as_deref(),
        Some("application/vnd.api+json")
    );

    Ok(())
}

#[test]
fn stamps_the_filter_profile_when_the_request_filters() -> TestResult {
    let manager = manager()?;
    let response = handle(
        &manager,
        request("GET", "/articles?filter[title]=Rust", "", &[])?,
        |_context| Ok(Response::new(None)),
    )?;

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        content_type(&response),
        Some(format!(
            "application/vnd.api+json;profile=\"{FILTER_PROFILE}\""
        ))
    );

    Ok(())
}

#[test]
fn stamps_the_pagination_profile_when_the_response_paginates() -> TestResult {
    let manager = manager()?;
    let response = handle(
        &manager,
        request("GET", "/articles", "", &[])?,
        |_context| {
            let document = Document {
                content: PrimaryContent::Empty { data: () },
                meta: None,
                jsonapi: None,
                links: Some(Links {
                    this: None,
                    related: None,
                    described_by: None,
                    pagination: Some(Pagination {
                        first: Some(Link::Uri("/articles?page=1".parse().expect("a valid uri"))),
                        last: None,
                        prev: None,
                        next: None,
                    }),
                }),
                included: None,
            };
            respond_with(StatusCode::OK, Some(document))
        },
    )?;

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        content_type(&response),
        Some(format!(
            "application/vnd.api+json;profile=\"{PAGINATION_PROFILE}\""
        ))
    );

    Ok(())
}
