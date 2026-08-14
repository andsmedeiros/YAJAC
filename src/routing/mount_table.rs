use crate::database::adapters::Adapter as AdapterInterface;
use crate::routing::controller::ResourceController;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::ops::Deref;

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
/// captured for it. A kind the router never mounted has no entry, and so no links.
pub(crate) struct MountTable<'sch, Adapter: AdapterInterface> {
    mounts: IndexMap<&'sch str, ResourceMount<'sch, Adapter>>,
}

impl<'sch, Adapter: AdapterInterface> MountTable<'sch, Adapter> {
    /// Wraps an already-deduplicated set of mounts, keyed by kind — the router's own assembly path.
    pub(crate) fn new(mounts: IndexMap<&'sch str, ResourceMount<'sch, Adapter>>) -> Self {
        Self { mounts }
    }
}

impl<'sch, Adapter: AdapterInterface> Default for MountTable<'sch, Adapter> {
    fn default() -> Self {
        Self {
            mounts: IndexMap::new(),
        }
    }
}

impl<'sch, Adapter: AdapterInterface> Deref for MountTable<'sch, Adapter> {
    type Target = IndexMap<&'sch str, ResourceMount<'sch, Adapter>>;

    fn deref(&self) -> &Self::Target {
        &self.mounts
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
