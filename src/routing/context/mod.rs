#[cfg(test)]
mod tests;

use super::PrimaryRequest;
use crate::{
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::{ForeignKeys, Identifier},
        connection_manager::ConnectionManager,
        error::Error as DatabaseError,
        query_parameters::QueryParameters,
        record::Record,
        relationships::Relationship,
        schema::{IdentifierType, RelationshipDescriptor, RelationshipKind, Schema},
        store::Store,
    },
    http_wrappers::Uri,
    json_api::{
        document::Document, identifier::Identifier as JsonApiIdentifier,
        primary_content::PrimaryContent, relationship::Linkage, resource::Resource,
    },
    routing::{BaseUri, Error, MountTable, RouteParameters},
    serialisation::{ByteStream, uri_generator::CanonicalUriGenerator},
};
use http::HeaderMap;
use itertools::Itertools;
use std::cell::{LazyCell, OnceCell};
use std::io::{Cursor, Read};
use std::ops::{Deref, DerefMut};

/// A lazily-acquired request connection: unforced until first use, then the pooled handle or the
/// failure that acquiring it produced. Boxed because the init closure captures the manager.
type LazyConnection<'sch, Adapter> = LazyCell<
    Result<<Adapter as AdapterInterface>::Connection, Error>,
    Box<dyn FnOnce() -> Result<<Adapter as AdapterInterface>::Connection, Error> + 'sch>,
>;

/// A query parsed lazily against the resource schema; unforced until first use, then the parsed
/// parameters or the parse failure. Boxed because the init closure captures the request's uri,
/// schema, and registry.
type LazyQueryParameters<'sch, 'req> = LazyCell<
    Result<QueryParameters<'sch, 'req>, Error>,
    Box<dyn FnOnce() -> Result<QueryParameters<'sch, 'req>, Error> + 'req>,
>;

/// The raw byte tier's request context: the request head and a streamed body, schema-oblivious. The
/// crossing upgrades it to a `ResourceContext` at a resource boundary; document-shaped request
/// operations live there.
pub struct PrimaryContext<'sch, 'req, Adapter: AdapterInterface>
where
    'sch: 'req,
{
    pub connection_manager: &'sch ConnectionManager<'sch, Adapter>,
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
        connection_manager: &'sch ConnectionManager<'sch, Adapter>,
        base_uri: &'req BaseUri<'sch>,
        mount_table: &'req MountTable<'sch, Adapter>,
        uri: &'req Uri,
        route: RouteParameters<'sch, 'req>,
        request: PrimaryRequest,
    ) -> Self {
        let (parts, body) = request.into_parts();
        let acquire: Box<dyn FnOnce() -> Result<Adapter::Connection, Error> + 'sch> =
            Box::new(move || connection_manager.acquire().map_err(Into::into));

        Self {
            connection_manager,
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
    pub fn require_body(&mut self) -> Result<ByteStream, Error> {
        self.body.take().ok_or(Error::RequestBodyConsumed)
    }

    /// Tests the request for body content, probed once and cached.
    /// This attempts to read a single byte from the body stream and prepend it back afterwards,
    /// replacing the body stream but making the data it yields identical.
    /// Returns whether a byte was read, and thus the body carries content, or none was, and thus the
    /// body is empty.
    pub fn contains_body(&mut self) -> Result<bool, Error> {
        if let Some(&present) = self.body_present.get() {
            return Ok(present);
        }

        let mut byte = 0u8;
        let mut body = self.require_body()?;
        let count = body
            .read(std::slice::from_mut(&mut byte))
            .map_err(|error| Error::RequestBodyPeekFailed {
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
        self.connection_manager
            .table(name, self.connection()?)
            .map_err(Into::into)
    }

    pub fn store(&self) -> Result<Store<'sch, '_, Adapter>, Error> {
        Ok(Store::new(self.connection_manager, self.connection()?))
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
        QueryParameters::parse(self.uri, schema, self.connection_manager.registry())
            .map_err(Into::into)
    }
}

/// A request narrowed to a single resource: the resource's schema paired with the
/// routing context. It lends the context's request operations already bound to that
/// schema, so controller handlers never thread the schema through by hand.
pub struct ResourceContext<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> {
    schema: &'sch Schema<'sch>,
    context: PrimaryContext<'sch, 'req, Adapter>,
    query_parameters: LazyQueryParameters<'sch, 'req>,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> ResourceContext<'sch, 'req, Adapter> {
    pub fn new(schema: &'sch Schema<'sch>, context: PrimaryContext<'sch, 'req, Adapter>) -> Self {
        let uri = context.uri;
        let registry = context.connection_manager.registry();
        Self {
            schema,
            context,
            query_parameters: LazyCell::new(Box::new(move || {
                QueryParameters::parse(uri, schema, registry).map_err(Into::into)
            })),
        }
    }

    pub fn schema(&self) -> &'sch Schema<'sch> {
        self.schema
    }

    pub fn uri(&self) -> &'req Uri {
        self.context.uri
    }

    /// The query parsed against the resource schema, parsed once on first access.
    pub fn query_parameters(&self) -> Result<&QueryParameters<'sch, 'req>, Error> {
        LazyCell::force(&self.query_parameters)
            .as_ref()
            .map_err(|error| error.clone())
    }

    /// Parses the streamed body into an optional document, straight off the stream. A bodyless
    /// request yields `None`; a JSON `null` likewise; malformed content surfaces as the parser's
    /// error.
    fn parse_body(&mut self) -> Result<Option<Document>, Error> {
        if !self.contains_body()? {
            return Ok(None);
        }

        let body = self.require_body()?;
        serde_json::from_reader(body).map_err(Into::into)
    }

    /// Parses the request body into a record validated against the resource schema.
    pub fn require_record(&mut self) -> Result<Record<'sch>, Error> {
        let schema = self.schema;
        let resource = self.require_resource()?;
        let record = Record {
            schema,
            id: match resource.identifier {
                JsonApiIdentifier::New { .. } => None,
                identifier => Some(self.materialise_id(identifier, schema.name())?),
            },
            attributes: resource
                .attributes
                .unwrap_or_default()
                .into_iter()
                .map(|(name, value)| {
                    let column =
                        schema
                            .attribute(&name)
                            .ok_or_else(|| Error::UnknownAttribute {
                                kind: schema.name().to_string(),
                                attribute: name.clone(),
                            })?;

                    Ok((column.name, serde_json::from_value(value)?))
                })
                .try_collect::<_, _, Error>()?,
            relationships: resource
                .relationships
                .unwrap_or_default()
                .into_iter()
                .map(|(name, relationship)| {
                    let descriptor = schema.relationship(&name).ok_or({
                        DatabaseError::ResourceValidationFailure {
                            schema: schema.name().to_string(),
                            attribute: name,
                            message: "Attempted to attach unknown relationship".to_string(),
                        }
                    })?;

                    Ok((
                        descriptor.name,
                        self.require_relationship(relationship.data, descriptor)?,
                    ))
                })
                .try_collect::<_, _, Error>()?,
            foreign_keys: ForeignKeys::new(),
        };

        Ok(record)
    }

    /// Extracts the request body as a single resource object, validating its type and — at a
    /// targeted endpoint — its id against the `:id` route parameter.
    pub fn require_resource(&mut self) -> Result<Resource, Error> {
        let schema = self.schema;
        let document = self.parse_body()?.ok_or(Error::MissingResourceBody)?;

        let resource = match document.content {
            PrimaryContent::Record { data } => *data,
            PrimaryContent::Errors { .. } => return Err(Error::ErrorDocumentSubmitted),
            PrimaryContent::Collection { .. } | PrimaryContent::Empty { .. } => {
                return Err(Error::PrimaryDataIsNotAResource);
            }
        };

        let (kind, id) = match &resource.identifier {
            JsonApiIdentifier::Existing { kind, id } => (kind.as_str(), Some(id)),
            JsonApiIdentifier::New { kind, .. } => (kind.as_str(), None),
        };

        if kind != schema.name() {
            return Err(Error::ResourceTypeMismatch {
                expected: schema.name().to_string(),
                actual: kind.to_string(),
            });
        }

        if let Some(expected) = self.route_parameters().get("id") {
            match id {
                Some(sent) if sent != expected => {
                    return Err(Error::ResourceIdMismatch {
                        expected: expected.to_string(),
                        actual: sent.to_string(),
                    });
                }
                None => {
                    return Err(Error::ResourceIdMissing {
                        expected: expected.to_string(),
                    });
                }
                _ => {}
            }
        }

        Ok(resource)
    }

    /// Resolves request-supplied linkage against the relationship it targets, materialising its
    /// identifiers into the typed keys the record layer stores. Absent and explicitly null linkage
    /// alike clear the relationship; linkage whose cardinality contradicts the relationship's
    /// direction is rejected.
    pub fn require_relationship(
        &self,
        linkage: Option<Linkage>,
        descriptor: &RelationshipDescriptor<'sch>,
    ) -> Result<Relationship, Error> {
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
                Err(DatabaseError::ResourceValidationFailure {
                    schema: self.schema.name().to_string(),
                    attribute: descriptor.name.to_string(),
                    message: "Attempted to attach relationship with wrong linkage".to_string(),
                })?
            }
        };

        Ok(relationship)
    }

    /// Extracts the request body as relationship linkage, the counterpart of `require_resource` for
    /// the relationship-endpoint family. Each resource object must be a bare identifier: carrying
    /// attributes, relationships, or links makes it a resource rather than linkage and is rejected.
    /// Type and id validation against the target resource is deferred to materialisation.
    pub fn require_linkage(&mut self) -> Result<Linkage, Error> {
        let document = self.parse_body()?.ok_or(Error::MissingLinkageBody)?;

        match document.content {
            PrimaryContent::Empty { .. } => Ok(Linkage::Empty),
            PrimaryContent::Record { data } => Ok(Linkage::ToOne(Self::require_identifier(*data)?)),
            PrimaryContent::Collection { data } => Ok(Linkage::ToMany(
                data.into_iter()
                    .map(Self::require_identifier)
                    .try_collect()?,
            )),
            PrimaryContent::Errors { .. } => Err(Error::ErrorDocumentSubmitted),
        }
    }

    /// Unwraps a resource object into its identifier, asserting it is a resource identifier object
    /// — no attributes, relationships, or links. Meta is permitted and discarded.
    fn require_identifier(resource: Resource) -> Result<JsonApiIdentifier, Error> {
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
            Err(Error::InvalidLinkage)
        }
    }

    /// Resolves a request-supplied identifier into a typed primary key. As it validates client
    /// input, every failure is a routing error: a `New` (`lid`) identifier has no id to resolve, a
    /// mismatched type cannot name the expected resource, and a non-integer id cannot be parsed.
    fn materialise_id(
        &self,
        identifier: JsonApiIdentifier,
        schema: &str,
    ) -> Result<Identifier, Error> {
        let schema = self.context.connection_manager.registry().schema(schema)?;
        let identifier = match identifier {
            JsonApiIdentifier::Existing { kind, id } if kind.as_str() == schema.name() => id,
            JsonApiIdentifier::New { .. } => return Err(Error::UnresolvableIdentifier),
            JsonApiIdentifier::Existing { kind, .. } => {
                return Err(Error::IdentifierTypeMismatch {
                    expected: schema.name().to_string(),
                    actual: kind,
                });
            }
        };

        match schema.primary_key().kind {
            IdentifierType::Text => Ok(Identifier::Text(identifier)),
            IdentifierType::Integer => identifier.parse().map(Identifier::Integer).map_err(|_| {
                Error::InvalidIntegerIdentifier {
                    id: identifier.clone(),
                }
            }),
        }
    }

    /// Resolves the endpoint's `:id` route parameter into a typed primary key.
    pub fn require_id(&self) -> Result<Identifier, Error> {
        let parameters = self.route_parameters();
        let identifier = match self.schema.primary_key().kind {
            IdentifierType::Text => Identifier::Text(parameters.require_as("id")?),
            IdentifierType::Integer => Identifier::Integer(parameters.require_as("id")?),
        };

        Ok(identifier)
    }
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> Deref
    for ResourceContext<'sch, 'req, Adapter>
{
    type Target = PrimaryContext<'sch, 'req, Adapter>;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> DerefMut
    for ResourceContext<'sch, 'req, Adapter>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}
