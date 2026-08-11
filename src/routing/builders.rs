use super::{
    Error, ResourceResult,
    controller::{ResourceContext, ResourceController},
    middleware::json_api::JsonApi,
    middleware::{Middleware, PrimaryMiddleware, ResourceMiddleware},
    mount_table::{RelationshipMounts, ResourceMount},
    router::{
        EndpointHandler, MaterialisedRoutes, MountSlot, PrimaryEndpointHandler,
        ResourceEndpointHandler, Route, RouterError, split_segments,
    },
};
use crate::database::{
    adapters::Adapter as AdapterInterface,
    schema::{RelationshipKind, Schema},
};
use crate::http_wrappers::StatusCode;
use http::Method;
use indexmap::IndexMap;
use itertools::Itertools;
use std::borrow::Cow;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::Arc;

/// The seam every route builder shares: its mount path, the middleware active at its level, its
/// route accumulator, and how it spawns a sibling of itself for a nested scope.
pub trait RouteBuilder<'sch, Adapter: AdapterInterface + 'sch>: Sized {
    fn path(&self) -> &[Cow<'sch, str>];
    fn middleware(&self) -> &[Middleware<'sch, Adapter>];
    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter>;
    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter>;
    /// Constructs a sibling of the same builder kind, extending this level's path and middleware
    /// with `segments` and `middleware` respectively (either may be empty).
    fn spawn(
        &self,
        segments: impl IntoIterator<Item = Cow<'sch, str>>,
        middleware: impl IntoIterator<Item = Middleware<'sch, Adapter>>,
    ) -> Self;

    /// Spawns a sibling one path segment deeper, carrying this level's middleware unchanged — the
    /// primitive behind `scope`.
    fn spawn_with_path(&self, segment: impl Into<Cow<'sch, str>>) -> Self {
        self.spawn(split_segments(segment), [])
    }

    /// Spawns a sibling at the same path, carrying this level's middleware plus `middleware` — the
    /// primitive behind `middleware`.
    fn spawn_with_middleware(&self, middleware: Middleware<'sch, Adapter>) -> Self {
        self.spawn([], [middleware])
    }

    /// Mounts `handler` for `method` at `segments` relative to this builder's path, stamping the
    /// active middleware onto the route. The middleware is already tier-ordered by construction —
    /// every builder is single-tier — so no reordering happens here.
    ///
    /// A raw handler carrying a schema-bound middleware is a fault, recorded in place of the route:
    /// the dispatcher would reach the handler without crossing into the JSON:API tier, silently
    /// dropping the middleware. No single-tier builder can express this today, but the check outlives
    /// that guarantee.
    fn mount(
        &mut self,
        method: Method,
        segments: impl IntoIterator<Item = Cow<'sch, str>>,
        handler: EndpointHandler<'sch, Adapter>,
    ) {
        let path: Vec<Cow<'sch, str>> = self.path().iter().cloned().chain(segments).collect();
        let middleware = self.middleware().to_vec();

        if path.iter().any(|segment| {
            segment
                .strip_prefix('*')
                .is_some_and(|name| !name.is_empty())
        }) {
            self.routes_mut().push_error(RouterError::NamedGlob {
                path: path.iter().join("/"),
            });
            return;
        }

        if path
            .iter()
            .rev()
            .skip(1)
            .any(|segment| segment.starts_with('*'))
        {
            self.routes_mut().push_error(RouterError::MisplacedGlob {
                path: path.iter().join("/"),
            });
            return;
        }

        let mut captures = HashSet::new();
        for name in path
            .iter()
            .filter_map(|segment| segment.strip_prefix(':'))
            .filter(|name| !name.is_empty())
        {
            if !captures.insert(name) {
                self.routes_mut()
                    .push_error(RouterError::DuplicateParameter {
                        path: path.iter().join("/"),
                        parameter: name.to_string(),
                    });
                return;
            }
        }

        if let EndpointHandler::Primary(_) = handler
            && middleware.iter().any(Middleware::is_resource)
        {
            self.routes_mut()
                .push_error(RouterError::ResourceMiddlewareOnPrimaryRoute {
                    path: path.iter().join("/"),
                });
            return;
        }

        self.routes_mut()
            .push_route(Route::new(method, path, middleware, handler));
    }

    /// Opens a nested scope at `segment`: builds a sibling inheriting this level's middleware, runs
    /// `configure`, merges it back.
    fn scope(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        configure: impl FnOnce(Self) -> Self,
    ) -> Self {
        let child = configure(self.spawn_with_path(segment));
        self.routes_mut().absorb(child.into_routes());
        self
    }
}

/// The verb surface whose handlers receive a bare `PrimaryContext` — the raw byte tier. Mounted only
/// on the primary builder.
pub trait UnboundVerbs<'sch, Adapter: AdapterInterface + 'sch>:
    RouteBuilder<'sch, Adapter>
{
    fn get(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl PrimaryEndpointHandler<'sch, Adapter>,
    ) -> Self {
        self.mount(
            Method::GET,
            split_segments(segment),
            EndpointHandler::primary(handler),
        );
        self
    }

    fn post(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl PrimaryEndpointHandler<'sch, Adapter>,
    ) -> Self {
        self.mount(
            Method::POST,
            split_segments(segment),
            EndpointHandler::primary(handler),
        );
        self
    }

    fn put(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl PrimaryEndpointHandler<'sch, Adapter>,
    ) -> Self {
        self.mount(
            Method::PUT,
            split_segments(segment),
            EndpointHandler::primary(handler),
        );
        self
    }

    fn patch(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl PrimaryEndpointHandler<'sch, Adapter>,
    ) -> Self {
        self.mount(
            Method::PATCH,
            split_segments(segment),
            EndpointHandler::primary(handler),
        );
        self
    }

    fn delete(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl PrimaryEndpointHandler<'sch, Adapter>,
    ) -> Self {
        self.mount(
            Method::DELETE,
            split_segments(segment),
            EndpointHandler::primary(handler),
        );
        self
    }
}

/// The verb surface whose handlers receive a `ResourceContext` bound to the builder's schema — the
/// JSON:API tier. Shared by the resource builder (collection-scoped routes) and the member builder
/// (record-scoped routes).
pub trait ResourceVerbs<'sch, Adapter: AdapterInterface + 'sch>:
    RouteBuilder<'sch, Adapter>
{
    /// The schema every handler mounted here is bound to.
    fn schema(&self) -> &'sch Schema<'sch>;

    fn get(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl ResourceEndpointHandler<'sch, Adapter>,
    ) -> Self {
        let schema = self.schema();
        self.mount(
            Method::GET,
            split_segments(segment),
            EndpointHandler::resource(schema, handler),
        );
        self
    }

    fn post(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl ResourceEndpointHandler<'sch, Adapter>,
    ) -> Self {
        let schema = self.schema();
        self.mount(
            Method::POST,
            split_segments(segment),
            EndpointHandler::resource(schema, handler),
        );
        self
    }

    fn put(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl ResourceEndpointHandler<'sch, Adapter>,
    ) -> Self {
        let schema = self.schema();
        self.mount(
            Method::PUT,
            split_segments(segment),
            EndpointHandler::resource(schema, handler),
        );
        self
    }

    fn patch(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl ResourceEndpointHandler<'sch, Adapter>,
    ) -> Self {
        let schema = self.schema();
        self.mount(
            Method::PATCH,
            split_segments(segment),
            EndpointHandler::resource(schema, handler),
        );
        self
    }

    fn delete(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        handler: impl ResourceEndpointHandler<'sch, Adapter>,
    ) -> Self {
        let schema = self.schema();
        self.mount(
            Method::DELETE,
            split_segments(segment),
            EndpointHandler::resource(schema, handler),
        );
        self
    }
}

/// The refusal a read-only mount serves for an unsupported write. Schema-bound so the refusal crosses
/// the JSON:API boundary and is rendered as an error document.
fn forbidden<'sch, 'req, Adapter: AdapterInterface>(
    _context: ResourceContext<'sch, 'req, Adapter>,
) -> ResourceResult {
    Err(Error::new(
        StatusCode::FORBIDDEN,
        "Forbidden",
        "This endpoint does not support the requested operation",
    ))
}

/// The top-level builder: mounts raw routes and resources at the router root, and yields the same
/// builder for every nested scope. The raw byte tier — its middleware fronts requests as bytes.
pub struct PrimaryRouteBuilder<'sch, Adapter: AdapterInterface> {
    path: Vec<Cow<'sch, str>>,
    middleware: Vec<Middleware<'sch, Adapter>>,
    routes: MaterialisedRoutes<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> RouteBuilder<'sch, Adapter>
    for PrimaryRouteBuilder<'sch, Adapter>
{
    fn path(&self) -> &[Cow<'sch, str>] {
        &self.path
    }

    fn middleware(&self) -> &[Middleware<'sch, Adapter>] {
        &self.middleware
    }

    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter> {
        &mut self.routes
    }

    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes
    }

    fn spawn(
        &self,
        segments: impl IntoIterator<Item = Cow<'sch, str>>,
        middleware: impl IntoIterator<Item = Middleware<'sch, Adapter>>,
    ) -> Self {
        Self {
            path: self.path.iter().cloned().chain(segments).collect(),
            middleware: self.middleware.iter().cloned().chain(middleware).collect(),
            routes: MaterialisedRoutes::new(),
        }
    }
}

impl<'sch, Adapter: AdapterInterface + 'sch> UnboundVerbs<'sch, Adapter>
    for PrimaryRouteBuilder<'sch, Adapter>
{
}

impl<'sch, Adapter: AdapterInterface + 'sch> PrimaryRouteBuilder<'sch, Adapter> {
    pub(crate) fn root() -> Self {
        Self {
            path: Vec::new(),
            middleware: Vec::new(),
            routes: MaterialisedRoutes::new(),
        }
    }

    /// Wraps the routes `build` mounts with `middleware`, on the raw byte tier: spawns a sibling
    /// carrying the extended middleware, runs `build`, merges it back — like `scope`, extending the
    /// middleware instead of the path.
    pub fn middleware(
        mut self,
        middleware: impl PrimaryMiddleware<'sch, Adapter>,
        build: impl FnOnce(Self) -> Self,
    ) -> Self {
        let child = build(self.spawn_with_middleware(Middleware::Primary(Arc::new(middleware))));
        self.routes.absorb(child.into_routes());
        self
    }

    /// `scope` and `middleware` in one: opens a nested scope at `segment` under `middleware`.
    pub fn middleware_at(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        middleware: impl PrimaryMiddleware<'sch, Adapter>,
        build: impl FnOnce(Self) -> Self,
    ) -> Self {
        let child = build(self.spawn(
            split_segments(segment),
            [Middleware::Primary(Arc::new(middleware))],
        ));
        self.routes.absorb(child.into_routes());
        self
    }

    pub fn resource<T>(self, segment: impl Into<Cow<'sch, str>>, schema: &'sch Schema<'sch>) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        self.resource_with::<T>(segment, schema, |resource| {
            resource.default_endpoints().all_relationships()
        })
    }

    pub fn resource_with<T>(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        schema: &'sch Schema<'sch>,
        configure: impl FnOnce(
            ResourceRouteBuilder<'sch, T, Adapter>,
        ) -> ResourceRouteBuilder<'sch, T, Adapter>,
    ) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        let path = self
            .path
            .iter()
            .cloned()
            .chain(split_segments(segment))
            .collect();
        let built = configure(ResourceRouteBuilder::new(
            path,
            schema,
            false,
            self.middleware.clone(),
        ))
        .build();
        self.routes.absorb(built);
        self
    }

    pub fn read_only_resource<T>(
        self,
        segment: impl Into<Cow<'sch, str>>,
        schema: &'sch Schema<'sch>,
    ) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        self.read_only_resource_with::<T>(segment, schema, |resource| {
            resource.default_endpoints().all_relationships()
        })
    }

    pub fn read_only_resource_with<T>(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        schema: &'sch Schema<'sch>,
        configure: impl FnOnce(
            ResourceRouteBuilder<'sch, T, Adapter>,
        ) -> ResourceRouteBuilder<'sch, T, Adapter>,
    ) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        let path = self
            .path
            .iter()
            .cloned()
            .chain(split_segments(segment))
            .collect();
        let built = configure(ResourceRouteBuilder::new(
            path,
            schema,
            true,
            self.middleware.clone(),
        ))
        .build();
        self.routes.absorb(built);
        self
    }
}

/// Which of a relationship's two endpoints a family mounts.
#[derive(Clone, Copy)]
struct SlotSet {
    linkage: bool,
    related: bool,
}

impl SlotSet {
    const BOTH: Self = Self {
        linkage: true,
        related: true,
    };
    const LINKAGE: Self = Self {
        linkage: true,
        related: false,
    };
    const RELATED: Self = Self {
        linkage: false,
        related: true,
    };
}

/// The resolved mounting options for a relationship's endpoints: whether writes are refused, and
/// the segment/keyword overriding the path defaults.
#[derive(Clone)]
struct SlotConfig<'sch> {
    read_only: bool,
    segment: Option<Cow<'sch, str>>,
    keyword: Option<Cow<'sch, str>>,
}

/// The per-relationship configuration a `*_with` closure customises: `read_only` refuses writes,
/// `at` relocates the endpoint's leaf segment and self-link keyword.
#[derive(Default)]
pub struct RelationshipConfig<'sch> {
    read_only: bool,
    segment: Option<Cow<'sch, str>>,
    keyword: Option<Cow<'sch, str>>,
}

impl<'sch> RelationshipConfig<'sch> {
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn at(
        mut self,
        segment: impl Into<Cow<'sch, str>>,
        keyword: impl Into<Cow<'sch, str>>,
    ) -> Self {
        self.segment = Some(segment.into());
        self.keyword = Some(keyword.into());
        self
    }

    fn resolve(self, read_only: bool) -> SlotConfig<'sch> {
        SlotConfig {
            read_only: self.read_only || read_only,
            segment: self.segment,
            keyword: self.keyword,
        }
    }
}

/// The configuration a plural `*_with` closure applies to every named relationship: `read_only`
/// refuses writes, `at` relocates the shared self-link keyword.
#[derive(Default)]
pub struct RelationshipsConfig<'sch> {
    read_only: bool,
    keyword: Option<Cow<'sch, str>>,
}

impl<'sch> RelationshipsConfig<'sch> {
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn at(mut self, keyword: impl Into<Cow<'sch, str>>) -> Self {
        self.keyword = Some(keyword.into());
        self
    }

    fn resolve(self, read_only: bool) -> SlotConfig<'sch> {
        SlotConfig {
            read_only: self.read_only || read_only,
            segment: None,
            keyword: self.keyword,
        }
    }
}

/// The `resource[_with]` surface: a resource's CRUD mounts on construction; this builder adds its
/// relationship endpoints, record-scoped `member` routes, and collection-scoped custom routes (its
/// own verbs). Every route it emits is schema-bound — the JSON:API tier.
pub struct ResourceRouteBuilder<'sch, T, Adapter: AdapterInterface> {
    path: Vec<Cow<'sch, str>>,
    schema: &'sch Schema<'sch>,
    read_only: bool,
    middleware: Vec<Middleware<'sch, Adapter>>,
    routes: MaterialisedRoutes<'sch, Adapter>,
    mounted: HashSet<(&'sch str, MountSlot)>,
    relationships: IndexMap<&'sch str, RelationshipMounts<'sch>>,
    controller: PhantomData<fn() -> T>,
}

impl<'sch, T, Adapter: AdapterInterface + 'sch> RouteBuilder<'sch, Adapter>
    for ResourceRouteBuilder<'sch, T, Adapter>
{
    fn path(&self) -> &[Cow<'sch, str>] {
        &self.path
    }

    fn middleware(&self) -> &[Middleware<'sch, Adapter>] {
        &self.middleware
    }

    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter> {
        &mut self.routes
    }

    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes
    }

    fn spawn(
        &self,
        segments: impl IntoIterator<Item = Cow<'sch, str>>,
        middleware: impl IntoIterator<Item = Middleware<'sch, Adapter>>,
    ) -> Self {
        Self {
            path: self.path.iter().cloned().chain(segments).collect(),
            schema: self.schema,
            read_only: self.read_only,
            middleware: self.middleware.iter().cloned().chain(middleware).collect(),
            routes: MaterialisedRoutes::new(),
            mounted: HashSet::new(),
            relationships: IndexMap::new(),
            controller: PhantomData,
        }
    }
}

impl<'sch, T, Adapter: AdapterInterface + 'sch> ResourceVerbs<'sch, Adapter>
    for ResourceRouteBuilder<'sch, T, Adapter>
{
    fn schema(&self) -> &'sch Schema<'sch> {
        self.schema
    }
}

impl<'sch, T, Adapter> ResourceRouteBuilder<'sch, T, Adapter>
where
    T: ResourceController<'sch, Adapter> + Default + 'sch,
    Adapter: AdapterInterface + 'sch,
{
    pub(crate) fn new(
        path: Vec<Cow<'sch, str>>,
        schema: &'sch Schema<'sch>,
        read_only: bool,
        mut middleware: Vec<Middleware<'sch, Adapter>>,
    ) -> Self {
        // The JSON:API boundary is the outermost schema-bound middleware, wrapping every route this
        // builder emits: appended once here, after the inherited raw-tier middleware.
        middleware.push(Middleware::Resource(Arc::new(JsonApi)));
        Self {
            path,
            schema,
            read_only,
            middleware,
            routes: MaterialisedRoutes::new(),
            mounted: HashSet::new(),
            relationships: IndexMap::new(),
            controller: PhantomData,
        }
    }

    /// Wraps the routes `build` mounts with `middleware`, on the JSON:API tier: spawns a sibling
    /// carrying the extended middleware, runs `build`, merges it back. The merge folds the child's
    /// relationship templates and claimed slots home, so a relationship mounted inside a block still
    /// contributes to the mount table.
    pub fn middleware(
        mut self,
        middleware: impl ResourceMiddleware<'sch, Adapter>,
        build: impl FnOnce(Self) -> Self,
    ) -> Self {
        let child = build(self.spawn_with_middleware(Middleware::Resource(Arc::new(middleware))));
        self.absorb_resource(child);
        self
    }

    /// Folds a nested resource builder home: its routes, its claimed relationship slots, and its
    /// captured relationship templates.
    fn absorb_resource(&mut self, child: Self) {
        self.mounted.extend(child.mounted);
        for (name, mounts) in child.relationships {
            let entry = self.relationships.entry(name).or_default();
            entry.linkage = entry.linkage.take().or(mounts.linkage);
            entry.related = entry.related.take().or(mounts.related);
        }
        self.routes.absorb(child.routes);
    }

    /// Finalises the resource: records its canonical mount and yields the accumulated routes.
    pub(crate) fn build(mut self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes.push_mount(ResourceMount {
            kind: self.schema.name(),
            factory: || Box::new(T::default()) as Box<dyn ResourceController<'sch, Adapter>>,
            base: self.path.clone(),
            relationships: self.relationships,
        });
        self.routes
    }

    /// Mounts the resource's default CRUD endpoints, honouring `read_only`. Opt-in: `resource` chains
    /// it in, while `resource_with` leaves the choice to its closure.
    pub fn default_endpoints(mut self) -> Self {
        let schema = self.schema;
        self.mount(
            Method::GET,
            std::iter::empty(),
            EndpointHandler::resource(schema, |context| T::default().index(context)),
        );
        self.mount(
            Method::GET,
            [Cow::Borrowed(":id")],
            EndpointHandler::resource(schema, |context| T::default().show(context)),
        );

        if self.read_only {
            self.mount(
                Method::POST,
                std::iter::empty(),
                EndpointHandler::resource(schema, forbidden),
            );
            self.mount(
                Method::PUT,
                [Cow::Borrowed(":id")],
                EndpointHandler::resource(schema, forbidden),
            );
            self.mount(
                Method::PATCH,
                [Cow::Borrowed(":id")],
                EndpointHandler::resource(schema, forbidden),
            );
            self.mount(
                Method::DELETE,
                [Cow::Borrowed(":id")],
                EndpointHandler::resource(schema, forbidden),
            );
        } else {
            self.mount(
                Method::POST,
                std::iter::empty(),
                EndpointHandler::resource(schema, |context| T::default().create(context)),
            );
            self.mount(
                Method::PUT,
                [Cow::Borrowed(":id")],
                EndpointHandler::resource(schema, |context| T::default().update(context)),
            );
            self.mount(
                Method::PATCH,
                [Cow::Borrowed(":id")],
                EndpointHandler::resource(schema, |context| T::default().update(context)),
            );
            self.mount(
                Method::DELETE,
                [Cow::Borrowed(":id")],
                EndpointHandler::resource(schema, |context| T::default().delete(context)),
            );
        }

        self
    }

    fn default_config(&self) -> SlotConfig<'sch> {
        SlotConfig {
            read_only: self.read_only,
            segment: None,
            keyword: None,
        }
    }

    /// Records that `slot` is now mounted for `relationship`, or reports it as a duplicate.
    fn claim_slot(&mut self, relationship: &'sch str, slot: MountSlot) -> bool {
        if self.mounted.insert((relationship, slot)) {
            true
        } else {
            let kind = self.schema.name().to_string();
            self.routes
                .push_error(RouterError::DuplicateRelationshipSlot {
                    kind,
                    relationship: relationship.to_string(),
                    slot,
                });
            false
        }
    }

    fn mount_relationship(&mut self, name: &str, slots: SlotSet, config: SlotConfig<'sch>) {
        let (relationship, kind) = match self.schema.relationship(name) {
            Some(descriptor) => (descriptor.name, descriptor.kind),
            None => {
                let owner = self.schema.name().to_string();
                self.routes.push_error(RouterError::UnknownRelationship {
                    kind: owner,
                    relationship: name.to_string(),
                });
                return;
            }
        };

        if slots.linkage {
            self.mount_linkage(relationship, kind, &config);
        }
        if slots.related {
            self.mount_related(relationship, &config);
        }
    }

    fn mount_linkage(
        &mut self,
        relationship: &'sch str,
        kind: RelationshipKind,
        config: &SlotConfig<'sch>,
    ) {
        if !self.claim_slot(relationship, MountSlot::Linkage) {
            return;
        }

        let schema = self.schema;
        let segment = config
            .segment
            .clone()
            .unwrap_or(Cow::Borrowed(relationship));
        let keyword = config
            .keyword
            .clone()
            .unwrap_or(Cow::Borrowed("relationships"));
        let path = || [Cow::Borrowed(":id"), keyword.clone(), segment.clone()];
        self.relationships.entry(relationship).or_default().linkage =
            Some(self.path.iter().cloned().chain(path()).collect());
        let to_many = kind == RelationshipKind::HasMany;

        self.mount(
            Method::GET,
            path(),
            EndpointHandler::resource(schema, move |context| {
                T::default().linkage(context, relationship)
            }),
        );

        if config.read_only {
            self.mount(
                Method::PATCH,
                path(),
                EndpointHandler::resource(schema, forbidden),
            );
            if to_many {
                self.mount(
                    Method::POST,
                    path(),
                    EndpointHandler::resource(schema, forbidden),
                );
                self.mount(
                    Method::DELETE,
                    path(),
                    EndpointHandler::resource(schema, forbidden),
                );
            }
        } else {
            self.mount(
                Method::PATCH,
                path(),
                EndpointHandler::resource(schema, move |context| {
                    T::default().relink(context, relationship)
                }),
            );
            if to_many {
                self.mount(
                    Method::POST,
                    path(),
                    EndpointHandler::resource(schema, move |context| {
                        T::default().link(context, relationship)
                    }),
                );
                self.mount(
                    Method::DELETE,
                    path(),
                    EndpointHandler::resource(schema, move |context| {
                        T::default().unlink(context, relationship)
                    }),
                );
            }
        }
    }

    fn mount_related(&mut self, relationship: &'sch str, config: &SlotConfig<'sch>) {
        if !self.claim_slot(relationship, MountSlot::Related) {
            return;
        }

        let schema = self.schema;
        let segment = config
            .segment
            .clone()
            .unwrap_or(Cow::Borrowed(relationship));
        self.relationships.entry(relationship).or_default().related = Some(
            self.path
                .iter()
                .cloned()
                .chain([Cow::Borrowed(":id"), segment.clone()])
                .collect(),
        );

        self.mount(
            Method::GET,
            [Cow::Borrowed(":id"), segment],
            EndpointHandler::resource(schema, move |context| {
                T::default().related(context, relationship)
            }),
        );
    }

    fn mount_all(&mut self, slots: SlotSet, config: SlotConfig<'sch>) {
        let schema = self.schema;
        for (name, _) in schema.relationships() {
            self.mount_relationship(name, slots, config.clone());
        }
    }

    fn mount_many(&mut self, names: &[&str], slots: SlotSet, config: SlotConfig<'sch>) {
        for name in names {
            self.mount_relationship(name, slots, config.clone());
        }
    }

    pub fn relationship(mut self, name: &str) -> Self {
        let config = self.default_config();
        self.mount_relationship(name, SlotSet::BOTH, config);
        self
    }

    pub fn relationship_with(
        mut self,
        name: &str,
        configure: impl FnOnce(RelationshipConfig<'sch>) -> RelationshipConfig<'sch>,
    ) -> Self {
        let config = configure(RelationshipConfig::default()).resolve(self.read_only);
        self.mount_relationship(name, SlotSet::BOTH, config);
        self
    }

    pub fn relationships(mut self, names: &[&str]) -> Self {
        let config = self.default_config();
        self.mount_many(names, SlotSet::BOTH, config);
        self
    }

    pub fn relationships_with(
        mut self,
        names: &[&str],
        configure: impl FnOnce(RelationshipsConfig<'sch>) -> RelationshipsConfig<'sch>,
    ) -> Self {
        let config = configure(RelationshipsConfig::default()).resolve(self.read_only);
        self.mount_many(names, SlotSet::BOTH, config);
        self
    }

    pub fn all_relationships(mut self) -> Self {
        let config = self.default_config();
        self.mount_all(SlotSet::BOTH, config);
        self
    }

    pub fn all_relationships_with(
        mut self,
        configure: impl FnOnce(RelationshipsConfig<'sch>) -> RelationshipsConfig<'sch>,
    ) -> Self {
        let config = configure(RelationshipsConfig::default()).resolve(self.read_only);
        self.mount_all(SlotSet::BOTH, config);
        self
    }

    pub fn linkage(mut self, name: &str) -> Self {
        let config = self.default_config();
        self.mount_relationship(name, SlotSet::LINKAGE, config);
        self
    }

    pub fn linkage_with(
        mut self,
        name: &str,
        configure: impl FnOnce(RelationshipConfig<'sch>) -> RelationshipConfig<'sch>,
    ) -> Self {
        let config = configure(RelationshipConfig::default()).resolve(self.read_only);
        self.mount_relationship(name, SlotSet::LINKAGE, config);
        self
    }

    pub fn linkages(mut self, names: &[&str]) -> Self {
        let config = self.default_config();
        self.mount_many(names, SlotSet::LINKAGE, config);
        self
    }

    pub fn all_linkages(mut self) -> Self {
        let config = self.default_config();
        self.mount_all(SlotSet::LINKAGE, config);
        self
    }

    pub fn related(mut self, name: &str) -> Self {
        let config = self.default_config();
        self.mount_relationship(name, SlotSet::RELATED, config);
        self
    }

    pub fn related_with(
        mut self,
        name: &str,
        configure: impl FnOnce(RelationshipConfig<'sch>) -> RelationshipConfig<'sch>,
    ) -> Self {
        let config = configure(RelationshipConfig::default()).resolve(self.read_only);
        self.mount_relationship(name, SlotSet::RELATED, config);
        self
    }

    pub fn relateds(mut self, names: &[&str]) -> Self {
        let config = self.default_config();
        self.mount_many(names, SlotSet::RELATED, config);
        self
    }

    pub fn all_relateds(mut self) -> Self {
        let config = self.default_config();
        self.mount_all(SlotSet::RELATED, config);
        self
    }

    /// Opens a record-scoped block at `:id`: its custom routes are bound to a single resource.
    pub fn member(
        mut self,
        configure: impl FnOnce(
            SubordinateRouteBuilder<'sch, Adapter>,
        ) -> SubordinateRouteBuilder<'sch, Adapter>,
    ) -> Self {
        let path = self
            .path
            .iter()
            .cloned()
            .chain([Cow::Borrowed(":id")])
            .collect();
        let child = configure(SubordinateRouteBuilder::at(
            path,
            self.schema,
            self.middleware.clone(),
        ));
        self.routes.absorb(child.into_routes());
        self
    }
}

/// The member body: verbs and nested scopes whose handlers receive a `ResourceContext` bound to the
/// resource schema, at the record (`:id`) path.
pub struct SubordinateRouteBuilder<'sch, Adapter: AdapterInterface> {
    path: Vec<Cow<'sch, str>>,
    schema: &'sch Schema<'sch>,
    middleware: Vec<Middleware<'sch, Adapter>>,
    routes: MaterialisedRoutes<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> RouteBuilder<'sch, Adapter>
    for SubordinateRouteBuilder<'sch, Adapter>
{
    fn path(&self) -> &[Cow<'sch, str>] {
        &self.path
    }

    fn middleware(&self) -> &[Middleware<'sch, Adapter>] {
        &self.middleware
    }

    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter> {
        &mut self.routes
    }

    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes
    }

    fn spawn(
        &self,
        segments: impl IntoIterator<Item = Cow<'sch, str>>,
        middleware: impl IntoIterator<Item = Middleware<'sch, Adapter>>,
    ) -> Self {
        Self {
            path: self.path.iter().cloned().chain(segments).collect(),
            schema: self.schema,
            middleware: self.middleware.iter().cloned().chain(middleware).collect(),
            routes: MaterialisedRoutes::new(),
        }
    }
}

impl<'sch, Adapter: AdapterInterface + 'sch> ResourceVerbs<'sch, Adapter>
    for SubordinateRouteBuilder<'sch, Adapter>
{
    fn schema(&self) -> &'sch Schema<'sch> {
        self.schema
    }
}

impl<'sch, Adapter: AdapterInterface + 'sch> SubordinateRouteBuilder<'sch, Adapter> {
    fn at(
        path: Vec<Cow<'sch, str>>,
        schema: &'sch Schema<'sch>,
        middleware: Vec<Middleware<'sch, Adapter>>,
    ) -> Self {
        Self {
            path,
            schema,
            middleware,
            routes: MaterialisedRoutes::new(),
        }
    }

    /// Wraps the routes `build` mounts with `middleware`, on the JSON:API tier: spawns a sibling
    /// carrying the extended middleware, runs `build`, merges it back.
    pub fn middleware(
        mut self,
        middleware: impl ResourceMiddleware<'sch, Adapter>,
        build: impl FnOnce(Self) -> Self,
    ) -> Self {
        let child = build(self.spawn_with_middleware(Middleware::Resource(Arc::new(middleware))));
        self.routes.absorb(child.into_routes());
        self
    }
}
