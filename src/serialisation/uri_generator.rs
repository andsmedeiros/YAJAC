use super::error::Error;
use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::record::Record;
use crate::http_wrappers::Uri;
use crate::routing::BaseUri;
use crate::routing::RouteParameters;
use crate::routing::mount_table::{ControllerFactory, MountTable};
use http::HeaderMap;
use std::borrow::Cow;

/// The router's link generator: a per-request view that renders a resource's `self`, relationship,
/// and related links from where its type is actually mounted. Private — the router builds it and the
/// serialisation funnel drives it; there is no user-facing seam. A type with no mount, or a
/// relationship slot that was not mounted, yields `None` (no link) rather than a broken one.
pub(crate) struct UriGenerator<'sch, 'req, Adapter: AdapterInterface> {
    base: &'req BaseUri<'sch>,
    mount_table: &'req MountTable<'sch, Adapter>,
    route: &'req RouteParameters,
    headers: &'req HeaderMap,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> UriGenerator<'sch, 'req, Adapter> {
    pub(crate) fn new(
        base: &'req BaseUri<'sch>,
        mount_table: &'req MountTable<'sch, Adapter>,
        route: &'req RouteParameters,
        headers: &'req HeaderMap,
    ) -> Self {
        Self {
            base,
            mount_table,
            route,
            headers,
        }
    }

    /// The resource's `self` link, or `None` when its type is not mounted. The resource path is the
    /// mount's base prefix plus `:id`.
    pub(crate) fn uri_for_resource(&self, record: &Record<'sch>) -> Result<Option<Uri>, Error> {
        self.mount_table
            .get(record.schema.name())
            .map(|mount| {
                let template: Vec<Cow<'sch, str>> = mount
                    .base
                    .iter()
                    .cloned()
                    .chain([Cow::Borrowed(":id")])
                    .collect();
                self.render(record, &template, mount.factory)
            })
            .transpose()
    }

    /// The relationship's `self` (linkage) link, or `None` when the type or that slot is not mounted.
    pub(crate) fn uri_for_linkage(
        &self,
        record: &Record<'sch>,
        relationship: &str,
    ) -> Result<Option<Uri>, Error> {
        if let Some(mount) = self.mount_table.get(record.kind())
            && let Some(template) = mount
                .relationships
                .get(relationship)
                .and_then(|mounts| mounts.linkage.as_deref())
        {
            self.render(record, template, mount.factory).map(Some)
        } else {
            Ok(None)
        }
    }

    /// The related-resource link, or `None` when the type or that slot is not mounted.
    pub(crate) fn uri_for_related(
        &self,
        record: &Record<'sch>,
        relationship: &str,
    ) -> Result<Option<Uri>, Error> {
        if let Some(mount) = self.mount_table.get(record.kind())
            && let Some(template) = mount
                .relationships
                .get(relationship)
                .and_then(|mounts| mounts.related.as_deref())
        {
            self.render(record, template, mount.factory).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Renders `template` for `record`: resolves its dynamic parameters through the record's
    /// controller, then substitutes and roots them against the base.
    fn render(
        &self,
        record: &Record<'sch>,
        template: &[Cow<'sch, str>],
        controller_factory: ControllerFactory<'sch, Adapter>,
    ) -> Result<Uri, Error> {
        let required: Vec<&'sch str> = template
            .iter()
            .filter_map(|segment| match segment {
                Cow::Borrowed(name) => name.strip_prefix(':'),
                Cow::Owned(_) => None,
            })
            .collect();

        let resolved = if required.is_empty() {
            Default::default()
        } else {
            controller_factory().parameters_for_route(record, self.route, self.headers, &required)
        };

        self.base.render(template, &resolved)
    }
}
