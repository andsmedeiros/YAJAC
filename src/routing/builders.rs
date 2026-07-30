use super::{
    Context, Error, Result,
    controller::{ResourceContext, ResourceController},
    router::{
        Handler, MaterialisedRoutes, MountSlot, ResourceMount, Route, RouterError, split_segments,
    },
};
use crate::database::{
    adapters::Adapter as AdapterInterface,
    schema::{RelationshipKind, Schema},
};
use crate::http_wrappers::StatusCode;
use http::Method;
use std::borrow::Cow;
use std::collections::HashSet;
use std::marker::PhantomData;

/// The seam every route builder shares: its mount prefix, its route accumulator, and how it spawns
/// a sibling of itself for a nested scope.
pub trait RouteBuilder<'sch, Adapter: AdapterInterface + 'sch>: Sized {
    fn prefix(&self) -> &[Cow<'sch, str>];
    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter>;
    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter>;
    fn spawn(&self, prefix: Vec<Cow<'sch, str>>) -> Self;

    /// The mount path of a `/`-delimited segment string relative to this builder's prefix.
    fn extend_prefix(&self, segment: &'sch str) -> Vec<Cow<'sch, str>> {
        self.prefix()
            .iter()
            .cloned()
            .chain(split_segments(segment))
            .collect()
    }

    /// Mounts `handler` for `method` at `segments` relative to this builder's prefix.
    fn mount(
        &mut self,
        method: Method,
        segments: impl IntoIterator<Item = Cow<'sch, str>>,
        handler: impl Handler<'sch, Adapter>,
    ) {
        let path = self.prefix().iter().cloned().chain(segments).collect();
        self.routes_mut()
            .push_route(Ok(Route::new(method, path, handler)));
    }

    /// Opens a nested scope at `segment`: builds a sibling, runs `configure`, merges it back.
    fn scope(mut self, segment: &'sch str, configure: impl FnOnce(Self) -> Self) -> Self {
        let child = configure(self.spawn(self.extend_prefix(segment)));
        self.routes_mut().absorb(child.into_routes());
        self
    }
}

/// The verb surface whose handlers receive a bare `Context`, shared by the builders that mount
/// schema-oblivious routes.
pub trait UnboundVerbs<'sch, Adapter: AdapterInterface + 'sch>: RouteBuilder<'sch, Adapter> {
    fn get(mut self, segment: &'sch str, handler: impl Handler<'sch, Adapter>) -> Self {
        self.mount(Method::GET, split_segments(segment), handler);
        self
    }

    fn post(mut self, segment: &'sch str, handler: impl Handler<'sch, Adapter>) -> Self {
        self.mount(Method::POST, split_segments(segment), handler);
        self
    }

    fn put(mut self, segment: &'sch str, handler: impl Handler<'sch, Adapter>) -> Self {
        self.mount(Method::PUT, split_segments(segment), handler);
        self
    }

    fn patch(mut self, segment: &'sch str, handler: impl Handler<'sch, Adapter>) -> Self {
        self.mount(Method::PATCH, split_segments(segment), handler);
        self
    }

    fn delete(mut self, segment: &'sch str, handler: impl Handler<'sch, Adapter>) -> Self {
        self.mount(Method::DELETE, split_segments(segment), handler);
        self
    }
}

/// A resource-scoped request handler: bound to the resource's schema by the builder before it is
/// mounted, so custom member/collection routes receive a `ResourceContext`.
pub trait ResourceHandler<'sch, Adapter: AdapterInterface>:
    for<'req> Fn(ResourceContext<'sch, 'req, Adapter>) -> Result + Send + Sync + 'sch
{
}

impl<'sch, T, Adapter: AdapterInterface> ResourceHandler<'sch, Adapter> for T where
    T: for<'req> Fn(ResourceContext<'sch, 'req, Adapter>) -> Result + Send + Sync + 'sch
{
}

/// The refusal a read-only mount serves for an unsupported write.
fn forbidden<'sch, 'req, Adapter: AdapterInterface>(
    _context: Context<'sch, 'req, Adapter>,
) -> Result {
    Err(Error::new(
        StatusCode::FORBIDDEN,
        "Forbidden",
        "This endpoint does not support the requested operation",
    ))
}

/// The top-level builder: mounts resources and opens scopes at the router root, and yields the same
/// builder for every nested scope.
pub struct PrimaryRouteBuilder<'sch, Adapter: AdapterInterface> {
    prefix: Vec<Cow<'sch, str>>,
    routes: MaterialisedRoutes<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> RouteBuilder<'sch, Adapter>
    for PrimaryRouteBuilder<'sch, Adapter>
{
    fn prefix(&self) -> &[Cow<'sch, str>] {
        &self.prefix
    }

    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter> {
        &mut self.routes
    }

    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes
    }

    fn spawn(&self, prefix: Vec<Cow<'sch, str>>) -> Self {
        Self {
            prefix,
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
            prefix: Vec::new(),
            routes: MaterialisedRoutes::new(),
        }
    }

    pub fn resource<T>(mut self, segment: &'sch str, schema: &'sch Schema<'sch>) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        let built = ResourceRouteBuilder::<T, Adapter>::new(
            self.extend_prefix(segment),
            schema,
            false,
        )
        .all_relationships()
        .build();
        self.routes.absorb(built);
        self
    }

    pub fn resource_with<T>(
        mut self,
        segment: &'sch str,
        schema: &'sch Schema<'sch>,
        configure: impl FnOnce(ResourceRouteBuilder<'sch, T, Adapter>) -> ResourceRouteBuilder<'sch, T, Adapter>,
    ) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        let built = configure(ResourceRouteBuilder::new(
            self.extend_prefix(segment),
            schema,
            false,
        ))
        .build();
        self.routes.absorb(built);
        self
    }

    pub fn read_only_resource<T>(mut self, segment: &'sch str, schema: &'sch Schema<'sch>) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        let built = ResourceRouteBuilder::<T, Adapter>::new(
            self.extend_prefix(segment),
            schema,
            true,
        )
        .all_relationships()
        .build();
        self.routes.absorb(built);
        self
    }

    pub fn read_only_resource_with<T>(
        mut self,
        segment: &'sch str,
        schema: &'sch Schema<'sch>,
        configure: impl FnOnce(ResourceRouteBuilder<'sch, T, Adapter>) -> ResourceRouteBuilder<'sch, T, Adapter>,
    ) -> Self
    where
        T: ResourceController<'sch, Adapter> + Default + 'sch,
    {
        let built = configure(ResourceRouteBuilder::new(
            self.extend_prefix(segment),
            schema,
            true,
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

    pub fn at(mut self, segment: &'sch str, keyword: &'sch str) -> Self {
        self.segment = Some(Cow::Borrowed(segment));
        self.keyword = Some(Cow::Borrowed(keyword));
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

    pub fn at(mut self, keyword: &'sch str) -> Self {
        self.keyword = Some(Cow::Borrowed(keyword));
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

/// The `resource[_with]` surface: a resource's CRUD is mounted on construction; this builder adds
/// its relationship endpoints and custom member/collection routes.
pub struct ResourceRouteBuilder<'sch, T, Adapter: AdapterInterface> {
    prefix: Vec<Cow<'sch, str>>,
    schema: &'sch Schema<'sch>,
    read_only: bool,
    routes: MaterialisedRoutes<'sch, Adapter>,
    mounted: HashSet<(&'sch str, MountSlot)>,
    controller: PhantomData<fn() -> T>,
}

impl<'sch, T, Adapter: AdapterInterface + 'sch> RouteBuilder<'sch, Adapter>
    for ResourceRouteBuilder<'sch, T, Adapter>
{
    fn prefix(&self) -> &[Cow<'sch, str>] {
        &self.prefix
    }

    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter> {
        &mut self.routes
    }

    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes
    }

    fn spawn(&self, prefix: Vec<Cow<'sch, str>>) -> Self {
        Self {
            prefix,
            schema: self.schema,
            read_only: self.read_only,
            routes: MaterialisedRoutes::new(),
            mounted: HashSet::new(),
            controller: PhantomData,
        }
    }
}

impl<'sch, T, Adapter: AdapterInterface + 'sch> UnboundVerbs<'sch, Adapter>
    for ResourceRouteBuilder<'sch, T, Adapter>
{
}

impl<'sch, T, Adapter> ResourceRouteBuilder<'sch, T, Adapter>
where
    T: ResourceController<'sch, Adapter> + Default + 'sch,
    Adapter: AdapterInterface + 'sch,
{
    pub(crate) fn new(
        prefix: Vec<Cow<'sch, str>>,
        schema: &'sch Schema<'sch>,
        read_only: bool,
    ) -> Self {
        Self {
            prefix,
            schema,
            read_only,
            routes: MaterialisedRoutes::new(),
            mounted: HashSet::new(),
            controller: PhantomData,
        }
    }

    /// Finalises the resource: mounts its default endpoints, records its canonical mount, and
    /// yields the accumulated routes. Defaults mount here, after the closure's routes.
    pub(crate) fn build(mut self) -> MaterialisedRoutes<'sch, Adapter> {
        self.mount_default_verbs();
        self.routes.push_mount(ResourceMount {
            kind: self.schema.name(),
            factory: || Box::new(T::default()) as Box<dyn ResourceController<'sch, Adapter>>,
        });
        self.routes
    }

    fn mount_default_verbs(&mut self) {
        let schema = self.schema;
        self.mount(Method::GET, std::iter::empty(), move |context| {
            T::default().index(ResourceContext::new(schema, context))
        });
        self.mount(Method::GET, [Cow::Borrowed(":id")], move |context| {
            T::default().show(ResourceContext::new(schema, context))
        });

        if self.read_only {
            self.mount(Method::POST, std::iter::empty(), forbidden);
            self.mount(Method::PUT, [Cow::Borrowed(":id")], forbidden);
            self.mount(Method::PATCH, [Cow::Borrowed(":id")], forbidden);
            self.mount(Method::DELETE, [Cow::Borrowed(":id")], forbidden);
        } else {
            self.mount(Method::POST, std::iter::empty(), move |context| {
                T::default().create(ResourceContext::new(schema, context))
            });
            self.mount(Method::PUT, [Cow::Borrowed(":id")], move |context| {
                T::default().update(ResourceContext::new(schema, context))
            });
            self.mount(Method::PATCH, [Cow::Borrowed(":id")], move |context| {
                T::default().update(ResourceContext::new(schema, context))
            });
            self.mount(Method::DELETE, [Cow::Borrowed(":id")], move |context| {
                T::default().delete(ResourceContext::new(schema, context))
            });
        }
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
            self.routes.push_route(Err(RouterError::DuplicateRelationshipSlot {
                kind,
                relationship: relationship.to_string(),
                slot,
            }));
            false
        }
    }

    fn mount_relationship(&mut self, name: &str, slots: SlotSet, config: SlotConfig<'sch>) {
        let (relationship, kind) = match self.schema.relationship(name) {
            Some(descriptor) => (descriptor.name, descriptor.kind),
            None => {
                let owner = self.schema.name().to_string();
                self.routes.push_route(Err(RouterError::UnknownRelationship {
                    kind: owner,
                    relationship: name.to_string(),
                }));
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
        let segment = config.segment.clone().unwrap_or(Cow::Borrowed(relationship));
        let keyword = config.keyword.clone().unwrap_or(Cow::Borrowed("relationships"));
        let path = || [Cow::Borrowed(":id"), keyword.clone(), segment.clone()];
        let to_many = kind == RelationshipKind::HasMany;

        self.mount(Method::GET, path(), move |context| {
            T::default().linkage(ResourceContext::new(schema, context), relationship)
        });

        if config.read_only {
            self.mount(Method::PATCH, path(), forbidden);
            if to_many {
                self.mount(Method::POST, path(), forbidden);
                self.mount(Method::DELETE, path(), forbidden);
            }
        } else {
            self.mount(Method::PATCH, path(), move |context| {
                T::default().relink(ResourceContext::new(schema, context), relationship)
            });
            if to_many {
                self.mount(Method::POST, path(), move |context| {
                    T::default().link(ResourceContext::new(schema, context), relationship)
                });
                self.mount(Method::DELETE, path(), move |context| {
                    T::default().unlink(ResourceContext::new(schema, context), relationship)
                });
            }
        }
    }

    fn mount_related(&mut self, relationship: &'sch str, config: &SlotConfig<'sch>) {
        if !self.claim_slot(relationship, MountSlot::Related) {
            return;
        }

        let schema = self.schema;
        let segment = config.segment.clone().unwrap_or(Cow::Borrowed(relationship));

        self.mount(Method::GET, [Cow::Borrowed(":id"), segment], move |context| {
            T::default().related(ResourceContext::new(schema, context), relationship)
        });
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

    pub fn member(
        mut self,
        configure: impl FnOnce(SubordinateRouteBuilder<'sch, Adapter>) -> SubordinateRouteBuilder<'sch, Adapter>,
    ) -> Self {
        let prefix = self
            .prefix
            .iter()
            .cloned()
            .chain([Cow::Borrowed(":id")])
            .collect();
        let child = configure(SubordinateRouteBuilder::at(prefix, self.schema));
        self.routes.absorb(child.into_routes());
        self
    }

    pub fn collection(
        mut self,
        configure: impl FnOnce(SubordinateRouteBuilder<'sch, Adapter>) -> SubordinateRouteBuilder<'sch, Adapter>,
    ) -> Self {
        let child = configure(SubordinateRouteBuilder::at(self.prefix.clone(), self.schema));
        self.routes.absorb(child.into_routes());
        self
    }
}

/// The member/collection body: verbs and nested scopes whose handlers receive a `ResourceContext`
/// bound to the resource schema.
pub struct SubordinateRouteBuilder<'sch, Adapter: AdapterInterface> {
    prefix: Vec<Cow<'sch, str>>,
    schema: &'sch Schema<'sch>,
    routes: MaterialisedRoutes<'sch, Adapter>,
}

impl<'sch, Adapter: AdapterInterface + 'sch> RouteBuilder<'sch, Adapter>
    for SubordinateRouteBuilder<'sch, Adapter>
{
    fn prefix(&self) -> &[Cow<'sch, str>] {
        &self.prefix
    }

    fn routes_mut(&mut self) -> &mut MaterialisedRoutes<'sch, Adapter> {
        &mut self.routes
    }

    fn into_routes(self) -> MaterialisedRoutes<'sch, Adapter> {
        self.routes
    }

    fn spawn(&self, prefix: Vec<Cow<'sch, str>>) -> Self {
        Self {
            prefix,
            schema: self.schema,
            routes: MaterialisedRoutes::new(),
        }
    }
}

impl<'sch, Adapter: AdapterInterface + 'sch> SubordinateRouteBuilder<'sch, Adapter> {
    fn at(prefix: Vec<Cow<'sch, str>>, schema: &'sch Schema<'sch>) -> Self {
        Self {
            prefix,
            schema,
            routes: MaterialisedRoutes::new(),
        }
    }

    /// Binds a resource handler to the resource schema, yielding a schema-oblivious routing handler.
    fn bind(&self, handler: impl ResourceHandler<'sch, Adapter>) -> impl Handler<'sch, Adapter> {
        let schema = self.schema;
        move |context| handler(ResourceContext::new(schema, context))
    }

    pub fn get(mut self, segment: &'sch str, handler: impl ResourceHandler<'sch, Adapter>) -> Self {
        let handler = self.bind(handler);
        self.mount(Method::GET, split_segments(segment), handler);
        self
    }

    pub fn post(mut self, segment: &'sch str, handler: impl ResourceHandler<'sch, Adapter>) -> Self {
        let handler = self.bind(handler);
        self.mount(Method::POST, split_segments(segment), handler);
        self
    }

    pub fn put(mut self, segment: &'sch str, handler: impl ResourceHandler<'sch, Adapter>) -> Self {
        let handler = self.bind(handler);
        self.mount(Method::PUT, split_segments(segment), handler);
        self
    }

    pub fn patch(mut self, segment: &'sch str, handler: impl ResourceHandler<'sch, Adapter>) -> Self {
        let handler = self.bind(handler);
        self.mount(Method::PATCH, split_segments(segment), handler);
        self
    }

    pub fn delete(mut self, segment: &'sch str, handler: impl ResourceHandler<'sch, Adapter>) -> Self {
        let handler = self.bind(handler);
        self.mount(Method::DELETE, split_segments(segment), handler);
        self
    }
}
