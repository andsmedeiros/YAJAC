use super::PrimaryRequest;
use crate::database::error::Error;
use crate::database::schema::Schema;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, connection::Connection as ConnectionInterface,
        connection_manager::ConnectionManager, query_parameters::QueryParameters, store::Store,
    },
    http_wrappers::Uri,
    routing::{BaseUri, Error as RoutingError, MountTable, RouteParameters},
    serialisation::ByteStream,
    serialisation::uri_generator::CanonicalUriGenerator,
};
use http::HeaderMap;
use std::cell::{LazyCell, OnceCell};
use std::io::{Cursor, Read};

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
    /// Whether the body carries content, filled once by the first `contains_body` probe.
    body_present: OnceCell<bool>,
    headers: HeaderMap,
    route: RouteParameters<'sch, 'req>,
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
        route: RouteParameters<'sch, 'req>,
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
            body_present: OnceCell::new(),
            headers: parts.headers,
            route,
            connection: LazyCell::new(acquire),
        }
    }

    /// The link generator for this request, resolving each record's links against where its type is
    /// mounted. Cheap to build — a view over the base, the mount table, and the request.
    pub(crate) fn uri_generator(&self) -> CanonicalUriGenerator<'sch, '_, Adapter> {
        CanonicalUriGenerator::new(self.base_uri, self.mount_table, &self.route, &self.headers)
    }

    /// Takes the request body stream by value — a primary handler owns it to read or parse however
    /// it needs (a document, a multipart upload, a file), and `require_*` take it here too. Since a
    /// context is always built with a body, `None` means it was already consumed upstream, an
    /// internal invariant violation rather than a client fault — hence the 500.
    pub fn require_body(&mut self) -> Result<ByteStream, RoutingError> {
        self.body.take().ok_or(RoutingError::RequestBodyConsumed)
    }

    /// Tests the request for body content, probed once and cached.
    /// This attempts to read a single byte from the body stream and prepend it back afterwards,
    /// replacing the body stream but making the data it yields identical.
    /// Returns whether a byte was read, and thus the body carries content, or none was, and thus the
    /// body is empty.
    pub fn contains_body(&mut self) -> Result<bool, RoutingError> {
        if let Some(&present) = self.body_present.get() {
            return Ok(present);
        }

        let mut byte = 0u8;
        let mut body = self.require_body()?;
        let count = body
            .read(std::slice::from_mut(&mut byte))
            .map_err(|error| RoutingError::RequestBodyPeekFailed {
                message: error.to_string(),
            })?;

        if count == 0 {
            self.body = Some(body);
        } else {
            self.body = Some(Box::new(Cursor::new([byte]).take(count as u64).chain(body)));
        }

        Ok(*self.body_present.get_or_init(|| count != 0))
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

    pub fn route_parameters(&self) -> &RouteParameters<'sch, 'req> {
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
