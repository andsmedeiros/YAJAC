use super::error::Error;
use crate::database::adapters::Adapter as AdapterInterface;
use crate::database::record::Record;
use crate::http_wrappers::Uri;
use crate::routing::BaseUri;
use crate::routing::RouteParameters;
use crate::routing::mount_table::{ControllerFactory, MountTable};
use http::HeaderMap;
use std::borrow::Cow;

/// Renders a record's `self`, relationship, and related links. The serialisation funnel drives it and
/// stays oblivious to which generator it holds: the canonical one resolves links from where a type is
/// mounted, the null one refuses every link. A slot that has no link yields `None`; a slot that
/// *should* have one but cannot be rendered yields `Err`.
pub(crate) trait UriGenerator<'sch> {
    /// The resource's `self` link, or `None` when its type is not mounted.
    fn uri_for_resource(&self, record: &Record<'sch>) -> Result<Option<Uri>, Error>;

    /// The relationship's `self` (linkage) link, or `None` when the type or that slot is not mounted.
    fn uri_for_linkage(
        &self,
        record: &Record<'sch>,
        relationship: &str,
    ) -> Result<Option<Uri>, Error>;

    /// The related-resource link, or `None` when the type or that slot is not mounted.
    fn uri_for_related(
        &self,
        record: &Record<'sch>,
        relationship: &str,
    ) -> Result<Option<Uri>, Error>;
}

/// The router's real link generator: a per-request view that renders each link from where its type is
/// actually mounted. A type with no mount, or a relationship slot that was not mounted, yields `None`
/// (no link) rather than a broken one.
pub(crate) struct CanonicalUriGenerator<'sch, 'req, Adapter: AdapterInterface> {
    base: &'req BaseUri<'sch>,
    mount_table: &'req MountTable<'sch, Adapter>,
    route: &'req RouteParameters,
    headers: &'req HeaderMap,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>
    CanonicalUriGenerator<'sch, 'req, Adapter>
{
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

    /// Renders `template` for `record`: resolves its dynamic parameters through the record's
    /// controller, then substitutes and roots them against the base.
    fn render(
        &self,
        record: &Record<'sch>,
        template: &[Cow<'sch, str>],
        controller_factory: ControllerFactory<'sch, Adapter>,
    ) -> Result<Uri, Error> {
        let required: Vec<&str> = template
            .iter()
            .filter_map(|segment| segment.strip_prefix(':'))
            .collect();

        let resolved = if required.is_empty() {
            Default::default()
        } else {
            controller_factory().parameters_for_route(record, self.route, self.headers, &required)
        };

        self.base.render(template, &resolved)
    }
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> UriGenerator<'sch>
    for CanonicalUriGenerator<'sch, 'req, Adapter>
{
    fn uri_for_resource(&self, record: &Record<'sch>) -> Result<Option<Uri>, Error> {
        self.mount_table
            .get(record.schema.name())
            .map(|mount| {
                // The resource path is the mount's base (collection) prefix plus `:id`.
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

    fn uri_for_linkage(
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

    fn uri_for_related(
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
}

/// A generator for a document that renders no per-record links — an errors document. Every method
/// fails, making the contract explicit: pass it only where no link is produced. A link request
/// against it is a framework bug, surfaced loudly rather than answered with a fabricated link.
pub(crate) struct NullUriGenerator;

impl NullUriGenerator {
    fn refusal() -> Error {
        Error::LinkGenerationError {
            message: "a link was requested from the null generator, which serves documents that must render none".to_string(),
        }
    }
}

impl<'sch> UriGenerator<'sch> for NullUriGenerator {
    fn uri_for_resource(&self, _record: &Record<'sch>) -> Result<Option<Uri>, Error> {
        Err(Self::refusal())
    }

    fn uri_for_linkage(
        &self,
        _record: &Record<'sch>,
        _relationship: &str,
    ) -> Result<Option<Uri>, Error> {
        Err(Self::refusal())
    }

    fn uri_for_related(
        &self,
        _record: &Record<'sch>,
        _relationship: &str,
    ) -> Result<Option<Uri>, Error> {
        Err(Self::refusal())
    }
}
