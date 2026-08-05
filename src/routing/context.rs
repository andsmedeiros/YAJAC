use super::Request;
use crate::database::attributes::{ForeignKeys, Identifier};
use crate::database::error::Error;
use crate::database::record::Record;
use crate::database::relationships::Relationship;
use crate::database::schema::{IdentifierType, RelationshipDescriptor, RelationshipKind, Schema};
use crate::json_api::identifier::Identifier as JsonApiIdentifier;
use crate::json_api::relationship::Linkage;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface, connection::Connection as ConnectionInterface,
        connection_manager::ConnectionManager, query_parameters::QueryParameters, store::Store,
    },
    http_wrappers::{StatusCode, Uri},
    json_api::{
        document::Document, identifier::Identifier as ResourceIdentifier,
        primary_content::PrimaryContent, resource::Resource,
    },
    routing::{BaseUri, Error as RoutingError, MountTable, RouteParameters},
    serialisation::uri_generator::UriGenerator,
};
use http::HeaderMap;
use itertools::Itertools;
use std::cell::LazyCell;

/// A lazily-acquired request connection: unforced until first use, then the pooled handle or the
/// failure that acquiring it produced. Boxed because the init closure captures the manager.
type LazyConnection<'sch, Adapter> = LazyCell<
    Result<<Adapter as AdapterInterface>::Connection, Error>,
    Box<dyn FnOnce() -> Result<<Adapter as AdapterInterface>::Connection, Error> + 'sch>,
>;

pub struct PrimaryContext<'sch, 'req, Adapter: AdapterInterface>
where
    'sch: 'req,
{
    pub manager: &'sch ConnectionManager<'sch, Adapter>,
    pub uri: &'req Uri,
    base_uri: &'req BaseUri<'sch>,
    mount_table: &'req MountTable<'sch, Adapter>,
    body: Option<Document>,
    headers: HeaderMap,
    route: RouteParameters,
    connection: LazyConnection<'sch, Adapter>,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface> PrimaryContext<'sch, 'req, Adapter> {
    /// Builds a context from the request, harvesting its owned body and headers and discarding the
    /// rest; `uri` is lent separately so the borrowing query parameters can reference it.
    pub(crate) fn from_request(
        manager: &'sch ConnectionManager<'sch, Adapter>,
        base_uri: &'req BaseUri<'sch>,
        mount_table: &'req MountTable<'sch, Adapter>,
        uri: &'req Uri,
        route: RouteParameters,
        request: Request,
    ) -> Self {
        let (parts, body) = request.into_parts();
        let acquire: Box<dyn FnOnce() -> Result<Adapter::Connection, Error> + 'sch> =
            Box::new(move || manager.acquire());

        Self {
            manager,
            uri,
            base_uri,
            mount_table,
            body,
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

    pub fn require_resource(&mut self, schema: &Schema) -> Result<Resource, RoutingError> {
        let document = self.body.take().ok_or_else(|| {
            RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "MissingBody",
                "This request requires a body containing a resource object",
            )
        })?;

        let PrimaryContent::Record { data } = document.content else {
            return Err(RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "InvalidDocument",
                "The request body must contain a single resource object as its primary data",
            ));
        };
        let resource = *data;

        let (kind, id) = match &resource.identifier {
            ResourceIdentifier::Existing { kind, id } => (kind.as_str(), Some(id)),
            ResourceIdentifier::New { kind, .. } => (kind.as_str(), None),
        };

        if kind != schema.name() {
            return Err(RoutingError::new(
                StatusCode::CONFLICT,
                "ResourceTypeMismatch",
                format!(
                    "The resource type '{kind}' does not match the '{}' resource served at this endpoint",
                    schema.name()
                ),
            ));
        }

        if let Some(expected) = self.route.get("id") {
            match id {
                Some(sent) if sent != expected => {
                    return Err(RoutingError::new(
                        StatusCode::CONFLICT,
                        "ResourceIdMismatch",
                        format!(
                            "The resource id '{sent}' does not match the id '{expected}' targeted by this endpoint"
                        ),
                    ));
                }
                None => {
                    return Err(RoutingError::new(
                        StatusCode::CONFLICT,
                        "ResourceIdMissing",
                        format!(
                            "The submitted resource must carry the id '{expected}' targeted by this endpoint"
                        ),
                    ));
                }
                _ => {}
            }
        }

        Ok(resource)
    }

    pub fn require_record(
        &mut self,
        schema: &'sch Schema<'sch>,
    ) -> Result<Record<'sch>, RoutingError> {
        let resource = self.require_resource(schema)?;
        let record = Record {
            schema,
            id: match resource.identifier {
                ResourceIdentifier::New { .. } => None,
                identifier => Some(self.materialise_id(identifier, schema.name())?),
            },
            attributes: resource
                .attributes
                .unwrap_or_default()
                .into_iter()
                .map(|(name, value)| {
                    let column = schema.attribute(&name).ok_or_else(|| {
                        RoutingError::new(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            "UnknownAttribute",
                            format!(
                                "The resource type '{}' has no attribute named '{name}'",
                                schema.name()
                            ),
                        )
                    })?;

                    Ok((column.name, serde_json::from_value(value)?))
                })
                .try_collect::<_, _, RoutingError>()?,
            relationships: resource
                .relationships
                .unwrap_or_default()
                .into_iter()
                .map(|(name, relationship)| {
                    let descriptor = schema.relationship(&name).ok_or_else(|| {
                        Error::ResourceValidationFailure {
                            schema: schema.name().to_string(),
                            attribute: name,
                            message: "Attempted to attach unknown relationship".to_string(),
                        }
                    })?;

                    Ok((
                        descriptor.name,
                        self.require_relationship(relationship.data, descriptor, schema)?,
                    ))
                })
                .try_collect::<_, _, RoutingError>()?,
            foreign_keys: ForeignKeys::new(),
        };

        Ok(record)
    }

    /// Resolves request-supplied linkage against the relationship it targets, materialising its
    /// identifiers into the typed keys the record layer stores. Absent and explicitly null linkage
    /// alike clear the relationship; linkage whose cardinality contradicts the relationship's
    /// direction is rejected.
    pub fn require_relationship(
        &self,
        linkage: Option<Linkage>,
        descriptor: &RelationshipDescriptor<'sch>,
        schema: &Schema,
    ) -> Result<Relationship, RoutingError> {
        let related = &descriptor.related;
        let relationship = match (linkage, descriptor.kind) {
            (Some(Linkage::ToOne(identifier)), RelationshipKind::HasOne) => {
                Relationship::HasOne(self.materialise_id(identifier, related.resource)?)
            }
            (Some(Linkage::ToOne(identifier)), RelationshipKind::BelongsTo) => {
                Relationship::BelongsTo(self.materialise_id(identifier, related.resource)?)
            }
            (Some(Linkage::ToMany(ids)), RelationshipKind::HasMany) => Relationship::HasMany(
                ids.into_iter()
                    .map(|identifier| self.materialise_id(identifier, related.resource))
                    .try_collect()?,
            ),
            (None | Some(Linkage::Empty), _) => Relationship::Empty,

            (Some(Linkage::ToOne(_)), RelationshipKind::HasMany)
            | (Some(Linkage::ToMany(_)), RelationshipKind::HasOne | RelationshipKind::BelongsTo) => {
                Err(Error::ResourceValidationFailure {
                    schema: schema.name().to_string(),
                    attribute: descriptor.name.to_string(),
                    message: "Attempted to attach relationship with wrong linkage".to_string(),
                })?
            }
        };

        Ok(relationship)
    }

    /// Extracts the request body as relationship linkage, the counterpart of `require_resource`
    /// for the relationship-endpoint family. Each resource object must be a bare identifier:
    /// carrying attributes, relationships, or links makes it a resource rather than linkage and is
    /// rejected. Type and id validation against the target resource is deferred to materialisation,
    /// where the relationship descriptor is known.
    pub fn require_linkage(&mut self) -> Result<Linkage, RoutingError> {
        let document = self.body.take().ok_or_else(|| {
            RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "MissingBody",
                "This request requires a body containing relationship linkage",
            )
        })?;

        match document.content {
            PrimaryContent::Empty { .. } => Ok(Linkage::Empty),
            PrimaryContent::Record { data } => Ok(Linkage::ToOne(Self::require_identifier(*data)?)),
            PrimaryContent::Collection { data } => Ok(Linkage::ToMany(
                data.into_iter()
                    .map(Self::require_identifier)
                    .try_collect()?,
            )),
            PrimaryContent::Errors { .. } => Err(RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "InvalidDocument",
                "The request body must contain relationship linkage as its primary data",
            )),
        }
    }

    /// Unwraps a resource object into its identifier, asserting it is a resource identifier object
    /// — no attributes, relationships, or links. Meta is permitted and discarded.
    fn require_identifier(resource: Resource) -> Result<ResourceIdentifier, RoutingError> {
        if let Resource {
            identifier,
            attributes: None,
            relationships: None,
            links: None,
            ..
        } = resource
        {
            Ok(identifier)
        } else {
            Err(RoutingError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "InvalidLinkage",
                "Relationship linkage must contain resource identifier objects, not full resources",
            ))
        }
    }

    /// Resolves a request-supplied identifier into a typed primary key. As it validates client
    /// input, every failure is a `routing::Error`: a `New` (`lid`) identifier has no id to resolve,
    /// a mismatched type cannot name the expected resource, and a non-integer id cannot be parsed.
    fn materialise_id(
        &self,
        identifier: JsonApiIdentifier,
        schema: &str,
    ) -> Result<Identifier, RoutingError> {
        let schema = self.manager.registry().schema(schema)?;
        let identifier = match identifier {
            JsonApiIdentifier::Existing { kind, id } if kind.as_str() == schema.name() => id,
            JsonApiIdentifier::New { .. } => {
                return Err(RoutingError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "UnresolvableLinkage",
                    "Relationship linkage must reference an existing resource by its id",
                ));
            }
            _ => {
                return Err(RoutingError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "RelationshipTypeMismatch",
                    "Relationship linkage references a resource of the wrong type for this relationship",
                ));
            }
        };

        match schema.primary_key().kind {
            IdentifierType::Text => Ok(Identifier::Text(identifier)),
            IdentifierType::Integer => identifier.parse().map(Identifier::Integer).map_err(|_| {
                RoutingError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "InvalidIdentifier",
                    format!("The id '{identifier}' is not a valid integer identifier"),
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
