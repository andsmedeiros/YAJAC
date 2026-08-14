mod controller;
mod query_parameters;
mod reading;
mod relationship_writes;
mod relationships;
mod writing;

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
