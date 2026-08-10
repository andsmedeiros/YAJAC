use super::{
    BaseUri, Error, PrimaryContext, PrimaryRequest, PrimaryResult, ResourceResult, RouteParameters,
    builders::{PrimaryRouteBuilder, RouteBuilder},
    controller::ResourceContext,
    middleware::Middleware,
    respond_with,
};
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, connection_manager::ConnectionManager,
        schema::Schema,
    },
    http_wrappers::{StatusCode, Uri},
    routing::mount_table::{MountTable, ResourceMount},
    serialisation::ByteStream,
};
use http::{HeaderMap, Method, Response};
use indexmap::IndexMap;
use log::debug;
use std::borrow::Cow;
use std::fmt::{self, Display};
use std::io::Cursor;
use std::result::Result as StdResult;

/// A handler that runs without a bound schema: it receives a `PrimaryContext` — the request head and
/// the raw request body as a byte stream — and returns a byte-stream response.
pub trait PrimaryEndpointHandler<'sch, Adapter: AdapterInterface>:
    for<'req> Fn(PrimaryContext<'sch, 'req, Adapter>) -> PrimaryResult + Sync + Send + 'sch
{
}

impl<'sch, T, Adapter: AdapterInterface> PrimaryEndpointHandler<'sch, Adapter> for T where
    T: for<'req> Fn(PrimaryContext<'sch, 'req, Adapter>) -> PrimaryResult + Sync + Send + 'sch
{
}

/// A handler bound to a resource's schema: it receives a `ResourceContext` and returns a `Document`
/// response, which the router serialises to bytes once the resource middleware around it has run.
pub trait ResourceEndpointHandler<'sch, Adapter: AdapterInterface>:
    for<'req> Fn(ResourceContext<'sch, 'req, Adapter>) -> ResourceResult + Sync + Send + 'sch
{
}

impl<'sch, T, Adapter: AdapterInterface> ResourceEndpointHandler<'sch, Adapter> for T where
    T: for<'req> Fn(ResourceContext<'sch, 'req, Adapter>) -> ResourceResult + Sync + Send + 'sch
{
}

/// The handler a route runs, in one of two forms. `Primary` serves raw bytes and its errors escape
/// to the embedder; `Resource` is bound to a schema and returns a `Document` the router serialises,
/// after building its `ResourceContext`. A route stores exactly one.
///
/// Part of the builder seam threaded through the public `RouteBuilder::mount`, hence `pub` — but an
/// internal type users never name (verbs construct it for them), so `doc(hidden)`.
#[doc(hidden)]
pub enum EndpointHandler<'sch, Adapter: AdapterInterface> {
    Primary(Box<dyn PrimaryEndpointHandler<'sch, Adapter>>),
    Resource {
        schema: &'sch Schema<'sch>,
        handler: Box<dyn ResourceEndpointHandler<'sch, Adapter>>,
    },
}

impl<'sch, Adapter: AdapterInterface + 'sch> EndpointHandler<'sch, Adapter> {
    /// A schema-less handler. The `impl` bound drives closure inference, so callers pass a bare
    /// closure rather than naming the boxed handler type.
    pub(crate) fn primary(handler: impl PrimaryEndpointHandler<'sch, Adapter>) -> Self {
        EndpointHandler::Primary(Box::new(handler))
    }

    /// A schema-bound handler, paired with the schema its `ResourceContext` is built against.
    pub(crate) fn resource(
        schema: &'sch Schema<'sch>,
        handler: impl ResourceEndpointHandler<'sch, Adapter>,
    ) -> Self {
        EndpointHandler::Resource {
            schema,
            handler: Box::new(handler),
        }
    }
}

/// A single mounted route: the method and path template it answers, the middleware wrapping it, and
/// the handler it runs. The middleware is ordered schema-less first, then schema-bound. Template
/// segments prefixed `:` are dynamic and captured into `RouteParameters` on a match.
pub(crate) struct Route<'sch, Adapter: AdapterInterface> {
    method: Method,
    path: Vec<Cow<'sch, str>>,
    middleware: Vec<Middleware<'sch, Adapter>>,
    handler: EndpointHandler<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> Route<'sch, Adapter> {
    pub(crate) fn new(
        method: Method,
        path: Vec<Cow<'sch, str>>,
        middleware: Vec<Middleware<'sch, Adapter>>,
        handler: EndpointHandler<'sch, Adapter>,
    ) -> Self {
        Route {
            method,
            path,
            middleware,
            handler,
        }
    }

    /// Matches the request line against this route's method and path template, capturing dynamic
    /// segments. Does not consult middleware.
    ///
    /// A trailing `*name` template segment is a glob: it matches one-or-more remaining path segments,
    /// captured joined under `name` (a bare `*` matches without capturing). Every other segment
    /// matches exactly one path segment — literally, or capturing a `:name` dynamic segment.
    fn match_path(&self, method: &Method, path_segments: &[&str]) -> Option<RouteParameters> {
        if self.method != method {
            return None;
        }

        let glob = self
            .path
            .last()
            .and_then(|segment| segment.strip_prefix('*'));

        if (glob.is_none() && path_segments.len() != self.path.len())
            || path_segments.len() < self.path.len()
        {
            return None;
        }

        let partition = self.path.len() - glob.is_some() as usize;

        let mut params = RouteParameters::new();
        for (segment, &path_segment) in self.path[..partition].iter().zip(path_segments) {
            if let Some(param_name) = segment.strip_prefix(':') {
                params.insert(param_name, path_segment);
            } else if segment != path_segment {
                return None;
            }
        }

        if let Some(name) = glob
            && !name.is_empty()
        {
            params.insert(name, path_segments[partition..].join("/"));
        }

        Some(params)
    }

    /// Matches the request line, then every middleware guard against the request head and the
    /// captured parameters. A guard that rejects the request makes the route not match — it falls
    /// through to the next route, or to a 404.
    fn matches(
        &self,
        method: &Method,
        path_segments: &[&str],
        uri: &Uri,
        headers: &HeaderMap,
    ) -> Option<RouteParameters> {
        let params = self.match_path(method, path_segments)?;
        self.middleware
            .iter()
            .all(|middleware| middleware.matches(headers, uri, &params))
            .then_some(params)
    }
}

/// Splits a `/`-delimited path fragment into its template segments, dropping empties. Segments
/// borrow the fragment, which — being computed at router-definition time — lives for `'sch`.
pub(crate) fn split_segments<'sch>(segment: &'sch str) -> impl Iterator<Item = Cow<'sch, str>> {
    segment
        .split('/')
        .filter(|s| !s.is_empty())
        .map(Cow::Borrowed)
}

/// The two per-relationship canonical endpoints, each independently mountable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MountSlot {
    Linkage,
    Related,
}

impl Display for MountSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A fault detected while assembling a router: a misconfiguration in how resources, their
/// relationship endpoints, or their middleware were mounted.
#[derive(Debug)]
pub enum RouterError {
    DuplicateResource {
        kind: String,
    },
    DuplicateRelationshipSlot {
        kind: String,
        relationship: String,
        slot: MountSlot,
    },
    UnknownRelationship {
        kind: String,
        relationship: String,
    },
    /// A schema-bound middleware wraps a raw route. A raw route serves no `Document`, so the
    /// middleware could never run against a `ResourceContext` — the router would silently ignore it.
    ResourceMiddlewareOnPrimaryRoute {
        path: String,
    },
    /// A `*name` glob segment appears anywhere but the end of a path template. A glob consumes the
    /// rest of the path, so any segment after it could never match.
    MisplacedGlob {
        path: String,
    },
    /// A path template captures the same parameter name in more than one segment. Both would resolve
    /// into the same key, silently overriding one another.
    DuplicateParameter {
        path: String,
        parameter: String,
    },
}

impl Display for RouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouterError::DuplicateResource { kind } => {
                write!(f, "the resource '{kind}' is mounted more than once")
            }
            RouterError::DuplicateRelationshipSlot {
                kind,
                relationship,
                slot,
            } => write!(
                f,
                "the {slot} endpoint of '{kind}.{relationship}' is mounted more than once"
            ),
            RouterError::UnknownRelationship { kind, relationship } => {
                write!(f, "'{kind}' has no relationship named '{relationship}'")
            }
            RouterError::ResourceMiddlewareOnPrimaryRoute { path } => write!(
                f,
                "a schema-bound middleware wraps the raw route '/{path}', which serves no JSON:API document"
            ),
            RouterError::MisplacedGlob { path } => {
                write!(
                    f,
                    "route '/{path}' contains a wildcard segment in an invalid position"
                )
            }
            RouterError::DuplicateParameter { path, parameter } => {
                write!(
                    f,
                    "route '/{path}' contains the named capture '{parameter}' multiple times"
                )
            }
        }
    }
}

impl std::error::Error for RouterError {}

/// The accumulating product of a builder: eagerly-built routes, each carrying its build outcome,
/// and the canonical mounts to validate once building completes. Opaque — the builder seam threads
/// it, but nothing outside the crate can read or write it.
#[doc(hidden)]
pub struct MaterialisedRoutes<'sch, Adapter: AdapterInterface> {
    routes: Vec<StdResult<Route<'sch, Adapter>, RouterError>>,
    mounts: Vec<ResourceMount<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> MaterialisedRoutes<'sch, Adapter> {
    pub(crate) fn new() -> Self {
        Self {
            routes: Vec::new(),
            mounts: Vec::new(),
        }
    }

    pub(crate) fn push_route(&mut self, route: Route<'sch, Adapter>) {
        self.routes.push(Ok(route));
    }

    pub(crate) fn push_error(&mut self, error: RouterError) {
        self.routes.push(Err(error));
    }

    pub(crate) fn push_mount(&mut self, mount: ResourceMount<'sch, Adapter>) {
        self.mounts.push(mount);
    }

    pub(crate) fn absorb(&mut self, other: MaterialisedRoutes<'sch, Adapter>) {
        self.routes.extend(other.routes);
        self.mounts.extend(other.mounts);
    }

    /// Unwraps every route (surfacing the first build fault), then folds the mounts by kind
    /// (surfacing a duplicate resource) into the finished routes and controller lookup.
    fn resolve(
        self,
    ) -> StdResult<(Vec<Route<'sch, Adapter>>, MountTable<'sch, Adapter>), RouterError> {
        let routes = self.routes.into_iter().collect::<StdResult<Vec<_>, _>>()?;

        let mut mounts: IndexMap<&'sch str, ResourceMount<'sch, Adapter>> = IndexMap::new();
        for mount in self.mounts {
            let kind = mount.kind;
            if mounts.insert(kind, mount).is_some() {
                return Err(RouterError::DuplicateResource {
                    kind: kind.to_string(),
                });
            }
        }

        Ok((routes, MountTable::new(mounts)))
    }
}

/// Runs a matched route. Each leading schema-less middleware runs against the `PrimaryContext`,
/// wrapping the recursive call that runs the rest. Once they are exhausted, a `Primary` handler runs
/// directly; a `Resource` handler is run against a `ResourceContext` built here, and the `Document`
/// it returns is serialised into a byte-stream response.
fn serve<'sch, 'req, Adapter>(
    middleware: &'req [Middleware<'sch, Adapter>],
    handler: &'req EndpointHandler<'sch, Adapter>,
    context: PrimaryContext<'sch, 'req, Adapter>,
) -> PrimaryResult
where
    'sch: 'req,
    Adapter: AdapterInterface + 'sch,
{
    match (middleware.split_first(), handler) {
        (Some((Middleware::Primary(primary), rest)), _) => {
            primary.handle(context, &|context| serve(rest, handler, context))
        }
        (_, EndpointHandler::Primary(handler)) => handler(context),
        (_, EndpointHandler::Resource { schema, handler }) => {
            let response = serve_resource(
                middleware,
                &**handler,
                ResourceContext::new(schema, context),
            )?;
            let (parts, body) = response.into_parts();
            let body = body
                .map(|document| serde_json::to_vec(&document))
                .transpose()?
                .map(|bytes| Box::new(Cursor::new(bytes)) as ByteStream);

            Ok(Response::from_parts(parts, body))
        }
    }
}

/// Runs the schema-bound middleware wrapping a resource handler — each wraps the recursive call that
/// runs the rest — then the handler. Any schema-less middleware left in the slice has already run
/// against the `PrimaryContext`, so it is passed over.
fn serve_resource<'sch, 'req, Adapter>(
    middleware: &'req [Middleware<'sch, Adapter>],
    handler: &'req dyn ResourceEndpointHandler<'sch, Adapter>,
    context: ResourceContext<'sch, 'req, Adapter>,
) -> ResourceResult
where
    'sch: 'req,
    Adapter: AdapterInterface + 'sch,
{
    match middleware.split_first() {
        Some((Middleware::Resource(resource), rest)) => {
            resource.handle(context, &|context| serve_resource(rest, handler, context))
        }
        // The chain is partitioned schema-less-first, so a schema-less middleware here means the
        // partition was built wrong — a router-assembly bug, surfaced rather than passed over.
        Some((Middleware::Primary(_), _)) => Err(Error::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MisorderedMiddleware",
            "A schema-less middleware appears within the schema-bound middleware chain",
        )),
        None => handler(context),
    }
}

/// A schema-aware router: the base its links are rooted at, the mounted routes it dispatches to,
/// and the mount table its handlers resolve controllers and link templates through.
pub struct Router<'sch, Adapter: AdapterInterface> {
    base_uri: BaseUri<'sch>,
    routes: Vec<Route<'sch, Adapter>>,
    pub(crate) mount_table: MountTable<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> Router<'sch, Adapter> {
    /// Assembles a router: runs `configure` against a root builder, then validates the eagerly
    /// built routes and their canonical mounts. `base_uri` roots every link the router mints.
    pub fn try_new(
        base_uri: BaseUri<'sch>,
        configure: impl FnOnce(PrimaryRouteBuilder<'sch, Adapter>) -> PrimaryRouteBuilder<'sch, Adapter>,
    ) -> StdResult<Self, RouterError> {
        let (routes, mount_table) = configure(PrimaryRouteBuilder::root())
            .into_routes()
            .resolve()?;
        Ok(Router {
            base_uri,
            routes,
            mount_table,
        })
    }

    /// Dispatches a request. An unmatched route yields a bare bodyless 404; a matched one runs its
    /// middleware and handler. The result is fallible to the embedder: expected JSON:API errors are
    /// rendered into documents by the resource middleware, so an `Err` here is exceptional.
    pub fn handle(
        &self,
        database: &'sch ConnectionManager<'sch, Adapter>,
        request: PrimaryRequest,
    ) -> PrimaryResult {
        let uri: Uri = request.uri().clone().into();
        let method = request.method().clone();
        let path_segments: Vec<&str> = uri.path().split('/').filter(|s| !s.is_empty()).collect();

        self.routes
            .iter()
            .find_map(|route| {
                route
                    .matches(&method, &path_segments, &uri, request.headers())
                    .map(|parameters| (route, parameters))
            })
            .map(|(route, parameters)| {
                debug!("Matched {method} {uri}");
                let context = PrimaryContext::from_request(
                    database,
                    &self.base_uri,
                    &self.mount_table,
                    &uri,
                    parameters,
                    request,
                );
                serve(&route.middleware, &route.handler, context)
            })
            .unwrap_or_else(|| {
                respond_with(StatusCode::NOT_FOUND, None::<ByteStream>).map_err(Into::into)
            })
    }
}
