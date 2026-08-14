//! Request contexts, built from a router and a request.
//!
//! Each builder resolves the request against the router's mounted routes and yields the context that
//! router would dispatch with: the route parameters captured by the matching template, and that
//! router's link mounts.

use super::Result;
use crate::database::adapters::SqliteAdapter;
use crate::database::connection_manager::ConnectionManager;
use crate::database::schema::Schema;
use crate::http_wrappers::Uri;
use crate::routing::context::ResourceContext;
use crate::routing::{BaseUri, PrimaryContext, PrimaryRequest, Router};

type Manager<'sch> = ConnectionManager<'sch, SqliteAdapter>;

/// The raw byte tier's context for `request`, carrying the parameters captured by the first mounted
/// route whose template matches the request path. Matching considers the path and method; it errors
/// when no route matches.
pub(crate) fn build_primary_context<'sch, 'req>(
    manager: &'sch Manager<'sch>,
    router: &'req Router<'sch, SqliteAdapter>,
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
        manager,
        base_uri,
        &router.mount_table,
        uri,
        route,
        request,
    ))
}

/// The same context, bound to `schema`.
pub(crate) fn build_resource_context<'sch, 'req>(
    manager: &'sch Manager<'sch>,
    router: &'req Router<'sch, SqliteAdapter>,
    base_uri: &'req BaseUri<'sch>,
    uri: &'req Uri,
    request: PrimaryRequest,
    schema: &'sch Schema<'sch>,
) -> Result<ResourceContext<'sch, 'req, SqliteAdapter>> {
    Ok(ResourceContext::new(
        schema,
        build_primary_context(manager, router, base_uri, uri, request)?,
    ))
}
