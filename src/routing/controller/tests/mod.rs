mod controller;
mod query_parameters;
mod reading;
mod relationship_writes;
mod relationships;
mod writing;

use super::ResourceController;
use crate::database::adapters::SqliteAdapter;
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
use crate::test_support::database::ConnectionManager;
use crate::test_support::routing::{Articles, Authors, Comments, Profiles, Publishers};
use crate::test_support::{Result, database, fixtures, routing};
use http::{HeaderMap, Method, Response};
use serde_json::json;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, empty};

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
    connection_manager: &'sch ConnectionManager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let registry = connection_manager.registry();
    let articles = registry.schema("articles")?;
    let authors = registry.schema("authors")?;
    let publishers = registry.schema("publishers")?;
    let profiles = registry.schema("profiles")?;
    let comments = registry.schema("comments")?;

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
    connection_manager: &'sch ConnectionManager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = connection_manager.registry().schema("articles")?;

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
    connection_manager: &'sch ConnectionManager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = connection_manager.registry().schema("articles")?;

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
