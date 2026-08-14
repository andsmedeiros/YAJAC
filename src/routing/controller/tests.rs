use super::{Configuration, ResourceController};
use crate::database::adapters::SqliteAdapter;
use crate::database::connection_manager::ConnectionManager;
use crate::database::query_parameters::QueryParameters;
use crate::database::record::Record;
use crate::http_wrappers::{StatusCode, Uri};
use crate::json_api::document::Document;
use crate::json_api::identifier::Identifier;
use crate::json_api::primary_content::PrimaryContent;
use crate::json_api::resource::Resource;
use crate::routing::builders::{ResourceVerbs, RouteBuilder};
use crate::routing::{BaseUri, RouteParameters, Router};
use crate::serialisation::ByteStream;
use crate::test_support::{Result, database, fixtures, routing};
use http::{HeaderMap, Method, Response};
use serde_json::json;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, empty};
use test_log::test;

type Manager<'sch> = ConnectionManager<'sch, SqliteAdapter>;

/// The query parameter families only a collection endpoint processes, spelled against `articles`.
const COLLECTION_PARAMETERS: [&str; 4] = [
    "sort=-title",
    "page[number]=2",
    "filter[published]=eq:true",
    "search=provenance",
];

/// Every query parameter family the crate reads, spelled against `articles`.
const EVERY_PARAMETER: [&str; 6] = [
    "fields[articles]=title",
    "include=author",
    "sort=-title",
    "page[number]=2",
    "filter[published]=eq:true",
    "search=provenance",
];

#[derive(Default)]
struct Articles;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {}

#[derive(Default)]
struct Authors;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Authors {}

/// `publishers` is keyed by a text primary key with no server-side source, so its controller accepts
/// the ids a client generates.
#[derive(Default)]
struct Publishers;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Publishers {
    fn configuration(&self) -> Configuration {
        Configuration {
            accepts_client_ids: true,
        }
    }
}

#[derive(Default)]
struct Profiles;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Profiles {}

#[derive(Default)]
struct Comments;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Comments {}

/// Resolves every route parameter from the request headers instead of the record and the route.
#[derive(Default)]
struct Tenanted;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Tenanted {
    fn parameters_for_route<'req>(
        &self,
        _record: &'req Record<'sch>,
        _route: &'req RouteParameters,
        headers: &'req HeaderMap,
        required_parameters: &[&'req str],
    ) -> HashMap<&'req str, Cow<'req, str>>
    where
        'sch: 'req,
    {
        required_parameters
            .iter()
            .filter_map(|&parameter| {
                headers
                    .get(parameter)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| (parameter, Cow::Borrowed(value)))
            })
            .collect()
    }
}

/// A router mounting each resource bare, so every canonical endpoint is enumerated from its schema.
fn build_router<'sch>(
    manager: &'sch Manager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = manager.registry().schema("articles")?;
    let authors = manager.registry().schema("authors")?;
    let publishers = manager.registry().schema("publishers")?;
    let profiles = manager.registry().schema("profiles")?;
    let comments = manager.registry().schema("comments")?;

    Router::try_new(base_uri, |root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Authors>("authors", authors)
            .resource::<Publishers>("publishers", publishers)
            .resource::<Profiles>("profiles", profiles)
            .resource::<Comments>("comments", comments)
    })
    .map_err(Into::into)
}

/// A router mounting `articles` with the relationship-write handlers hand-mounted onto `author`, a
/// to-one relationship the default mounts serve read-only.
fn build_miswired_router<'sch>(
    manager: &'sch Manager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = manager.registry().schema("articles")?;

    Router::try_new(base_uri, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.member(|member| {
                member
                    .post("author", |context| Articles.link(context, "author"))
                    .delete("author", |context| Articles.unlink(context, "author"))
            })
        })
    })
    .map_err(Into::into)
}

/// A router mounting `articles` under a dynamic `:tenant` scope, so every route carries a parameter
/// the request supplies and the record cannot.
fn build_scoped_router<'sch>(
    manager: &'sch Manager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = manager.registry().schema("articles")?;

    Router::try_new(base_uri, |root| {
        root.scope(":tenant", |scope| {
            scope.resource::<Articles>("articles", articles)
        })
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

    let (status, code) = Articles
        .show(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn linkage_of_a_to_many_yields_every_member_identifier() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.linkage(context, "articles")?;
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
fn linkage_of_a_belongs_to_yields_its_identifier() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1/relationships/author")
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

    let response = Articles.linkage(context, "author")?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }
    );

    Ok(())
}

#[test]
fn linkage_of_an_unset_belongs_to_is_empty() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/3/relationships/author")
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

    let response = Articles.linkage(context, "author")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

/// `authors.profile` joins on `handle`, a unique text column that is not the primary key, so this
/// resolves the relationship without the identifier ever standing in for the join key.
#[test]
fn linkage_of_a_has_one_joined_by_a_non_primary_key_yields_its_identifier() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/relationships/profile")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.linkage(context, "profile")?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "profiles".to_string(),
            id: "1".to_string()
        }
    );

    Ok(())
}

#[test]
fn linkage_of_an_unset_has_one_is_empty() -> Result {
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
        .uri("/authors/1/relationships/profile")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.linkage(context, "profile")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

#[test]
fn linkage_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([("authors", fixtures::authors::ann()?)])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .linkage(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn related_of_a_to_many_serves_the_related_records() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.related(context, "articles")?;
    let records = require_collection(&response)?;
    let identifiers: Vec<&Identifier> = records
        .iter()
        .map(|resource| &resource.identifier)
        .collect();
    let titles: Vec<Option<&serde_json::Value>> = records
        .iter()
        .map(|resource| {
            resource
                .attributes
                .as_ref()
                .and_then(|attributes| attributes.get("title"))
        })
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
    assert_eq!(
        titles,
        vec![
            Some(&json!("On Borrowed Lifetimes")),
            Some(&json!("The Cost of a Clone"))
        ]
    );

    Ok(())
}

#[test]
fn related_of_a_belongs_to_serves_the_related_record() -> Result {
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
        .uri("/articles/1/author")
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

    let response = Articles.related(context, "author")?;
    let record = require_record(&response)?;
    let name = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("name"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(name, Some(&json!("Ann Sorensen")));

    Ok(())
}

#[test]
fn related_of_an_unset_belongs_to_is_empty() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/3/author")
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

    let response = Articles.related(context, "author")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

/// The related record is reached through `handle`, so serving it exercises the non-primary-key join
/// on the read path rather than only in the linkage.
#[test]
fn related_of_a_has_one_joined_by_a_non_primary_key_serves_the_related_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/profile")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.related(context, "profile")?;
    let record = require_record(&response)?;
    let bio = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("bio"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "profiles".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(
        bio,
        Some(&json!("Writes about systems, mostly the parts that leak."))
    );

    Ok(())
}

#[test]
fn related_includes_the_resources_the_query_solicits() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/articles?include=author")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.related(context, "articles")?;
    let included: Vec<&Identifier> = response
        .body()
        .as_ref()
        .and_then(|document| document.included.as_ref())
        .map(|resources| {
            resources
                .iter()
                .map(|resource| &resource.identifier)
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        included,
        vec![&Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }]
    );

    Ok(())
}

#[test]
fn related_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .related(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn create_persists_the_submitted_record() -> Result {
    let manager = database::build_database([("authors", fixtures::authors::ann()?)])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "articles",
            "attributes": { "title": "A Late Draft", "body": "Written in one sitting." },
            "relationships": { "author": { "data": { "type": "authors", "id": "1" } } }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let response = Articles.create(context)?;
    let record = require_record(&response)?;
    let title = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("title"));
    let author = record
        .relationships
        .as_ref()
        .and_then(|relationships| relationships.get("author"))
        .map(serde_json::to_value)
        .transpose()?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(title, Some(&json!("A Late Draft")));
    assert_eq!(
        author,
        Some(json!({
            "links": {
                "self": "/articles/1/relationships/author",
                "related": "/articles/1/author"
            },
            "data": { "type": "authors", "id": "1" }
        }))
    );

    Ok(())
}

#[test]
fn create_rejects_a_type_that_is_not_the_addressed_resource() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "attributes": { "title": "Wrong" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceTypeMismatch".to_string()));

    Ok(())
}

#[test]
fn create_rejects_an_attribute_the_schema_does_not_declare() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "attributes": { "headline": "Nope" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("UnknownAttribute".to_string()));

    Ok(())
}

#[test]
fn create_refuses_a_client_generated_id_a_controller_does_not_accept() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "id": "42", "attributes": { "title": "Numbered" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::FORBIDDEN));
    assert_eq!(code, Some("ClientGeneratedIdNotSupported".to_string()));

    Ok(())
}

#[test]
fn create_honours_a_client_generated_id_a_controller_accepts() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "publishers",
            "id": "verso-books",
            "attributes": { "name": "Verso Books" }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/publishers")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let response = Publishers.create(context)?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "publishers".to_string(),
            id: "verso-books".to_string()
        }
    );

    Ok(())
}

#[test]
fn update_changes_the_submitted_attributes() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "id": "1", "attributes": { "title": "Retitled" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let response = Articles.update(context)?;
    let record = require_record(&response)?;
    let attributes = record.attributes.as_ref();
    let title = attributes.and_then(|attributes| attributes.get("title"));
    let body = attributes.and_then(|attributes| attributes.get("body"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(title, Some(&json!("Retitled")));
    assert_eq!(
        body,
        Some(&json!("A study of provenance in layered systems."))
    );

    Ok(())
}

#[test]
fn update_rejects_an_id_that_is_not_the_addressed_one() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceIdMismatch".to_string()));

    Ok(())
}

#[test]
fn delete_removes_the_record() -> Result {
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
        .method(Method::DELETE)
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

    let deleted = Articles.delete(context)?;

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
    let (status, code) = Articles
        .show(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(deleted.body().as_ref(), None);
    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn link_adds_the_targets_keeping_the_members_already_linked() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "3" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.link(context, "articles")?;
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
            },
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "3".to_string()
            }
        ]
    );

    Ok(())
}

#[test]
fn link_of_a_target_that_does_not_exist_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "404" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RelatedRecordNotFound".to_string()));

    Ok(())
}

#[test]
fn link_of_an_absent_parent_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/404/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn link_of_a_to_one_linkage_on_a_to_many_relationship_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "id": "3" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure".to_string()));

    Ok(())
}

#[test]
fn link_of_an_identifier_naming_another_resource_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("IdentifierTypeMismatch".to_string()));

    Ok(())
}

#[test]
fn link_of_an_identifier_that_is_not_an_integer_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "the-first-one" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidIntegerIdentifier".to_string()));

    Ok(())
}

/// Linkage carries resource identifier objects; a resource object bearing attributes is not one.
#[test]
fn link_of_linkage_that_is_not_a_bare_identifier_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": [{ "type": "articles", "id": "3", "attributes": { "title": "Renamed" } }]
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidLinkage".to_string()));

    Ok(())
}

#[test]
fn link_without_a_body_is_rejected() -> Result {
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
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingLinkageBody".to_string()));

    Ok(())
}

#[test]
fn link_on_a_to_one_relationship_is_a_kind_mismatch() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_miswired_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles/1/author")
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

    let (status, code) = Articles
        .link(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("MismatchedRelationshipKind".to_string()));

    Ok(())
}

#[test]
fn link_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([("authors", fixtures::authors::ann()?)])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .link(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn relink_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": null });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/1/relationships/profile")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .relink(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn relink_replaces_the_members_of_a_to_many() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "3" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.relink(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1/relationships/author")
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
    let detached = Articles.linkage(context, "author")?;
    let content = detached.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "3".to_string()
        }]
    );
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

#[test]
fn relink_of_an_empty_collection_clears_a_to_many() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.relink(context, "articles")?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(require_collection(&response)?, []);

    Ok(())
}

#[test]
fn relink_sets_a_belongs_to() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/3/relationships/author")
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

    let response = Articles.relink(context, "author")?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "2".to_string()
        }
    );

    Ok(())
}

#[test]
fn relink_of_null_linkage_clears_a_belongs_to() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": null });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/author")
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

    let response = Articles.relink(context, "author")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

/// `authors.profile` is held by the profile's `author_handle`, so setting it writes the far side's
/// key from a value the identifier never carries.
#[test]
fn relink_sets_a_has_one_joined_by_a_non_primary_key() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "profiles", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/2/relationships/profile")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.relink(context, "profile")?;
    let record = require_record(&response)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/relationships/profile")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;
    let detached = Authors.linkage(context, "profile")?;
    let content = detached.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "profiles".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

#[test]
fn relink_of_a_to_many_linkage_on_a_to_one_relationship_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "authors", "id": "2" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/author")
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure".to_string()));

    Ok(())
}

#[test]
fn relink_of_a_target_that_does_not_exist_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "404" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/author")
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RelatedRecordNotFound".to_string()));

    Ok(())
}

#[test]
fn relink_of_an_absent_parent_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/404/relationships/author")
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn relink_without_a_body_is_rejected() -> Result {
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
        .method(Method::PATCH)
        .uri("/articles/1/relationships/author")
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingLinkageBody".to_string()));

    Ok(())
}

#[test]
fn unlink_on_a_to_one_relationship_is_a_kind_mismatch() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_miswired_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/articles/1/author")
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

    let (status, code) = Articles
        .unlink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("MismatchedRelationshipKind".to_string()));

    Ok(())
}

#[test]
fn unlink_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .unlink(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn unlink_removes_the_targets_from_the_collection() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.unlink(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "2".to_string()
        }]
    );

    Ok(())
}

#[test]
fn unlink_of_a_target_that_is_not_a_member_leaves_the_collection_whole() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "3" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.unlink(context, "articles")?;
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
fn unlink_of_an_absent_parent_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/404/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .unlink(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn unlink_without_a_body_is_rejected() -> Result {
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
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let (status, code) = Authors
        .unlink(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingLinkageBody".to_string()));

    Ok(())
}

#[test]
fn create_attaches_a_to_many_the_submitted_record_carries() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "authors",
            "attributes": { "name": "Cleo Nakamura", "handle": "cleo" },
            "relationships": {
                "articles": { "data": [{ "type": "articles", "id": "3" }] }
            }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.create(context)?;
    let record = require_record(&response)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/3/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;
    let linkage = Authors.linkage(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&linkage)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "3".to_string()
        }
    );
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "3".to_string()
        }]
    );

    Ok(())
}

/// The new author's `handle` is written into the profile's `author_handle`, a key the submitted
/// linkage never names.
#[test]
fn create_attaches_a_has_one_joined_by_a_non_primary_key() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "authors",
            "attributes": { "name": "Cleo Nakamura", "handle": "cleo" },
            "relationships": {
                "profile": { "data": { "type": "profiles", "id": "1" } }
            }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.create(context)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/profiles/1/author")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("profiles")?,
    )?;
    let related = Profiles.related(context, "author")?;
    let author = require_record(&related)?;
    let handle = author
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("handle"));

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        author.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "3".to_string()
        }
    );
    assert_eq!(handle, Some(&json!("cleo")));

    Ok(())
}

#[test]
fn create_rejects_a_document_whose_primary_data_is_not_a_resource() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("PrimaryDataIsNotAResource".to_string()));

    Ok(())
}

#[test]
fn create_rejects_an_errors_document() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "errors": [{ "title": "Nothing went wrong here" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ErrorDocumentSubmitted".to_string()));

    Ok(())
}

#[test]
fn create_rejects_a_body_that_is_not_json() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(Cursor::new(br#"{"data": }"#.to_vec()));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::BAD_REQUEST));
    assert_eq!(code, Some("MalformedRequestBody".to_string()));

    Ok(())
}

#[test]
fn create_rejects_json_that_is_not_a_document() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": 7 } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidRequestBodyContent".to_string()));

    Ok(())
}

#[test]
fn create_without_a_body_is_rejected() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::POST)
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

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingResourceBody".to_string()));

    Ok(())
}

#[test]
fn update_changes_a_belongs_to_the_submitted_record_carries() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "articles",
            "id": "1",
            "relationships": {
                "author": { "data": { "type": "authors", "id": "2" } }
            }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let response = Articles.update(context)?;
    let record = require_record(&response)?;
    let title = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("title"));

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1/relationships/author")
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
    let linkage = Articles.linkage(context, "author")?;
    let author = require_record(&linkage)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(title, Some(&json!("On Borrowed Lifetimes")));
    assert_eq!(
        author.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "2".to_string()
        }
    );

    Ok(())
}

#[test]
fn update_of_an_absent_record_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "id": "404", "attributes": { "title": "Ghostwritten" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn update_rejects_a_type_that_is_not_the_addressed_resource() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceTypeMismatch".to_string()));

    Ok(())
}

#[test]
fn update_rejects_a_resource_carrying_no_id() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "attributes": { "title": "Anonymous" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceIdMissing".to_string()));

    Ok(())
}

#[test]
fn update_without_a_body_is_rejected() -> Result {
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
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingResourceBody".to_string()));

    Ok(())
}

#[test]
fn delete_of_an_absent_record_is_not_found() -> Result {
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
        .method(Method::DELETE)
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

    let (status, code) = Articles
        .delete(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn index_serves_only_the_fields_the_query_solicits() -> Result {
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
        .uri("/articles?fields[articles]=title")
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
    let attributes: Vec<_> = require_collection(&response)?
        .iter()
        .map(|resource| serde_json::to_value(&resource.attributes))
        .collect::<std::result::Result<_, _>>()?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        attributes,
        vec![
            json!({ "title": "On Borrowed Lifetimes" }),
            json!({ "title": "The Cost of a Clone" })
        ]
    );

    Ok(())
}

#[test]
fn index_includes_the_resources_the_query_solicits() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?include=author")
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
    let included: Vec<&Identifier> = response
        .body()
        .as_ref()
        .and_then(|document| document.included.as_ref())
        .map(|resources| {
            resources
                .iter()
                .map(|resource| &resource.identifier)
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        included,
        vec![&Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }]
    );

    Ok(())
}

#[test]
fn index_orders_the_collection_the_query_sorts_by() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?sort=-title")
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
    let titles: Vec<_> = require_collection(&response)?
        .iter()
        .map(|resource| {
            resource
                .attributes
                .as_ref()
                .and_then(|attributes| attributes.get("title"))
        })
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        titles,
        vec![
            Some(&json!("The Cost of a Clone")),
            Some(&json!("On Borrowed Lifetimes")),
            Some(&json!("Notes Found in a Drawer"))
        ]
    );

    Ok(())
}

#[test]
fn index_scopes_the_collection_the_query_filters() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?filter[published]=eq:true")
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
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "1".to_string()
        }]
    );

    Ok(())
}

#[test]
fn index_truncates_the_collection_the_query_pages() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?page[size]=1&page[number]=2")
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
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "2".to_string()
        }]
    );

    Ok(())
}

#[test]
fn index_scopes_the_collection_the_query_searches() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?search=bottleneck")
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
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "2".to_string()
        }]
    );

    Ok(())
}

#[test]
fn show_serves_only_the_fields_the_query_solicits() -> Result {
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
        .uri("/articles/1?fields[articles]=title,views")
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
    let attributes = serde_json::to_value(&require_record(&response)?.attributes)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        attributes,
        json!({ "title": "On Borrowed Lifetimes", "views": 1204 })
    );

    Ok(())
}

#[test]
fn show_includes_the_resources_the_query_solicits() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1?include=comments")
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
    let included: HashSet<Identifier> = response
        .body()
        .as_ref()
        .and_then(|document| document.included.as_ref())
        .map(|resources| {
            resources
                .iter()
                .map(|resource| resource.identifier.clone())
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        included,
        HashSet::from([
            Identifier::Existing {
                kind: "comments".to_string(),
                id: "1".to_string()
            },
            Identifier::Existing {
                kind: "comments".to_string(),
                id: "2".to_string()
            }
        ])
    );

    Ok(())
}

#[test]
fn show_of_a_text_keyed_record_yields_the_addressed_record() -> Result {
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
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let response = Publishers.show(context)?;
    let record = require_record(&response)?;
    let name = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("name"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "publishers".to_string(),
            id: "acme-press".to_string()
        }
    );
    assert_eq!(name, Some(&json!("Acme Press")));

    Ok(())
}

#[test]
fn update_of_a_text_keyed_record_changes_the_submitted_attributes() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "publishers",
            "id": "acme-press",
            "attributes": { "name": "Acme Press International" }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let response = Publishers.update(context)?;
    let record = require_record(&response)?;
    let name = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("name"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "publishers".to_string(),
            id: "acme-press".to_string()
        }
    );
    assert_eq!(name, Some(&json!("Acme Press International")));

    Ok(())
}

#[test]
fn delete_of_a_text_keyed_record_removes_it() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("publishers", fixtures::publishers::acme()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let deleted = Publishers.delete(context)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;
    let (status, code) = Publishers
        .show(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(deleted.body().as_ref(), None);
    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn parameters_for_route_resolves_the_id_from_the_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/2")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Articles.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["id"],
    );

    assert_eq!(resolved, HashMap::from([("id", Cow::Borrowed("2"))]));

    Ok(())
}

#[test]
fn parameters_for_route_echoes_the_parameters_the_request_carries() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_scoped_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/acme-press/articles/2")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Articles.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["id", "tenant"],
    );

    assert_eq!(
        resolved,
        HashMap::from([
            ("id", Cow::Borrowed("2")),
            ("tenant", Cow::Borrowed("acme-press"))
        ])
    );

    Ok(())
}

#[test]
fn parameters_for_route_omits_a_parameter_it_cannot_resolve() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Articles.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["id", "tenant"],
    );

    assert_eq!(resolved, HashMap::from([("id", Cow::Borrowed("1"))]));

    Ok(())
}

#[test]
fn parameters_for_route_resolves_what_a_controller_overrides() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .header("tenant", "acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Tenanted.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["tenant"],
    );

    assert_eq!(
        resolved,
        HashMap::from([("tenant", Cow::Borrowed("acme-press"))])
    );

    Ok(())
}

#[test]
fn a_controller_refuses_client_generated_ids_unless_it_says_otherwise() -> Result {
    assert!(!Articles.configuration().accepts_client_ids);
    assert!(Publishers.configuration().accepts_client_ids);

    Ok(())
}

#[test]
fn show_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/articles/1?{query}"))
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

            Ok(Articles
                .show(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn related_of_a_to_one_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/comments/1/article?{query}"))
                .body(stream)?;
            let uri: Uri = request.uri().clone().into();
            let context = routing::build_resource_context(
                &manager,
                &router,
                &base,
                &uri,
                request,
                manager.registry().schema("comments")?,
            )?;

            Ok(Comments
                .related(context, "article")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn linkage_of_a_to_one_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/articles/1/relationships/author?{query}"))
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

            Ok(Articles
                .linkage(context, "author")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn linkage_of_a_to_many_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/articles/1/relationships/comments?{query}"))
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

            Ok(Articles
                .linkage(context, "comments")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn create_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("publishers", fixtures::publishers::acme()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let body =
                json!({ "data": { "type": "articles", "attributes": { "title": "Sorted" } } });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::POST)
                .uri(format!("/articles?{query}"))
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

            Ok(Articles
                .create(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn update_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let body = json!({
                "data": { "type": "articles", "id": "1", "attributes": { "title": "Paged" } }
            });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::PATCH)
                .uri(format!("/articles/1?{query}"))
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

            Ok(Articles
                .update(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn delete_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::DELETE)
                .uri(format!("/articles/1?{query}"))
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

            Ok(Articles
                .delete(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn link_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("comments", fixtures::comments::praise()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::POST)
                .uri(format!("/articles/2/relationships/comments?{query}"))
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

            Ok(Articles
                .link(context, "comments")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn relink_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let body = json!({ "data": { "type": "authors", "id": "2" } });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::PATCH)
                .uri(format!("/articles/1/relationships/author?{query}"))
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

            Ok(Articles
                .relink(context, "author")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn unlink_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::DELETE)
                .uri(format!("/articles/1/relationships/comments?{query}"))
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

            Ok(Articles
                .unlink(context, "comments")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}
