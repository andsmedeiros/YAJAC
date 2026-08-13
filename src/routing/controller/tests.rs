use super::ResourceController;
use crate::database::adapters::SqliteAdapter;
use crate::database::connection_manager::ConnectionManager;
use crate::http_wrappers::{StatusCode, Uri};
use crate::json_api::document::Document;
use crate::json_api::identifier::Identifier;
use crate::json_api::primary_content::PrimaryContent;
use crate::json_api::resource::Resource;
use crate::routing::{BaseUri, Router};
use crate::serialisation::ByteStream;
use crate::test_support::{Result, database, fixtures, routing};
use http::{Method, Response};
use serde_json::json;
use std::io::empty;
use test_log::test;

type Manager<'sch> = ConnectionManager<'sch, SqliteAdapter>;

#[derive(Default)]
struct Articles;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {}

#[derive(Default)]
struct Authors;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Authors {}

/// A router mounting both resources bare, so every canonical endpoint is enumerated from its schema.
fn build_router<'sch>(
    manager: &'sch Manager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = manager.registry().schema("articles")?;
    let authors = manager.registry().schema("authors")?;

    Router::try_new(base_uri, |root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Authors>("authors", authors)
    })
    .map_err(Into::into)
}

/// The single resource a response's primary data holds.
fn require_record(response: &Response<Option<Document>>) -> Result<&Resource> {
    if let Some(PrimaryContent::Record { data }) =
        response.body().as_ref().map(|document| &document.content)
    {
        Ok(data)
    } else {
        Err("primary data is not a single resource".into())
    }
}

/// The resources a response's primary data holds, in the order they were serialised.
fn require_collection(response: &Response<Option<Document>>) -> Result<&[Resource]> {
    if let Some(PrimaryContent::Collection { data }) =
        response.body().as_ref().map(|document| &document.content)
    {
        Ok(data)
    } else {
        Err("primary data is not a collection".into())
    }
}

#[test]
fn index_yields_every_record_of_the_resource() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let response = Articles.index(context)?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "1".to_string()
            },
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "2".to_string()
            }
        ]
    );

    Ok(())
}

#[test]
fn show_yields_the_addressed_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let response = Articles.show(context)?;
    let record = require_record(&response)?;
    let title = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("title"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "articles".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(title, Some(&json!("On Borrowed Lifetimes")));

    Ok(())
}

#[test]
fn show_of_an_absent_record_is_not_found() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/404")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let status = Articles.show(context).err().map(|error| error.status);

    assert_eq!(status, Some(StatusCode::NOT_FOUND));

    Ok(())
}
