use super::Request;
use crate::database::error::Error;
use crate::database::schema::TableSchema;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, connection::Connection as ConnectionInterface,
        pool::Pool as PoolInterface, query_parameters::QueryParameters, registry::Registry,
        store::Store,
    },
    http_wrappers::{StatusCode, Uri},
    json_api::{
        document::Document, identifier::Identifier as ResourceIdentifier,
        primary_content::PrimaryContent, resource::Resource,
    },
    routing::{Error as RoutingError, RouteParameters},
};
use http::HeaderMap;
use std::cell::OnceCell;

type Handle<'req, Adapter> = <<Adapter as AdapterInterface>::Pool as PoolInterface>::Handle<'req>;

pub struct Context<'sch, 'req, Adapter: AdapterInterface>
where
    'sch: 'req,
{
    pub registry: &'sch Registry<'sch, Adapter>,
    pub uri: &'req Uri,
    body: Option<Document>,
    headers: HeaderMap,
    route: RouteParameters,
    query: OnceCell<QueryParameters<'sch, 'req>>,
    connection: OnceCell<Handle<'req, Adapter>>,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface> Context<'sch, 'req, Adapter> {
    /// Builds a context from the request, harvesting its owned body and headers and discarding the
    /// rest; `uri` is lent separately so the borrowing query parameters can reference it.
    pub fn from_request(
        registry: &'sch Registry<'sch, Adapter>,
        uri: &'req Uri,
        route: RouteParameters,
        request: Request,
    ) -> Self {
        let (parts, body) = request.into_parts();

        Self {
            registry,
            uri,
            body,
            headers: parts.headers,
            route,
            query: OnceCell::new(),
            connection: OnceCell::new(),
        }
    }

    /// Lazily acquires the request connection from the pool and lends it as a shared reference.
    pub fn connection(&self) -> Result<&Adapter::Connection, Error> {
        match self.connection.get() {
            Some(handle) => Ok(handle),
            None => {
                let handle = self.registry.acquire()?;
                Ok(self.connection.get_or_init(|| handle))
            }
        }
    }

    pub fn table(&self, name: &str) -> Result<Adapter::Table<'sch, '_>, Error> {
        self.registry.table(name, self.connection()?)
    }

    pub fn store(&self) -> Result<Store<'sch, '_, Adapter>, Error> {
        Ok(Store::new(self.registry, self.connection()?))
    }

    pub fn require_resource(&self, schema: &TableSchema) -> Result<&Resource, RoutingError> {
        let document = self.body.as_ref().ok_or_else(|| {
            RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "MissingBody",
                "the request requires a body",
            )
        })?;

        let PrimaryContent::Record { data } = &document.content else {
            return Err(RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "InvalidDocument",
                "the request body must contain a single resource object",
            ));
        };
        let resource = data.as_ref();

        let (kind, id) = match &resource.identifier {
            ResourceIdentifier::Existing { kind, id } => (kind.as_str(), Some(id)),
            ResourceIdentifier::New { kind, .. } => (kind.as_str(), None),
        };

        if kind != schema.name {
            return Err(RoutingError::new(
                StatusCode::CONFLICT,
                "ResourceTypeMismatch",
                format!("expected resource type '{}', got '{kind}'", schema.name),
            ));
        }

        if let Some(expected) = self.route.get("id") {
            match id {
                Some(sent) if sent != expected => {
                    return Err(RoutingError::new(
                        StatusCode::CONFLICT,
                        "ResourceIdMismatch",
                        format!("resource id '{sent}' does not match endpoint id '{expected}'"),
                    ));
                }
                None => {
                    return Err(RoutingError::new(
                        StatusCode::CONFLICT,
                        "ResourceIdMissing",
                        format!("the resource is missing the id required by endpoint '{expected}'"),
                    ));
                }
                _ => {}
            }
        }

        Ok(resource)
    }

    /// Runs `operation` inside a transaction on the request connection.
    pub fn transaction<R>(
        &self,
        operation: impl FnOnce(&Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        self.connection()?.transaction(|| operation(self))
    }

    pub fn body(&self) -> &Option<Document> {
        &self.body
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn route_parameters(&self) -> &RouteParameters {
        &self.route
    }

    pub fn query_parameters(
        &self,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<&QueryParameters<'sch, 'req>, Error> {
        match self.query.get() {
            Some(parameters) => Ok(parameters),
            None => {
                let parameters = QueryParameters::parse(self.uri, schema, self.registry)?;
                Ok(self.query.get_or_init(|| parameters))
            }
        }
    }
}
