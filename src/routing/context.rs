use super::PrimaryRequest;
use crate::database::error::Error;
use crate::database::schema::Schema;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, connection::Connection as ConnectionInterface,
        connection_manager::ConnectionManager, query_parameters::QueryParameters, store::Store,
    },
    http_wrappers::{StatusCode, Uri},
    routing::{BaseUri, Error as RoutingError, MountTable, RouteParameters},
    serialisation::ByteStream,
    serialisation::uri_generator::UriGenerator,
};
use http::HeaderMap;
use std::cell::LazyCell;

/// A lazily-acquired request connection: unforced until first use, then the pooled handle or the
/// failure that acquiring it produced. Boxed because the init closure captures the manager.
type LazyConnection<'sch, Adapter> = LazyCell<
    Result<<Adapter as AdapterInterface>::Connection, Error>,
    Box<dyn FnOnce() -> Result<<Adapter as AdapterInterface>::Connection, Error> + 'sch>,
>;

/// The raw byte tier's request context: the request head and a streamed body, schema-oblivious. The
/// crossing upgrades it to a `ResourceContext` at a resource boundary; document-shaped request
/// operations live there.
pub struct PrimaryContext<'sch, 'req, Adapter: AdapterInterface>
where
    'sch: 'req,
{
    pub manager: &'sch ConnectionManager<'sch, Adapter>,
    pub uri: &'req Uri,
    base_uri: &'req BaseUri<'sch>,
    mount_table: &'req MountTable<'sch, Adapter>,
    body: Option<ByteStream>,
    headers: HeaderMap,
    route: RouteParameters,
    connection: LazyConnection<'sch, Adapter>,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface> PrimaryContext<'sch, 'req, Adapter> {
    /// Builds a context from the request, harvesting its streamed body and headers and discarding
    /// the rest; `uri` is lent separately so the borrowing query parameters can reference it.
    pub(crate) fn from_request(
        manager: &'sch ConnectionManager<'sch, Adapter>,
        base_uri: &'req BaseUri<'sch>,
        mount_table: &'req MountTable<'sch, Adapter>,
        uri: &'req Uri,
        route: RouteParameters,
        request: PrimaryRequest,
    ) -> Self {
        let (parts, body) = request.into_parts();
        let acquire: Box<dyn FnOnce() -> Result<Adapter::Connection, Error> + 'sch> =
            Box::new(move || manager.acquire());

        Self {
            manager,
            uri,
            base_uri,
            mount_table,
            body: Some(body),
            headers: parts.headers,
            route,
            connection: LazyCell::new(acquire),
        }
    }

    /// The link generator for this request, resolving each record's links against where its type is
    /// mounted. Cheap to build — a view over the base, the mount table, and the request.
    pub(crate) fn uri_generator(&self) -> UriGenerator<'sch, '_, Adapter> {
        UriGenerator::new(self.base_uri, self.mount_table, &self.route, &self.headers)
    }

    /// The base every link is rooted at — lent to the crossing for error-document rendering.
    pub(crate) fn base_uri(&self) -> &'req BaseUri<'sch> {
        self.base_uri
    }

    /// The mount table controllers and link templates resolve through — lent to the crossing.
    pub(crate) fn mount_table(&self) -> &'req MountTable<'sch, Adapter> {
        self.mount_table
    }

    /// Takes the request body stream by value — a primary handler owns it to read or parse however
    /// it needs (a document, a multipart upload, a file), and `require_*` take it here too. Since a
    /// context is always built with a body, `None` means it was already consumed upstream, an
    /// internal invariant violation rather than a client fault — hence the 500.
    pub fn require_body(&mut self) -> Result<ByteStream, RoutingError> {
        self.body.take().ok_or_else(|| {
            RoutingError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BodyConsumed",
                "The request body has already been consumed",
            )
        })
    }

    pub fn body(&self) -> &Option<ByteStream> {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut Option<ByteStream> {
        &mut self.body
    }

    /// Lazily acquires the request connection from the pool and lends it as a shared reference.
    pub fn connection(&self) -> Result<&Adapter::Connection, Error> {
        LazyCell::force(&self.connection)
            .as_ref()
            .map_err(|error| error.clone())
    }

    pub fn table(&self, name: &str) -> Result<Adapter::Table<'sch, '_>, Error> {
        self.manager.table(name, self.connection()?)
    }

    pub fn store(&self) -> Result<Store<'sch, '_, Adapter>, Error> {
        Ok(Store::new(self.manager, self.connection()?))
    }

    /// Runs `operation` inside a transaction on the request connection.
    pub fn transaction<R>(
        &self,
        operation: impl FnOnce(&Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        self.connection()?.transaction(|| operation(self))
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn route_parameters(&self) -> &RouteParameters {
        &self.route
    }

    /// Parses this request's query string against `schema` — the hatch for a `QueryParameters` bound
    /// to any schema (e.g. `related`'s related type). Uncached; the cached, own-schema query is the
    /// `LazyCell` on `ResourceContext`.
    pub fn parse_query(
        &self,
        schema: &'sch Schema<'sch>,
    ) -> Result<QueryParameters<'sch, 'req>, Error> {
        QueryParameters::parse(self.uri, schema, self.manager.registry())
    }
}
