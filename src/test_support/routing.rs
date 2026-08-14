//! The controllers a mount needs, and request contexts built from a router and a request.
//!
//! One controller per resource of the shared schema set, each a bare implementation serving the
//! framework's own behaviour: what a suite reaches for when the controller is beneath the unit it is
//! testing. A suite whose subject *is* a controller declares its own.
//!
//! Each context builder resolves the request against the router's mounted routes and yields the
//! context that router would dispatch with: the route parameters captured by the matching template,
//! and that router's link mounts.

use super::Result;
use super::database::ConnectionManager;
use crate::database::adapters::SqliteAdapter;
use crate::database::schema::Schema;
use crate::http_wrappers::Uri;
use crate::routing::context::ResourceContext;
use crate::routing::controller::{Configuration, ResourceController};
use crate::routing::{BaseUri, PrimaryContext, PrimaryRequest};

/// The crate's router, bound to the adapter every suite runs against, so a suite mounting one
/// carries no adapter turbofish.
pub(crate) type Router<'sch> = crate::routing::Router<'sch, SqliteAdapter>;

#[derive(Default)]
pub(crate) struct Authors;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Authors {}

/// `publishers` is keyed by a text primary key with no server-side source, so the only id it can be
/// created with is the client's.
#[derive(Default)]
pub(crate) struct Publishers;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Publishers {
    fn configuration(&self) -> Configuration {
        Configuration {
            accepts_client_ids: true,
        }
    }
}

#[derive(Default)]
pub(crate) struct Articles;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {}

#[derive(Default)]
pub(crate) struct Comments;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Comments {}

#[derive(Default)]
pub(crate) struct Profiles;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Profiles {}

/// The raw byte tier's context for `request`, carrying the parameters captured by the first mounted
/// route whose template matches the request path. Matching considers the path and method; it errors
/// when no route matches.
pub(crate) fn build_primary_context<'sch, 'req>(
    connection_manager: &'sch ConnectionManager<'sch>,
    router: &'req Router<'sch>,
    base_uri: &'req BaseUri<'sch>,
    uri: &'req Uri,
    request: PrimaryRequest,
) -> Result<PrimaryContext<'sch, 'req, SqliteAdapter>> {
    let method = request.method().clone();
    let segments: Vec<&str> = uri.path().split('/').filter(|s| !s.is_empty()).collect();
    let route = router
        .routes
        .iter()
        .find_map(|route| route.match_path(&method, &segments))
        .ok_or_else(|| format!("no mounted route matches {method} {uri}"))?;

    Ok(PrimaryContext::from_request(
        connection_manager,
        base_uri,
        &router.mount_table,
        uri,
        route,
        request,
    ))
}

/// The same context, bound to `schema`.
pub(crate) fn build_resource_context<'sch, 'req>(
    connection_manager: &'sch ConnectionManager<'sch>,
    router: &'req Router<'sch>,
    base_uri: &'req BaseUri<'sch>,
    uri: &'req Uri,
    request: PrimaryRequest,
    schema: &'sch Schema<'sch>,
) -> Result<ResourceContext<'sch, 'req, SqliteAdapter>> {
    Ok(ResourceContext::new(
        schema,
        build_primary_context(connection_manager, router, base_uri, uri, request)?,
    ))
}
