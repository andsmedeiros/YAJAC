use super::{
    Context, DefaultUriGenerator, Error, Request, Result, RouteParameters,
    builders::{PrimaryRouteBuilder, RouteBuilder},
    controller::{DefaultController, ResourceController},
    default_response, respond_with,
};
use crate::{
    core::factories::to_document,
    database::{adapters::Adapter as AdapterInterface, connection_manager::ConnectionManager},
    http_wrappers::{StatusCode, Uri},
    json_api::{document::Document, error::Error as JsonApiError},
};
use http::{Method, Response};
use indexmap::IndexMap;
use log::{debug, error};
use std::borrow::Cow;
use std::fmt::{self, Display};
use std::result::Result as StdResult;

/// A schema-oblivious request handler: the leaf every mounted route resolves to.
pub trait Handler<'sch, Adapter: AdapterInterface>:
    for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch
{
}

impl<'sch, T, Adapter: AdapterInterface> Handler<'sch, Adapter> for T where
    T: for<'req> Fn(Context<'sch, 'req, Adapter>) -> Result + Sync + Send + 'sch
{
}

/// A single mounted route: the method and path template it answers, and the handler it runs.
/// Template segments prefixed `:` are dynamic and captured into `RouteParameters` on a match.
pub(crate) struct Route<'sch, Adapter: AdapterInterface> {
    method: Method,
    path: Vec<Cow<'sch, str>>,
    handler: Box<dyn Handler<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> Route<'sch, Adapter> {
    pub(crate) fn new(
        method: Method,
        path: Vec<Cow<'sch, str>>,
        handler: impl Handler<'sch, Adapter>,
    ) -> Self {
        Route {
            method,
            path,
            handler: Box::new(handler),
        }
    }

    fn matches(&self, method: &Method, path_segments: &[&str]) -> Option<RouteParameters> {
        if self.method != method || self.path.len() != path_segments.len() {
            return None;
        }

        let mut params = RouteParameters::new();
        for (segment, &path_segment) in self.path.iter().zip(path_segments) {
            if let Some(param_name) = segment.strip_prefix(':') {
                params.insert(param_name, path_segment);
            } else if segment.as_ref() != path_segment {
                return None;
            }
        }
        Some(params)
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

/// A fault detected while assembling a router: a misconfiguration in how resources and their
/// relationship endpoints were mounted.
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
        }
    }
}

impl std::error::Error for RouterError {}

/// Builds a fresh, type-erased controller instance for one resource kind.
pub(crate) type ControllerFactory<'sch, Adapter> =
    fn() -> Box<dyn ResourceController<'sch, Adapter> + 'sch>;

/// The two canonical link templates of one mounted relationship, each present only when its
/// endpoint was mounted. The segments mirror the mounted route's path, so a link rendered from a
/// template always lands on a real route.
#[derive(Default)]
pub(crate) struct RelationshipMounts<'sch> {
    pub linkage: Option<Vec<Cow<'sch, str>>>,
    pub related: Option<Vec<Cow<'sch, str>>>,
}

/// The canonical mount of one resource: its kind, its controller factory, and the path templates
/// its links are rendered from — the `base` (collection) prefix, from which the resource path is
/// `base` + `:id`, and each mounted relationship.
pub(crate) struct ResourceMount<'sch, Adapter: AdapterInterface> {
    pub kind: &'sch str,
    pub factory: ControllerFactory<'sch, Adapter>,
    pub base: Vec<Cow<'sch, str>>,
    pub relationships: IndexMap<&'sch str, RelationshipMounts<'sch>>,
}

/// Resolves a resource kind to its mount: the controller factory and the link templates the router
/// captured for it. A kind with no mounted resource resolves to `DefaultController` and no links.
pub struct MountTable<'sch, Adapter: AdapterInterface> {
    mounts: IndexMap<&'sch str, ResourceMount<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> MountTable<'sch, Adapter> {
    /// The controller serving `kind`, or `DefaultController` when `kind` is unmounted.
    pub fn resolve(&self, kind: &str) -> Box<dyn ResourceController<'sch, Adapter> + 'sch> {
        self.mounts.get(kind).map_or_else(
            || Box::new(DefaultController) as Box<dyn ResourceController<'sch, Adapter>>,
            |mount| (mount.factory)(),
        )
    }

    /// The mount for `kind`, absent when `kind` is unmounted — the source of its link templates.
    pub(crate) fn mount(&self, kind: &str) -> Option<&ResourceMount<'sch, Adapter>> {
        self.mounts.get(kind)
    }
}

impl<'sch, Adapter: AdapterInterface> Default for MountTable<'sch, Adapter> {
    fn default() -> Self {
        Self {
            mounts: IndexMap::new(),
        }
    }
}

impl<'sch, Adapter: AdapterInterface> FromIterator<ResourceMount<'sch, Adapter>>
    for MountTable<'sch, Adapter>
{
    /// Collects complete mounts into a table by kind, last write winning. Unlike the router's own
    /// assembly it does not reject duplicates — a hand-built table (in tests) owns that.
    fn from_iter<I: IntoIterator<Item = ResourceMount<'sch, Adapter>>>(iter: I) -> Self {
        Self {
            mounts: iter.into_iter().map(|mount| (mount.kind, mount)).collect(),
        }
    }
}

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

    pub(crate) fn push_route(&mut self, route: StdResult<Route<'sch, Adapter>, RouterError>) {
        self.routes.push(route);
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

        Ok((routes, MountTable { mounts }))
    }
}

/// A schema-aware router: the mounted routes it dispatches to, and the controller lookup its
/// handlers resolve related resources through.
pub struct Router<'sch, Adapter: AdapterInterface> {
    routes: Vec<Route<'sch, Adapter>>,
    mount_table: MountTable<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> Router<'sch, Adapter> {
    /// Assembles a router: runs `configure` against a root builder, then validates the eagerly
    /// built routes and their canonical mounts.
    pub fn try_new(
        configure: impl FnOnce(PrimaryRouteBuilder<'sch, Adapter>) -> PrimaryRouteBuilder<'sch, Adapter>,
    ) -> StdResult<Self, RouterError> {
        let (routes, mount_table) = configure(PrimaryRouteBuilder::root()).into_routes().resolve()?;
        Ok(Router { routes, mount_table })
    }

    /// The router's mount table — the per-resource controller factories and link templates it
    /// captured while building.
    pub(crate) fn mount_table(&self) -> &MountTable<'sch, Adapter> {
        &self.mount_table
    }

    pub fn handle(
        &self,
        database: &'sch ConnectionManager<'sch, Adapter>,
        request: http::Request<Vec<u8>>,
    ) -> Response<Option<Document>> {
        let uri: Uri = request.uri().clone().into();
        let method = request.method().clone();
        let path_segments: Vec<&str> = uri.path().split('/').filter(|s| !s.is_empty()).collect();

        self.routes
            .iter()
            .find_map(|route| {
                route
                    .matches(&method, &path_segments)
                    .map(|parameters| (route, parameters))
            })
            .map(|(route, parameters)| {
                debug!("Matched {method} {uri}");
                let (parts, body) = request.into_parts();
                let request = Request::from_parts(parts, serde_json::from_slice(&body)?);
                let context = Context::from_request(database, &uri, parameters, request)
                    .with_mount_table(&self.mount_table);
                (route.handler)(context)
            })
            .unwrap_or_else(|| {
                Err(Error::new(
                    StatusCode::NOT_FOUND,
                    "ResourceNotFound",
                    format!("{method} {uri}: Resource not found"),
                ))
            })
            .or_else(|error| {
                let status = error.status_code();

                let error = if status.is_server_error() {
                    error!("{method} {uri} failed: {error:?}");

                    if cfg!(debug_assertions) {
                        error
                    } else {
                        Error::new(status.clone(), "InternalServerError", "Internal server error")
                    }
                } else {
                    error
                };

                let document = to_document(
                    vec![JsonApiError::from(error)],
                    Vec::new(),
                    &uri,
                    &DefaultUriGenerator::default(),
                )?;

                respond_with(status.into(), Some(document))
            })
            .unwrap_or_else(|error| {
                error!("Failed to construct error response: {error:?}");
                default_response()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(None)
                    .unwrap_or_else(|_| Response::new(None))
            })
    }
}
