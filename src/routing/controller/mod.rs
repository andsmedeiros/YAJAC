#[cfg(test)]
mod tests;

use crate::{
    database::{
        adapters::Adapter as AdapterInterface, attributes::Identifier, composite::Composite,
        error::Error as DatabaseError, query_parameters::QueryParameters, record::Record,
        relationships::Relationship, schema::RelationshipKind,
    },
    http_wrappers::StatusCode,
    json_api::identifier::Identifier as JsonApiIdentifier,
    routing::{Error, ResourceContext, ResourceResult, RouteParameters, responder::*},
    serialisation::factories::{Content, to_document},
};
use http::HeaderMap;
use std::borrow::Cow;
use std::collections::HashMap;

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
            .connection_manager
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

        let registry = context.connection_manager.registry();
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
            .connection_manager
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
            .connection_manager
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
            .connection_manager
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
