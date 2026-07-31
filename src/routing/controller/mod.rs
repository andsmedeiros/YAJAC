#[cfg(test)]
mod tests;

use std::ops::{Deref, DerefMut};

use crate::{
    core::factories::{Content, to_document},
    database::{
        adapters::Adapter as AdapterInterface,
        attributes::Identifier,
        composite::Composite,
        error::Error as DatabaseError,
        query_parameters::QueryParameters,
        record::Record,
        relationships::Relationship,
        schema::{IdentifierType, RelationshipDescriptor, RelationshipKind, Schema},
    },
    http_wrappers::Uri,
    json_api::{
        identifier::Identifier as JsonApiIdentifier, relationship::Linkage, resource::Resource,
    },
    routing::{
        Context, DefaultUriGenerator, Error, Result, error::ClientGeneratedIdNotSupportedError,
        responder::*,
    },
};
use http::StatusCode;

/// A request narrowed to a single resource: the resource's schema paired with the
/// routing context. It lends the context's request operations already bound to that
/// schema, so controller handlers never thread the schema through by hand.
pub struct ResourceContext<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> {
    schema: &'sch Schema<'sch>,
    context: Context<'sch, 'req, Adapter>,
}

impl<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch> ResourceContext<'sch, 'req, Adapter> {
    pub fn new(schema: &'sch Schema<'sch>, context: Context<'sch, 'req, Adapter>) -> Self {
        Self { schema, context }
    }

    pub fn schema(&self) -> &'sch Schema<'sch> {
        self.schema
    }

    pub fn uri(&self) -> &'req Uri {
        self.context.uri
    }

    /// Parses the request body into a record validated against the resource schema.
    pub fn require_record(&mut self) -> std::result::Result<Record<'sch>, Error> {
        self.context.require_record(self.schema)
    }

    /// Lazily parses the query string against the resource schema.
    pub fn query_parameters(
        &self,
    ) -> std::result::Result<&QueryParameters<'sch, 'req>, DatabaseError> {
        self.context.query_parameters(self.schema)
    }

    pub fn require_resource(&mut self) -> std::result::Result<Resource, Error> {
        self.context.require_resource(self.schema)
    }

    pub fn require_relationship(
        &self,
        linkage: Option<Linkage>,
        descriptor: &RelationshipDescriptor<'sch>,
    ) -> std::result::Result<Relationship, Error> {
        self.context
            .require_relationship(linkage, descriptor, self.schema)
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
    type Target = Context<'sch, 'req, Adapter>;

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

    fn index<'req>(&self, context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let parameters = context.query_parameters()?;
        let Composite { content, included } = context
            .store()?
            .fetch_collection(context.schema(), parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn show<'req>(&self, context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let parameters = context.query_parameters()?;
        let id = context.require_id()?;
        let Composite { content, included } =
            context
                .store()?
                .fetch_record(context.schema(), id, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn create<'req>(&self, mut context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let record = context.require_record()?;

        if record.id.is_some() && !self.configuration().accepts_client_ids {
            return Err(ClientGeneratedIdNotSupportedError.into());
        }

        let parameters = context.query_parameters()?;
        let Composite { content, included } = context.store()?.create_record(record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond_with(StatusCode::CREATED, Some(document))
    }

    fn update<'req>(&self, mut context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        let record = context.require_record()?;
        let parameters = context.query_parameters()?;
        let Composite { content, included } = context.store()?.update_record(record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn delete<'req>(&self, context: ResourceContext<'sch, 'req, Adapter>) -> Result
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
    ) -> Result
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

        let document = to_document(
            content,
            Vec::new(),
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn related<'req>(
        &self,
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
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

        let generator = DefaultUriGenerator::default();
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
    ) -> Result
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

        let document = to_document(
            content,
            Vec::new(),
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn unlink<'req>(
        &self,
        mut context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
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

        let document = to_document(
            content,
            Vec::new(),
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond(Some(document))
    }

    fn relink<'req>(
        &self,
        mut context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
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

        let document = to_document(
            content,
            Vec::new(),
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

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
