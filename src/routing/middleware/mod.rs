pub mod json_api;

use super::context::{PrimaryContext, ResourceContext};
use super::result::{PrimaryResult, ResourceResult};
use super::route_parameters::RouteParameters;
use crate::database::adapters::Adapter as AdapterInterface;
use crate::http_wrappers::Uri;
use http::HeaderMap;
use std::sync::Arc;

/// Any primary-tier handler: a request handler on the raw byte tier, and the `next` continuation a
/// `PrimaryMiddleware` calls to run the rest of the chain. The `'req` bound ties the erased handler
/// to the request, so the recursive continuation need not be `'static`.
pub type PrimaryHandler<'sch, 'req, Adapter> =
    dyn Fn(PrimaryContext<'sch, 'req, Adapter>) -> PrimaryResult + 'req;

/// Any resource-tier handler: a schema-bound handler, and the `next` continuation a
/// `ResourceMiddleware` calls to run the rest of the chain.
pub type ResourceHandler<'sch, 'req, Adapter> =
    dyn Fn(ResourceContext<'sch, 'req, Adapter>) -> ResourceResult + 'req;

/// Middleware over the raw byte tier. It fronts a `PrimaryContext` — the request head and the
/// byte-stream body — and is declared inside a routing block, wrapping every route in that block.
pub trait PrimaryMiddleware<'sch, Adapter: AdapterInterface + 'sch>: Send + Sync + 'sch {
    /// Constrain routing: `false` makes the wrapped routes not match this request. Runs during
    /// matching, before any context exists, from the request's headers, URL, and route parameters.
    fn matches(&self, _headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        true
    }

    /// Act on the context, call `next` to run the rest of the chain, act on its response — or skip
    /// `next` to short-circuit.
    fn handle<'req>(
        &self,
        context: PrimaryContext<'sch, 'req, Adapter>,
        next: &PrimaryHandler<'sch, 'req, Adapter>,
    ) -> PrimaryResult
    where
        'sch: 'req,
    {
        next(context)
    }
}

/// Middleware over the JSON:API tier. It fronts a `ResourceContext` — the schema-bound request and
/// its parsed document — entered at a resource boundary.
pub trait ResourceMiddleware<'sch, Adapter: AdapterInterface + 'sch>: Send + Sync + 'sch {
    fn matches(&self, _headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        true
    }

    fn handle<'req>(
        &self,
        context: ResourceContext<'sch, 'req, Adapter>,
        next: &ResourceHandler<'sch, 'req, Adapter>,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        next(context)
    }
}

/// One middleware in a route's chain, tagged by the tier it works. `Arc`-shared so the route table
/// clones cheaply and a middleware may hold unique resources behind interior mutability. A route's
/// chain is partitioned primaries-first, then resources.
///
/// Part of the builder seam threaded through the public `RouteBuilder` trait, hence `pub` — but an
/// internal type users never name (they pass middleware values to `.middleware`), so `doc(hidden)`.
#[doc(hidden)]
pub enum Middleware<'sch, Adapter: AdapterInterface> {
    Primary(Arc<dyn PrimaryMiddleware<'sch, Adapter>>),
    Resource(Arc<dyn ResourceMiddleware<'sch, Adapter>>),
}

/// Cloning bumps the `Arc` refcount — the middleware itself is never copied. Hand-written rather than
/// derived so the bound stays `Adapter: AdapterInterface`, not the `Adapter: Clone` a derive imposes.
impl<'sch, Adapter: AdapterInterface> Clone for Middleware<'sch, Adapter> {
    fn clone(&self) -> Self {
        match self {
            Middleware::Primary(middleware) => Middleware::Primary(middleware.clone()),
            Middleware::Resource(middleware) => Middleware::Resource(middleware.clone()),
        }
    }
}

impl<'sch, Adapter: AdapterInterface + 'sch> Middleware<'sch, Adapter> {
    /// The middleware's match-guard, dispatched to whichever tier it belongs to.
    pub(crate) fn matches(&self, headers: &HeaderMap, uri: &Uri, route: &RouteParameters) -> bool {
        match self {
            Middleware::Primary(middleware) => middleware.matches(headers, uri, route),
            Middleware::Resource(middleware) => middleware.matches(headers, uri, route),
        }
    }

    /// Whether this is a resource-tier middleware — the primaries/resources partition check.
    pub(crate) fn is_resource(&self) -> bool {
        matches!(self, Middleware::Resource(_))
    }
}
