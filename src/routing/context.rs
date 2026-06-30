use super::Request;
use crate::database::attributes::{ForeignKeys, Identifier};
use crate::database::error::Error;
use crate::database::record::Record;
use crate::database::relationships::Relationship;
use crate::database::schema::{IdentifierType, Relationship as SchemaRelationship, TableSchema};
use crate::json_api::identifier::Identifier as JsonApiIdentifier;
use crate::json_api::relationship::Linkage;
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
use itertools::Itertools;
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

    pub fn require_resource(&mut self, schema: &TableSchema) -> Result<Resource, RoutingError> {
        let document = self.body.take().ok_or_else(|| {
            RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "MissingBody",
                "the request requires a body",
            )
        })?;

        let PrimaryContent::Record { data } = document.content else {
            return Err(RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "InvalidDocument",
                "the request body must contain a single resource object",
            ));
        };
        let resource = *data;

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

    pub fn require_record(
        &mut self,
        schema: &'sch TableSchema<'sch>,
    ) -> Result<Record<'sch>, RoutingError> {
        let resource = self.require_resource(schema)?;
        let record = Record {
            schema,
            id: match resource.identifier {
                ResourceIdentifier::New { .. } => None,
                identifier => Some(self.materialise_id(identifier, schema.name)?),
            },
            attributes: resource
                .attributes
                .unwrap_or_default()
                .into_iter()
                .map(|(name, value)| {
                    if !schema.has_attribute(&name) {
                        return Err(RoutingError::new(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            "UnknownAttribute",
                            format!(
                                "Unknown attribute '{name}' for resource type '{}'",
                                schema.name
                            ),
                        ));
                    }

                    Ok((name, serde_json::from_value(value)?))
                })
                .try_collect::<_, _, RoutingError>()?,
            relationships: resource
                .relationships
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(name, relationship)| {
                    (|| -> Result<Option<(_, _)>, RoutingError> {
                        let &(name, ref descriptor) = schema
                            .relationships
                            .iter()
                            .find(|(n, _)| *n == name.as_str())
                            .ok_or_else(|| Error::ResourceValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: name,
                                message: "Attempted to attach unknown relationship".to_string(),
                            })?;
                        let result = match (relationship.data, descriptor) {
                            (
                                Some(Linkage::ToOne(identifier)),
                                SchemaRelationship::HasOne(related),
                            ) => Some(Relationship::HasOne(
                                self.materialise_id(identifier, related.resource)?,
                            )),
                            (
                                Some(Linkage::ToOne(identifier)),
                                SchemaRelationship::BelongsTo(related),
                            ) => Some(Relationship::BelongsTo(
                                self.materialise_id(identifier, related.resource)?,
                            )),
                            (Some(Linkage::ToMany(ids)), SchemaRelationship::HasMany(related)) => {
                                Some(Relationship::HasMany(
                                    ids.into_iter()
                                        .map(|identifier| {
                                            self.materialise_id(identifier, related.resource)
                                        })
                                        .try_collect()?,
                                ))
                            }

                            (None | Some(Linkage::Empty), _) => Some(Relationship::Empty),
                            _ => Err(Error::ResourceValidationFailure {
                                schema: schema.name.to_string(),
                                attribute: name.to_string(),
                                message: "Attempted to attach relationship with wrong linkage"
                                    .to_string(),
                            })?,
                        }
                        .map(|relationship| (name, relationship));

                        Ok(result)
                    })()
                    .transpose()
                })
                .try_collect()?,
            foreign_keys: ForeignKeys::new(),
        };

        Ok(record)
    }

    /// Resolves a request-supplied identifier into a typed primary key. As it validates client
    /// input, every failure is a `routing::Error`: a `New` (`lid`) identifier has no id to resolve,
    /// a mismatched type cannot name the expected resource, and a non-integer id cannot be parsed.
    fn materialise_id(
        &self,
        identifier: JsonApiIdentifier,
        schema: &str,
    ) -> Result<Identifier, RoutingError> {
        let schema = self.registry.schema(schema)?;
        let identifier = match identifier {
            JsonApiIdentifier::Existing { kind, id } if kind.as_str() == schema.name => id,
            JsonApiIdentifier::New { .. } => {
                return Err(RoutingError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "UnresolvableLinkage",
                    "A relationship linkage must reference an existing resource by id",
                ));
            }
            _ => {
                return Err(RoutingError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "RelationshipTypeMismatch",
                    "A relationship linkage references the wrong resource type",
                ));
            }
        };

        match schema.primary_key.kind {
            IdentifierType::Text => Ok(Identifier::Text(identifier)),
            IdentifierType::Integer => identifier.parse().map(Identifier::Integer).map_err(|_| {
                RoutingError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "InvalidIdentifier",
                    format!("Identifier '{identifier}' is not a valid integer"),
                )
            }),
        }
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
