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

    #[derive(Default)]
    struct Authors;
    impl<'sch> ResourceController<'sch, SqliteAdapter> for Authors {}

    #[derive(Default)]
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
    fn test_index_returns_collection() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/authors", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

        let response =
            Authors::default().index(ResourceContext::new(schema(&manager, "authors"), context))?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);
        assert!(
            body(&response)["data"]
                .as_array()
                .expect("a data array")
                .iter()
                .all(|resource| resource["type"] == json!("authors"))
        );

        Ok(())
    }

    #[test]
    fn test_show_returns_record() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/books/1", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response =
            Books::default().show(ResourceContext::new(schema(&manager, "books"), context))?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("books"));
        assert_eq!(body(&response)["data"]["id"], json!("1"));
        assert_eq!(body(&response)["data"]["attributes"]["title"], json!("One"));

        Ok(())
    }

    #[test]
    fn test_show_missing_is_not_found() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/books/999", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("999"), request);

        match Books::default().show(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("a missing record must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::NOT_FOUND);
                Ok(())
            }
        }
    }

    #[test]
    fn test_create_persists_record() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/books",
            json!({ "data": { "type": "books", "attributes": { "title": "Four" } } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

        let created =
            Books::default().create(ResourceContext::new(schema(&manager, "books"), context))?;

        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(body(&created)["data"]["type"], json!("books"));
        assert_eq!(body(&created)["data"]["attributes"]["title"], json!("Four"));

        // Persistence: the assigned id resolves to the stored record on a fresh read.
        let id = body(&created)["data"]["id"]
            .as_str()
            .expect("an assigned id")
            .to_string();
        let request = build_request("GET", &format!("/books/{id}"), Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id(&id), request);

        let fetched =
            Books::default().show(ResourceContext::new(schema(&manager, "books"), context))?;
        assert_eq!(body(&fetched)["data"]["attributes"]["title"], json!("Four"));

        Ok(())
    }

    #[test]
    fn test_create_with_belongs_to_relationship() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/books",
            json!({
                "data": {
                    "type": "books",
                    "attributes": { "title": "Five" },
                    "relationships": { "author": { "data": { "type": "authors", "id": "2" } } }
                }
            }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

        let created =
            Books::default().create(ResourceContext::new(schema(&manager, "books"), context))?;

        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(
            body(&created)["data"]["relationships"]["author"]["data"]["id"],
            json!("2")
        );

        Ok(())
    }

    #[test]
    fn test_create_rejects_type_mismatch() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/books",
            json!({ "data": { "type": "authors", "attributes": { "title": "Wrong" } } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

        match Books::default().create(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("a type mismatch must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::CONFLICT);
                Ok(())
            }
        }
    }

    #[test]
    fn test_create_rejects_unknown_attribute() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/books",
            json!({ "data": { "type": "books", "attributes": { "bogus": "x" } } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

        match Books::default().create(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("an unknown attribute must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                Ok(())
            }
        }
    }

    #[test]
    fn test_create_rejects_malformed_document() -> TestResult {
        let manager = manager()?;
        let request = build_request("POST", "/books", json!({ "title": "Naked" }))?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

        match Books::default().create(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("a malformed document must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                Ok(())
            }
        }
    }

    #[test]
    fn test_update_changes_attributes() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1",
            json!({
                "data": { "type": "books", "id": "1", "attributes": { "title": "Renamed" } }
            }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response =
            Books::default().update(ResourceContext::new(schema(&manager, "books"), context))?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body(&response)["data"]["attributes"]["title"],
            json!("Renamed")
        );

        Ok(())
    }

    #[test]
    fn test_update_missing_is_not_found() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/999",
            json!({ "data": { "type": "books", "id": "999", "attributes": { "title": "Ghost" } } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("999"), request);

        match Books::default().update(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("a missing record must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::NOT_FOUND);
                Ok(())
            }
        }
    }

    #[test]
    fn test_update_patches_belongs_to_relationship() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1",
            json!({
                "data": {
                    "type": "books",
                    "id": "1",
                    "relationships": { "author": { "data": { "type": "authors", "id": "2" } } }
                }
            }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response =
            Books::default().update(ResourceContext::new(schema(&manager, "books"), context))?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body(&response)["data"]["relationships"]["author"]["data"]["id"],
            json!("2")
        );

        Ok(())
    }

    #[test]
    fn test_update_rejects_type_mismatch() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1",
            json!({ "data": { "type": "authors", "id": "1" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Books::default().update(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("a type mismatch must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::CONFLICT);
                Ok(())
            }
        }
    }

    #[test]
    fn test_update_rejects_id_mismatch() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "PATCH",
            "/books/1",
            json!({ "data": { "type": "books", "id": "2" } }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Books::default().update(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("an id mismatch must error".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::CONFLICT);
                Ok(())
            }
        }
    }

    #[test]
    fn test_delete_removes_record() -> TestResult {
        let manager = manager()?;
        let request = build_request("DELETE", "/books/2", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("2"), request);

        let deleted =
            Books::default().delete(ResourceContext::new(schema(&manager, "books"), context))?;
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

        let request = build_request("GET", "/books/2", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("2"), request);

        match Books::default().show(ResourceContext::new(schema(&manager, "books"), context)) {
            Ok(_) => Err("a deleted record must be gone".into()),
            Err(error) => {
                let status: StatusCode = error.status_code().into();
                assert_eq!(status, StatusCode::NOT_FOUND);
                Ok(())
            }
        }
    }

    #[test]
    fn test_linkage_to_many() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/authors/1/relationships/books", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Authors::default().linkage(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);
        assert!(
            body(&response)["data"]
                .as_array()
                .expect("a linkage array")
                .iter()
                .all(|identifier| identifier["type"] == json!("books"))
        );

        Ok(())
    }

    #[test]
    fn test_linkage_to_one() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/books/1/relationships/author", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Books::default().linkage(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("authors"));
        assert_eq!(body(&response)["data"]["id"], json!("1"));

        Ok(())
    }

    #[test]
    fn test_linkage_empty_to_one() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/books/3/relationships/author", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("3"), request);

        let response = Books::default().linkage(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"], json!(null));

        Ok(())
    }

    #[test]
    fn test_linkage_has_one() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/authors/1/relationships/bio", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        let response = Authors::default().linkage(
            ResourceContext::new(schema(&manager, "authors"), context),
            "bio",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("bios"));
        assert_eq!(body(&response)["data"]["id"], json!("1"));

        Ok(())
    }

    #[test]
    fn test_linkage_unknown_relationship_is_internal_error() -> TestResult {
        let manager = manager()?;
        let request = build_request("GET", "/authors/1/relationships/ghost", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("1"), request);

        match Authors::default().linkage(
            ResourceContext::new(schema(&manager, "authors"), context),
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
    fn test_link_adds_to_collection() -> TestResult {
        let manager = manager()?;
        let request = build_request(
            "POST",
            "/authors/2/relationships/books",
            json!({ "data": [{ "type": "books", "id": "3" }] }),
        )?;
        let uri: Uri = request.uri().clone().into();
        let context = Context::from_request(&manager, &uri, route_id("2"), request);

        let response = Authors::default().link(
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

        let response = Authors::default().relink(
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

        let response = Authors::default().unlink(
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

        let response = Books::default().relink(
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

        let response = Books::default().relink(
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

        let response = Authors::default().relink(
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

        match Books::default().relink(
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

        match Authors::default().link(
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

        match Authors::default().link(
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

        match Books::default().relink(
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

        match Books::default().relink(
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

        match Books::default().relink(
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

        match Authors::default().link(
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

        match Authors::default().unlink(
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

    // `related` serves the related resources as primary content through their own
    // canonical controller, which the request context resolves from a controller
    // lookup. These tests inject that lookup directly.

    #[derive(Default)]
    struct Bios;
    impl<'sch> ResourceController<'sch, SqliteAdapter> for Bios {}

    // Maps each resource kind to its canonical controller — the resolution `related`
    // performs to forward to the related type's serving.
    fn controllers() -> ControllerLookup<'static, SqliteAdapter> {
        ControllerLookup::default()
            .register::<Authors>("authors")
            .register::<Books>("books")
            .register::<Bios>("bios")
    }

    #[test]
    fn test_related_to_many_serves_collection() -> TestResult {
        let manager = manager()?;
        let lookup = controllers();
        let request = build_request("GET", "/authors/1/books", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context =
            Context::from_request(&manager, &uri, route_id("1"), request).with_controllers(&lookup);

        let response = Authors::default().related(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);
        assert!(
            body(&response)["data"]
                .as_array()
                .expect("a data array")
                .iter()
                .all(|resource| resource["type"] == json!("books")
                    && resource["attributes"]["title"].is_string())
        );

        Ok(())
    }

    #[test]
    fn test_related_to_one_serves_record() -> TestResult {
        let manager = manager()?;
        let lookup = controllers();
        let request = build_request("GET", "/books/1/author", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context =
            Context::from_request(&manager, &uri, route_id("1"), request).with_controllers(&lookup);

        let response = Books::default().related(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("authors"));
        assert_eq!(body(&response)["data"]["id"], json!("1"));
        assert_eq!(body(&response)["data"]["attributes"]["name"], json!("Ann"));

        Ok(())
    }

    #[test]
    fn test_related_empty_to_one_is_null() -> TestResult {
        let manager = manager()?;
        let lookup = controllers();
        let request = build_request("GET", "/books/3/author", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context =
            Context::from_request(&manager, &uri, route_id("3"), request).with_controllers(&lookup);

        let response = Books::default().related(
            ResourceContext::new(schema(&manager, "books"), context),
            "author",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"], json!(null));

        Ok(())
    }

    #[test]
    fn test_related_has_one_serves_record() -> TestResult {
        let manager = manager()?;
        let lookup = controllers();
        let request = build_request("GET", "/authors/1/bio", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context =
            Context::from_request(&manager, &uri, route_id("1"), request).with_controllers(&lookup);

        let response = Authors::default().related(
            ResourceContext::new(schema(&manager, "authors"), context),
            "bio",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(&response)["data"]["type"], json!("bios"));
        assert_eq!(body(&response)["data"]["id"], json!("1"));

        Ok(())
    }

    #[test]
    fn test_related_supports_primary_content_include() -> TestResult {
        let manager = manager()?;
        let lookup = controllers();
        let request = build_request("GET", "/authors/1/books?include=author", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context =
            Context::from_request(&manager, &uri, route_id("1"), request).with_controllers(&lookup);

        let response = Authors::default().related(
            ResourceContext::new(schema(&manager, "authors"), context),
            "books",
        )?;

        assert_eq!(response.status(), StatusCode::OK);
        let included = body(&response)["included"]
            .as_array()
            .expect("an included array")
            .clone();
        assert_eq!(included.len(), 1);
        assert_eq!(included[0]["type"], json!("authors"));
        assert_eq!(included[0]["id"], json!("1"));

        Ok(())
    }

    #[test]
    fn test_related_unknown_relationship_is_internal_error() -> TestResult {
        let manager = manager()?;
        let lookup = controllers();
        let request = build_request("GET", "/authors/1/ghost", Value::Null)?;
        let uri: Uri = request.uri().clone().into();
        let context =
            Context::from_request(&manager, &uri, route_id("1"), request).with_controllers(&lookup);

        match Authors::default().related(
            ResourceContext::new(schema(&manager, "authors"), context),
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
}
