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
        mut context: ResourceContext<'sch, 'req, Adapter>,
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

    pub(super) fn unlink<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        mut context: ResourceContext<'sch, 'req, Adapter>,
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

    pub(super) fn relink<'sch: 'req, 'req, Adapter: AdapterInterface + 'sch>(
        mut context: ResourceContext<'sch, 'req, Adapter>,
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

#[cfg(test)]
mod tests {
    use super::{ResourceContext, ResourceController};
    use crate::database::adapters::SqliteAdapter;
    use crate::database::adapters::sqlite::Pool;
    use crate::database::connection_manager::ConnectionManager;
    use crate::database::registry::Registry;
    use crate::database::schema::{AttributeType, Related, Schema, SchemaBuilder};
    use crate::http_wrappers::Uri;
    use crate::json_api::document::Document;
    use crate::routing::{Context, Request, RouteParameters};
    use http::StatusCode;
    use serde_json::{Value, json};
    use std::error::Error as StdError;
    use test_log::test;

    type Manager = ConnectionManager<'static, SqliteAdapter>;
    type TestResult = Result<(), Box<dyn StdError>>;

    struct Authors;
    impl<'sch> ResourceController<'sch, SqliteAdapter> for Authors {}

    struct Books;
    impl<'sch> ResourceController<'sch, SqliteAdapter> for Books {}

    // `books.author_id` and `bios.author_id` are nullable so that detaching (relink/unlink) is a
    // legal write; `bios.author_id` is unique, exercising the has-one path.
    fn schemas() -> [SchemaBuilder<'static>; 3] {
        [
            SchemaBuilder::table("authors")
                .attribute("name", AttributeType::Text)
                .has_many(
                    "books",
                    Related::to("books")
                        .pointing_related("author_id")
                        .to_own("id"),
                )
                .has_one(
                    "bio",
                    Related::to("bios")
                        .pointing_related("author_id")
                        .to_own("id"),
                ),
            SchemaBuilder::table("books")
                .attribute("title", AttributeType::Text)
                .foreign_key("author_id", AttributeType::Integer)
                .belongs_to(
                    "author",
                    Related::to("authors")
                        .pointing_own("author_id")
                        .to_related("id"),
                ),
            SchemaBuilder::table("bios")
                .attribute("text", AttributeType::Text)
                .foreign_key("author_id", AttributeType::Integer)
                .belongs_to(
                    "author",
                    Related::to("authors")
                        .pointing_own("author_id")
                        .to_related("id"),
                ),
        ]
    }

    fn manager() -> Result<Manager, Box<dyn StdError>> {
        let manager: Manager =
            ConnectionManager::new(Registry::try_new(schemas())?, Pool::memory()?);

        manager.acquire()?.execute_batch(
            "CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
             CREATE TABLE books ( \
               id INTEGER PRIMARY KEY, \
               author_id INTEGER, \
               title TEXT NOT NULL, \
               FOREIGN KEY(author_id) REFERENCES authors(id) \
             ); \
             CREATE TABLE bios ( \
               id INTEGER PRIMARY KEY, \
               author_id INTEGER UNIQUE, \
               text TEXT NOT NULL, \
               FOREIGN KEY(author_id) REFERENCES authors(id) \
             ); \
             INSERT INTO authors (id, name) VALUES (1, 'Ann'), (2, 'Bob'); \
             INSERT INTO books (id, author_id, title) \
               VALUES (1, 1, 'One'), (2, 1, 'Two'), (3, NULL, 'Three'); \
             INSERT INTO bios (id, author_id, text) VALUES (1, 1, 'About Ann');",
        )?;

        Ok(manager)
    }

    fn build_request(method: &str, uri: &str, body: Value) -> Result<Request, Box<dyn StdError>> {
        let document = match body {
            Value::Null => None,
            value => Some(serde_json::from_value(value)?),
        };

        Ok(http::Request::builder()
            .method(method)
            .uri(uri)
            .body(document)?)
    }

    fn route_id(id: &str) -> RouteParameters {
        let mut route = RouteParameters::new();
        route.insert("id", id);
        route
    }

    fn schema<'sch>(manager: &'sch Manager, name: &str) -> &'sch Schema<'sch> {
        manager
            .registry()
            .schema(name)
            .expect("schema is registered")
    }

    fn body(response: &http::Response<Option<Document>>) -> Value {
        serde_json::to_value(response.body()).expect("a serialisable document")
    }

    fn data_ids(response: &http::Response<Option<Document>>) -> Vec<Value> {
        let mut ids: Vec<Value> = body(response)["data"]
            .as_array()
            .expect("a linkage array")
            .iter()
            .map(|member| member["id"].clone())
            .collect();
        ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        ids
    }

    #[test]
    fn test_link_adds_to_collection() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/authors/2/relationships/books",
            json!({ "data": [{ "type": "books", "id": "3" }] }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("2"), request);

        let response = Authors::link(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(data_ids(&response), vec![json!("3")]);

        Ok(())
    }

    #[test]
    fn test_relink_replaces_collection() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/authors/1/relationships/books",
            json!({ "data": [{ "type": "books", "id": "2" }, { "type": "books", "id": "3" }] }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Authors::relink(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(data_ids(&response), vec![json!("2"), json!("3")]);

        Ok(())
    }

    #[test]
    fn test_unlink_removes_from_collection() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "DELETE",
            "/authors/1/relationships/books",
            json!({ "data": [{ "type": "books", "id": "2" }] }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Authors::unlink(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(data_ids(&response), vec![json!("1")]);

        Ok(())
    }

    #[test]
    fn test_relink_sets_belongs_to_target() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1/relationships/author",
            json!({ "data": { "type": "authors", "id": "2" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Books::relink(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("authors"));
        assert_eq!(body(&response)["data"]["id"], json!("2"));

        Ok(())
    }

    #[test]
    fn test_relink_null_clears_to_one() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1/relationships/author",
            json!({ "data": null }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Books::relink(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"], json!(null));

        Ok(())
    }

    #[test]
    fn test_relink_sets_has_one_target() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/authors/2/relationships/bio",
            json!({ "data": { "type": "bios", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("2"), request);

        let response = Authors::relink(
            ResourceContext::new(schema(&manager, "authors"), context),
            "bio",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("bios"));
        assert_eq!(body(&response)["data"]["id"], json!("1"));

        Ok(())
    }

    #[test]
    fn test_relink_missing_target_is_not_found() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1/relationships/author",
            json!({ "data": { "type": "authors", "id": "999" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Books::relink(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        ) {
            Ok(_) => Err("a missing target must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::NOT_FOUND);
                Ok(())
            }
        }
    }

    #[test]
    fn test_link_rejects_to_one_linkage() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/authors/1/relationships/books",
            json!({ "data": { "type": "books", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Authors::link(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        ) {
            Ok(_) => Err("a to-one linkage on a to-many endpoint must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                Ok(())
            }
        }
    }

    #[test]
    fn test_link_missing_target_is_not_found() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/authors/1/relationships/books",
            json!({ "data": [{ "type": "books", "id": "999" }] }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Authors::link(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        ) {
            Ok(_) => Err("a missing target must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::NOT_FOUND);
                Ok(())
            }
        }
    }

    #[test]
    fn test_relink_missing_parent_is_not_found() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/999/relationships/author",
            json!({ "data": { "type": "authors", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("999"), request);

        match Books::relink(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        ) {
            Ok(_) => Err("a missing parent must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::NOT_FOUND);
                Ok(())
            }
        }
    }

    #[test]
    fn test_relink_unknown_relationship_is_internal_error() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1/relationships/ghost",
            json!({ "data": { "type": "authors", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Books::relink(
            ResourceContext::new(schema(&manager, "books"), context),
            "ghost",
        ) {
            Ok(_) => Err("an unknown relationship must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                Ok(())
            }
        }
    }

    #[test]
    fn test_relink_without_body_is_unprocessable() -> TestResult {
        let manager = manager()?;
        let request = build_request("PATCH", "/books/1/relationships/author", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Books::relink(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        ) {
            Ok(_) => Err("a missing body must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                Ok(())
            }
        }
    }

    #[test]
    fn test_link_on_to_one_is_kind_mismatch() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/authors/1/relationships/bio",
            json!({ "data": { "type": "bios", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Authors::link(
            ResourceContext::new(schema(&manager, "authors"), context),
            "bio",
        ) {
            Ok(_) => Err("adding to a to-one must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                Ok(())
            }
        }
    }

    #[test]
    fn test_unlink_on_to_one_is_kind_mismatch() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "DELETE",
            "/authors/1/relationships/bio",
            json!({ "data": { "type": "bios", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Authors::unlink(
            ResourceContext::new(schema(&manager, "authors"), context),
            "bio",
        ) {
            Ok(_) => Err("removing from a to-one must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                Ok(())
            }
        }
    }
}
