#[cfg(test)]
mod tests;

use std::ops::{Deref, DerefMut};

use crate::{
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::{ForeignKeys, Identifier},
        composite::Composite,
        error::Error as DatabaseError,
        query_parameters::QueryParameters,
        record::Record,
        relationships::Relationship,
        schema::{IdentifierType, RelationshipDescriptor, RelationshipKind, Schema},
    },
    http_wrappers::{StatusCode, Uri},
    json_api::{
        document::Document, identifier::Identifier as JsonApiIdentifier,
        primary_content::PrimaryContent, relationship::Linkage, resource::Resource,
    },
    routing::{Error, PrimaryContext, ResourceResult, RouteParameters, responder::*},
    serialisation::factories::{Content, to_document},
};
use http::HeaderMap;
use itertools::Itertools;
use std::borrow::Cow;
use std::cell::LazyCell;
use std::collections::HashMap;

/// A query parsed lazily against the resource schema; unforced until first use, then the parsed
/// parameters or the parse failure. Boxed because the init closure captures the request's uri,
/// schema, and registry.
type LazyQueryParameters<'sch, 'req> = LazyCell<
    std::result::Result<QueryParameters<'sch, 'req>, DatabaseError>,
    Box<dyn FnOnce() -> std::result::Result<QueryParameters<'sch, 'req>, DatabaseError> + 'req>,
>;

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
        let registry = context.manager.registry();
        Self {
            schema,
            context,
            query_parameters: LazyCell::new(Box::new(move || {
                QueryParameters::parse(uri, schema, registry)
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
    pub fn query_parameters(
        &self,
    ) -> std::result::Result<&QueryParameters<'sch, 'req>, DatabaseError> {
        LazyCell::force(&self.query_parameters)
            .as_ref()
            .map_err(|error| error.clone())
    }

    /// Parses the streamed body into an optional document, straight off the stream. A bodyless
    /// request yields `None`; a JSON `null` likewise; malformed content surfaces as the parser's
    /// error.
    fn parse_body(&mut self) -> std::result::Result<Option<Document>, Error> {
        if !self.contains_body()? {
            return Ok(None);
        }

        let body = self.require_body()?;
        serde_json::from_reader(body).map_err(Into::into)
    }

    /// Parses the request body into a record validated against the resource schema.
    pub fn require_record(&mut self) -> std::result::Result<Record<'sch>, Error> {
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
                    let descriptor = schema.relationship(&name).ok_or_else(|| {
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
    pub fn require_resource(&mut self) -> std::result::Result<Resource, Error> {
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
    ) -> std::result::Result<Relationship, Error> {
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
    pub fn require_linkage(&mut self) -> std::result::Result<Linkage, Error> {
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
    fn require_identifier(resource: Resource) -> std::result::Result<JsonApiIdentifier, Error> {
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
    ) -> std::result::Result<Identifier, Error> {
        let schema = self.context.manager.registry().schema(schema)?;
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
    pub fn require_id(&self) -> std::result::Result<Identifier, Error> {
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

/// A controller's behaviour configuration: the knobs it exposes to shape how the framework serves
/// its resource. Expands as new hooks are added; today it governs only client-generated ids.
#[derive(Default)]
pub struct Configuration {
    /// Whether a create request may carry a client-generated id. When false the server assigns
    /// every id and a client-supplied id is refused with 403 Forbidden.
    pub accepts_client_ids: bool,
}

/// The behaviour served at a resource's endpoints. Every method defaults to the framework's
/// serving; an implementor overrides only the endpoints and configuration it customises.
pub trait ResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    /// This controller's behaviour configuration; override to opt out of the framework defaults.
    fn configuration(&self) -> Configuration {
        Configuration::default()
    }

    /// Resolves a route's required parameters to concrete, request-scoped values, for the router to
    /// render a link against. The default takes the `:id` parameter from `record` — the resource's
    /// identifier is always mounted as `:id`, regardless of the primary key's column name — and
    /// echoes every other parameter from the request's route parameters, omitting any it cannot
    /// resolve (an unresolved parameter leaves the link unrenderable). Override to resolve a
    /// parameter from the request headers instead.
    fn parameters_for_route<'req>(
        &self,
        record: &'req Record<'sch>,
        route: &'req RouteParameters,
        _headers: &'req HeaderMap,
        required_parameters: &[&'req str],
    ) -> HashMap<&'req str, Cow<'req, str>>
    where
        'sch: 'req,
    {
        required_parameters
            .iter()
            .filter_map(|&parameter| {
                let value = if parameter == "id" {
                    record.get_id().map(|id| match id {
                        Identifier::Integer(id) => Cow::Owned(id.to_string()),
                        Identifier::Text(id) => Cow::Borrowed(id.as_str()),
                    })
                } else {
                    route
                        .get(parameter)
                        .map(|value| Cow::Borrowed(value.as_ref()))
                };
                value.map(|value| (parameter, value))
            })
            .collect()
    }

    fn index<'req>(&self, context: ResourceContext<'sch, 'req, Adapter>) -> ResourceResult
    where
        'sch: 'req,
    {
        let parameters = context.query_parameters()?;
        let Composite { content, included } = context
            .store()?
            .fetch_collection(context.schema(), parameters)?;
        let document = to_document(&content, included, context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }

    fn show<'req>(&self, context: ResourceContext<'sch, 'req, Adapter>) -> ResourceResult
    where
        'sch: 'req,
    {
        let parameters = context.query_parameters()?;
        let id = context.require_id()?;
        let Composite { content, included } =
            context
                .store()?
                .fetch_record(context.schema(), id, parameters)?;
        let document = to_document(&content, included, context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }

    fn create<'req>(&self, mut context: ResourceContext<'sch, 'req, Adapter>) -> ResourceResult
    where
        'sch: 'req,
    {
        let record = context.require_record()?;

        if record.id.is_some() && !self.configuration().accepts_client_ids {
            return Err(Error::ClientGeneratedIdNotSupported {
                kind: record.schema.name().to_string(),
            }
            .into());
        }

        let parameters = context.query_parameters()?;
        let Composite { content, included } = context.store()?.create_record(record, parameters)?;
        let document = to_document(&content, included, context.uri(), &context.uri_generator())?;

        respond_with(StatusCode::CREATED, Some(document))
    }

    fn update<'req>(&self, mut context: ResourceContext<'sch, 'req, Adapter>) -> ResourceResult
    where
        'sch: 'req,
    {
        let record = context.require_record()?;
        let parameters = context.query_parameters()?;
        let Composite { content, included } = context.store()?.update_record(record, parameters)?;
        let document = to_document(&content, included, context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }

    fn delete<'req>(&self, context: ResourceContext<'sch, 'req, Adapter>) -> ResourceResult
    where
        'sch: 'req,
    {
        let id = context.require_id()?;
        context.store()?.delete_record(context.schema(), id)?;

        no_content()
    }

    fn linkage<'req>(
        &self,
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        let schema = context.schema();
        let descriptor = schema.relationship(relationship).ok_or_else(|| {
            DatabaseError::InvalidRelationshipAccess {
                schema: schema.name().to_string(),
                relationship: relationship.to_string(),
            }
        })?;
        let related_schema = context
            .context
            .manager
            .registry()
            .schema(descriptor.related.resource)?;

        let id = context.require_id()?;
        let store = context.store()?;
        let parent = store
            .fetch_record(schema, id, &QueryParameters::new(schema))?
            .content;

        let content: Content<'sch, 'req> = match descriptor.kind {
            RelationshipKind::HasMany => store
                .peek_related_collection(&parent, relationship)?
                .into_iter()
                .map(|id| JsonApiIdentifier::from((id, related_schema)))
                .collect::<Vec<_>>()
                .into(),
            RelationshipKind::BelongsTo | RelationshipKind::HasOne => store
                .peek_related_record(&parent, relationship)?
                .map(|id| JsonApiIdentifier::from((id, related_schema)))
                .into(),
        };

        let document = to_document(content, Vec::new(), context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }

    fn related<'req>(
        &self,
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        let schema = context.schema();
        let descriptor = schema.relationship(relationship).ok_or_else(|| {
            DatabaseError::InvalidRelationshipAccess {
                schema: schema.name().to_string(),
                relationship: relationship.to_string(),
            }
        })?;

        let registry = context.context.manager.registry();
        let related_schema = registry.schema(descriptor.related.resource)?;
        let store = context.store()?;

        let id = context.require_id()?;
        let parameters = QueryParameters {
            fields: [(schema.name(), [descriptor.related.keys.own].into())].into(),
            ..QueryParameters::new(schema)
        };
        let parent = store.fetch_record(schema, id, &parameters)?.content;

        let uri = context.uri();
        let related_parameters = QueryParameters::parse(uri, related_schema, registry)?;

        let generator = context.uri_generator();
        let document = match descriptor.kind {
            RelationshipKind::HasMany => {
                let Composite { content, included } =
                    store.fetch_related_collection(&parent, relationship, related_parameters)?;
                to_document(&content, included, uri, &generator)?
            }
            RelationshipKind::BelongsTo | RelationshipKind::HasOne => {
                match store.fetch_related_record(&parent, relationship, related_parameters)? {
                    Some(Composite { content, included }) => {
                        to_document(&content, included, uri, &generator)?
                    }
                    None => to_document(Content::Empty, Vec::new(), uri, &generator)?,
                }
            }
        };

        respond(Some(document))
    }

    fn link<'req>(
        &self,
        mut context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        let schema = context.schema();
        let descriptor = schema.relationship(relationship).ok_or_else(|| {
            DatabaseError::InvalidRelationshipAccess {
                schema: schema.name().to_string(),
                relationship: relationship.to_string(),
            }
        })?;
        let related_schema = context
            .context
            .manager
            .registry()
            .schema(descriptor.related.resource)?;

        let id = context.require_id()?;
        let linkage = context.require_linkage()?;
        let targets = match context.require_relationship(Some(linkage), descriptor)? {
            Relationship::HasMany(identifiers) => identifiers,
            Relationship::Empty => Vec::new(),
            Relationship::BelongsTo(_) | Relationship::HasOne(_) => {
                return Err(DatabaseError::MismatchedRelationshipKind {
                    schema: schema.name().to_string(),
                    relationship: relationship.to_string(),
                }
                .into());
            }
        };

        let store = context.store()?;
        let parent = store
            .fetch_record(schema, id, &QueryParameters::new(schema))?
            .content;

        let content: Content<'sch, 'req> = store
            .link_collection(parent, relationship, targets)?
            .into_iter()
            .map(|id| JsonApiIdentifier::from((id, related_schema)))
            .collect::<Vec<_>>()
            .into();

        let document = to_document(content, Vec::new(), context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }

    fn unlink<'req>(
        &self,
        mut context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        let schema = context.schema();
        let descriptor = schema.relationship(relationship).ok_or_else(|| {
            DatabaseError::InvalidRelationshipAccess {
                schema: schema.name().to_string(),
                relationship: relationship.to_string(),
            }
        })?;
        let related_schema = context
            .context
            .manager
            .registry()
            .schema(descriptor.related.resource)?;

        let id = context.require_id()?;
        let linkage = context.require_linkage()?;
        let targets = match context.require_relationship(Some(linkage), descriptor)? {
            Relationship::HasMany(identifiers) => identifiers,
            Relationship::Empty => Vec::new(),
            Relationship::BelongsTo(_) | Relationship::HasOne(_) => {
                return Err(DatabaseError::MismatchedRelationshipKind {
                    schema: schema.name().to_string(),
                    relationship: relationship.to_string(),
                }
                .into());
            }
        };

        let store = context.store()?;
        let parent = store
            .fetch_record(schema, id, &QueryParameters::new(schema))?
            .content;

        let content: Content<'sch, 'req> = store
            .unlink_collection(parent, relationship, targets)?
            .into_iter()
            .map(|id| JsonApiIdentifier::from((id, related_schema)))
            .collect::<Vec<_>>()
            .into();

        let document = to_document(content, Vec::new(), context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }

    fn relink<'req>(
        &self,
        mut context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> ResourceResult
    where
        'sch: 'req,
    {
        let schema = context.schema();
        let descriptor = schema.relationship(relationship).ok_or_else(|| {
            DatabaseError::InvalidRelationshipAccess {
                schema: schema.name().to_string(),
                relationship: relationship.to_string(),
            }
        })?;
        let related_schema = context
            .context
            .manager
            .registry()
            .schema(descriptor.related.resource)?;

        let id = context.require_id()?;
        let linkage = context.require_linkage()?;
        let target = context.require_relationship(Some(linkage), descriptor)?;

        let store = context.store()?;
        let parent = store
            .fetch_record(schema, id, &QueryParameters::new(schema))?
            .content;

        let content: Content<'sch, 'req> = match target {
            Relationship::BelongsTo(identifier) | Relationship::HasOne(identifier) => store
                .relink_record(parent, relationship, identifier)?
                .map(|id| JsonApiIdentifier::from((id, related_schema)))
                .into(),
            Relationship::HasMany(identifiers) => store
                .relink_collection(parent, relationship, identifiers)?
                .into_iter()
                .map(|id| JsonApiIdentifier::from((id, related_schema)))
                .collect::<Vec<_>>()
                .into(),
            Relationship::Empty => match descriptor.kind {
                RelationshipKind::HasMany => store
                    .relink_collection(parent, relationship, Vec::new())?
                    .into_iter()
                    .map(|id| JsonApiIdentifier::from((id, related_schema)))
                    .collect::<Vec<_>>()
                    .into(),
                RelationshipKind::BelongsTo | RelationshipKind::HasOne => store
                    .unlink_record(parent, relationship)?
                    .map(|id| JsonApiIdentifier::from((id, related_schema)))
                    .into(),
            },
        };

        let document = to_document(content, Vec::new(), context.uri(), &context.uri_generator())?;

        respond(Some(document))
    }
}

/// A controller that customises nothing — every endpoint uses the framework default.
#[derive(Default)]
pub struct DefaultController;

impl<'sch, Adapter: AdapterInterface + 'sch> ResourceController<'sch, Adapter>
    for DefaultController
{
}
