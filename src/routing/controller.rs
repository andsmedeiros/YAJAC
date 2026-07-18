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
    routing::{Context, DefaultUriGenerator, Error, Result, responder::*},
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

pub trait ReadOnlyResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn index<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::index(context)
    }

    fn show<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::show(context)
    }

    fn linkage<'req>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
    where
        'sch: 'req,
    {
        serve::linkage(context, relationship)
    }

    fn related<'req>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
    where
        'sch: 'req,
    {
        serve::related(context, relationship)
    }
}

pub trait ResourceController<'sch, Adapter: AdapterInterface + 'sch> {
    fn index<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::index(context)
    }

    fn show<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::show(context)
    }

    fn create<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::create(context)
    }

    fn update<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::update(context)
    }

    fn delete<'req>(context: ResourceContext<'sch, 'req, Adapter>) -> Result
    where
        'sch: 'req,
    {
        serve::delete(context)
    }

    fn linkage<'req>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
    where
        'sch: 'req,
    {
        serve::linkage(context, relationship)
    }

    fn related<'req>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
    where
        'sch: 'req,
    {
        serve::related(context, relationship)
    }

    fn link<'req>(context: ResourceContext<'sch, 'req, Adapter>, relationship: &'sch str) -> Result
    where
        'sch: 'req,
    {
        serve::link(context, relationship)
    }

    fn unlink<'req>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
    where
        'sch: 'req,
    {
        serve::unlink(context, relationship)
    }

    fn relink<'req>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result
    where
        'sch: 'req,
    {
        serve::relink(context, relationship)
    }
}

mod serve {
    use super::*;

    pub(super) fn index<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        context: ResourceContext<'sch, 'req, Adapter>,
    ) -> Result {
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

    pub(super) fn show<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        context: ResourceContext<'sch, 'req, Adapter>,
    ) -> Result {
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

    pub(super) fn create<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        mut context: ResourceContext<'sch, 'req, Adapter>,
    ) -> Result {
        let new_record = context.require_record()?;
        let parameters = context.query_parameters()?;
        let Composite { content, included } =
            context.store()?.create_record(new_record, parameters)?;
        let document = to_document(
            &content,
            included,
            context.uri(),
            &DefaultUriGenerator::default(),
        )?;

        respond_with(StatusCode::CREATED, Some(document))
    }

    pub(super) fn update<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        mut context: ResourceContext<'sch, 'req, Adapter>,
    ) -> Result {
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

    pub(super) fn delete<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        context: ResourceContext<'sch, 'req, Adapter>,
    ) -> Result {
        let id = context.require_id()?;
        context.store()?.delete_record(context.schema(), id)?;

        no_content()
    }

    pub(super) fn linkage<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        context: ResourceContext<'sch, 'req, Adapter>,
        relationship: &'sch str,
    ) -> Result {
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

    pub(super) fn related<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        _context: ResourceContext<'sch, 'req, Adapter>,
        _relationship: &'sch str,
    ) -> Result {
        unimplemented!("implement related endpoint")
    }

    pub(super) fn link<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        _context: ResourceContext<'sch, 'req, Adapter>,
        _relationship: &'sch str,
    ) -> Result {
        unimplemented!("implement link endpoint")
    }

    pub(super) fn unlink<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        _context: ResourceContext<'sch, 'req, Adapter>,
        _relationship: &'sch str,
    ) -> Result {
        unimplemented!("implement unlink endpoint")
    }

    pub(super) fn relink<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        _context: ResourceContext<'sch, 'req, Adapter>,
        _relationship: &'sch str,
    ) -> Result {
        unimplemented!("implement relink endpoint")
    }
}
