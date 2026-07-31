//! Shared fixture for the JSON:API v1.1 conformance suite.
//!
//! Black-box: drives the crate exactly as an embedder would — build a
//! `ConnectionManager`, wire a `RouterBuilder`, hand `Router::handle` an
//! `http::Request<Vec<u8>>`, and inspect the `http::Response<Option<Document>>`.
//!
//! Only the response's status, headers, and serialised document are exposed;
//! generic validation lives in `validations`, and request-specific assertions
//! live in each test.

// A shared toolbox: not every test module uses every accessor.
#![allow(dead_code)]

use http::{HeaderMap, Method, StatusCode};
use serde_json::Value;
use yajac::database::adapters::SqliteAdapter;
use yajac::database::adapters::sqlite::Pool;
use yajac::database::connection_manager::ConnectionManager;
use yajac::database::registry::Registry;
use yajac::database::schema::{AttributeType, IdentifierType, Related, SchemaBuilder};
use yajac::routing::Router;
use yajac::routing::controller::{Configuration, ResourceController};

pub type BoxError = Box<dyn std::error::Error>;
pub type TestResult = Result<(), BoxError>;

/// The JSON:API media type, sans parameters.
pub const JSONAPI: &str = "application/vnd.api+json";

// --- Controllers ----------------------------------------------------------
//
// Stateless ZST markers, one per resource. `profiles` is the contract's
// read-only resource, so it carries a `ReadOnlyResourceController`; the rest are
// read-write.

#[derive(Default)]
pub struct Authors;
#[derive(Default)]
pub struct Articles;
#[derive(Default)]
pub struct Comments;
#[derive(Default)]
pub struct Profiles;
#[derive(Default)]
pub struct Tags;

impl<'sch> ResourceController<'sch, SqliteAdapter> for Authors {}
impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {}
impl<'sch> ResourceController<'sch, SqliteAdapter> for Comments {}
impl<'sch> ResourceController<'sch, SqliteAdapter> for Profiles {}
// `tags` has a text primary key with no server-side source, so it accepts the
// client-generated ids the spec's `Client-Generated IDs` section provides for.
impl<'sch> ResourceController<'sch, SqliteAdapter> for Tags {
    #[allow(clippy::needless_update)]
    fn configuration(&self) -> Configuration {
        Configuration {
            accepts_client_ids: true,
            ..Default::default()
        }
    }
}

// --- Abstract schema set --------------------------------------------------

fn authors_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("authors")
        .attribute("name", AttributeType::Text)
        .attribute("age", AttributeType::Integer)
        .attribute("rating", AttributeType::Float)
        .attribute("active", AttributeType::Boolean)
        .attribute("joined_at", AttributeType::DateTime)
        .has_many(
            "articles",
            Related::to("articles")
                .pointing_related("author_id")
                .to_own("id"),
        )
        .has_one(
            "profile",
            Related::to("profiles")
                .pointing_related("author_id")
                .to_own("id"),
        )
        .has_many(
            "edited",
            Related::to("articles")
                .pointing_related("editor_id")
                .to_own("id"),
        )
}

fn articles_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("articles")
        .attribute("title", AttributeType::Text)
        .attribute("body", AttributeType::Text)
        .attribute("published", AttributeType::Boolean)
        .foreign_key("author_id", AttributeType::Integer)
        .foreign_key("editor_id", AttributeType::Integer)
        .belongs_to(
            "author",
            Related::to("authors")
                .pointing_own("author_id")
                .to_related("id"),
        )
        .belongs_to(
            "editor",
            Related::to("authors")
                .pointing_own("editor_id")
                .to_related("id"),
        )
        .has_many(
            "comments",
            Related::to("comments")
                .pointing_related("article_id")
                .to_own("id"),
        )
}

fn comments_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("comments")
        .attribute("content", AttributeType::Text)
        .foreign_key("article_id", AttributeType::Integer)
        .foreign_key("author_id", AttributeType::Integer)
        .foreign_key("parent_id", AttributeType::Integer)
        .belongs_to(
            "article",
            Related::to("articles")
                .pointing_own("article_id")
                .to_related("id"),
        )
        .belongs_to(
            "author",
            Related::to("authors")
                .pointing_own("author_id")
                .to_related("id"),
        )
        .belongs_to(
            "parent",
            Related::to("comments")
                .pointing_own("parent_id")
                .to_related("id"),
        )
        .has_many(
            "replies",
            Related::to("comments")
                .pointing_related("parent_id")
                .to_own("id"),
        )
}

fn profiles_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("profiles")
        .attribute("bio", AttributeType::Text)
        .foreign_key("author_id", AttributeType::Integer)
        .belongs_to(
            "author",
            Related::to("authors")
                .pointing_own("author_id")
                .to_related("id"),
        )
}

fn tags_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("tags")
        .primary_key("id", IdentifierType::Text)
        .attribute("label", AttributeType::Text)
}

fn schemas() -> [SchemaBuilder<'static>; 5] {
    [
        authors_schema(),
        articles_schema(),
        comments_schema(),
        profiles_schema(),
        tags_schema(),
    ]
}

// Authors are seeded so that name, age, and (active, name) orderings each
// differ from insertion order, giving sorting tests a real signal:
//   name  ascending → 2,1,5,3,4        age ascending → 5,2,3,4,1
//   (active, name)  → 2,4,1,5,3
const SCHEMA_AND_SEED: &str = "
    CREATE TABLE authors (
        id INTEGER PRIMARY KEY, name TEXT NOT NULL, age INTEGER,
        rating REAL, active BOOLEAN, joined_at TEXT
    );
    CREATE TABLE profiles (
        id INTEGER PRIMARY KEY, author_id INTEGER NOT NULL UNIQUE, bio TEXT,
        FOREIGN KEY(author_id) REFERENCES authors(id)
    );
    CREATE TABLE articles (
        id INTEGER PRIMARY KEY, author_id INTEGER NOT NULL, editor_id INTEGER,
        title TEXT NOT NULL, body TEXT, published BOOLEAN,
        FOREIGN KEY(author_id) REFERENCES authors(id),
        FOREIGN KEY(editor_id) REFERENCES authors(id)
    );
    CREATE TABLE comments (
        id INTEGER PRIMARY KEY, article_id INTEGER NOT NULL,
        author_id INTEGER NOT NULL, parent_id INTEGER, content TEXT NOT NULL,
        FOREIGN KEY(article_id) REFERENCES articles(id),
        FOREIGN KEY(author_id) REFERENCES authors(id),
        FOREIGN KEY(parent_id) REFERENCES comments(id)
    );
    CREATE TABLE tags (id TEXT PRIMARY KEY, label TEXT NOT NULL);

    INSERT INTO authors (id, name, age, rating, active, joined_at) VALUES
        (1, 'Carol', 40, 4.5, 1, '2018-01-01T00:00:00Z'),
        (2, 'Alice', 25, 3.0, 0, '2019-01-01T00:00:00Z'),
        (3, 'Eve',   30, 4.0, 1, '2020-01-01T00:00:00Z'),
        (4, 'Zed',   35, 2.5, 0, '2021-01-01T00:00:00Z'),
        (5, 'Dave',  20, 5.0, 1, '2022-01-01T00:00:00Z');
    INSERT INTO profiles (id, author_id, bio) VALUES
        (1, 1, 'Carol bio'), (2, 2, 'Alice bio');
    -- Author 3 edits articles 1 and 2, giving `authors/3/edited` a clearable to-many.
    INSERT INTO articles (id, author_id, editor_id, title, body, published) VALUES
        (1, 1, 3,    'First',  'Body one',   1),
        (2, 1, 3,    'Second', 'Body two',   0),
        (3, 2, NULL, 'Third',  'Body three', 1);
    INSERT INTO comments (id, article_id, author_id, parent_id, content) VALUES
        (1, 1, 2, NULL, 'Nice'),
        (2, 1, 1, 1,    'Thanks'),
        (3, 3, 1, NULL, 'Cool');
    INSERT INTO tags (id, label) VALUES ('rust', 'Rust'), ('web', 'Web');
";

// --- Fixture --------------------------------------------------------------

/// A freshly seeded API instance. Build one per test for isolation; requests
/// against it share the in-memory database, so mutations persist within a test.
pub struct Api {
    manager: ConnectionManager<'static, SqliteAdapter>,
}

impl Api {
    pub fn new() -> Result<Self, BoxError> {
        let manager: ConnectionManager<SqliteAdapter> =
            ConnectionManager::new(Registry::try_new(schemas())?, Pool::memory()?);
        manager.acquire()?.execute_batch(SCHEMA_AND_SEED)?;
        Ok(Api { manager })
    }

    /// Dispatches a request with a valid JSON:API `Content-Type`. `body` is sent
    /// as the request body; pass `Value::Null` for an empty body.
    pub fn request(&self, method: &str, uri: &str, body: Value) -> Result<Res, BoxError> {
        self.request_with(method, uri, body, &[("Content-Type", JSONAPI)])
    }

    /// Dispatches a request with an explicit header set (for content negotiation).
    pub fn request_with(
        &self,
        method: &str,
        uri: &str,
        body: Value,
        headers: &[(&str, &str)],
    ) -> Result<Res, BoxError> {
        let registry = self.manager.registry();
        let authors = registry.schema("authors")?;
        let articles = registry.schema("articles")?;
        let comments = registry.schema("comments")?;
        let profiles = registry.schema("profiles")?;
        let tags = registry.schema("tags")?;
        let router = Router::try_new(|root| {
            root.resource::<Authors>("authors", authors)
                .resource::<Articles>("articles", articles)
                .resource::<Comments>("comments", comments)
                .read_only_resource::<Profiles>("profiles", profiles)
                .resource::<Tags>("tags", tags)
        })?;

        let mut request = http::Request::builder()
            .method(Method::from_bytes(method.as_bytes())?)
            .uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = request.body(serde_json::to_vec(&body)?)?;

        let response = router.handle(&self.manager, request);
        let status = response.status();
        let headers = response.headers().clone();
        let doc = serde_json::to_value(response.body())?;

        Ok(Res {
            status,
            headers,
            doc,
        })
    }

    pub fn get(&self, uri: &str) -> Result<Res, BoxError> {
        self.request("GET", uri, Value::Null)
    }
    pub fn post(&self, uri: &str, body: Value) -> Result<Res, BoxError> {
        self.request("POST", uri, body)
    }
    pub fn patch(&self, uri: &str, body: Value) -> Result<Res, BoxError> {
        self.request("PATCH", uri, body)
    }
    pub fn delete(&self, uri: &str) -> Result<Res, BoxError> {
        self.request("DELETE", uri, Value::Null)
    }
}

// --- Response accessors ---------------------------------------------------

/// A dispatched response, reduced to what a black-box test inspects.
pub struct Res {
    status: StatusCode,
    headers: HeaderMap,
    doc: Value,
}

impl Res {
    /// The numeric HTTP status.
    pub fn status(&self) -> u16 {
        self.status.as_u16()
    }

    /// Whether the status is a client error (`4xx`).
    pub fn is_client_error(&self) -> bool {
        (400..=499).contains(&self.status())
    }

    /// A response header value, if present.
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// The whole response document as JSON (`null` when there is no body).
    pub fn doc(&self) -> &Value {
        &self.doc
    }

    /// The value at a JSON Pointer within the document, if present.
    pub fn at(&self, pointer: &str) -> Option<&Value> {
        self.doc.pointer(pointer)
    }
}

// --- Optional-affordance enforcement --------------------------------------
//
// A few invariants are MUST/SHOULD *given support* for a feature whose support
// is itself a MAY (`include`, `sort`, client-generated ids, full to-many
// replacement, relationship-member deletion). They live in the
// mandatory/recommended tiers at their true obligation level, but each guards on
// the spec-defined "unsupported" status: absent enforcement, that response lets
// the test log and return rather than fail. Any other response falls through and
// is asserted. Enforcement is per-affordance so a suite can demand a specific
// feature without demanding the rest.

/// An optional affordance whose support the spec leaves to the implementation.
#[derive(Clone, Copy)]
pub enum Affordance {
    Include,
    Sort,
    ClientIds,
    FullReplacement,
    RelationshipDelete,
}

impl Affordance {
    fn key(self) -> &'static str {
        match self {
            Affordance::Include => "include",
            Affordance::Sort => "sort",
            Affordance::ClientIds => "client-ids",
            Affordance::FullReplacement => "full-replacement",
            Affordance::RelationshipDelete => "relationship-delete",
        }
    }
}

/// Whether an affordance is enforced — its non-support signal treated as a
/// failure rather than a skip. Set `YAJAC_ENFORCE_OPTIONAL` to a comma-separated
/// list of affordance keys (`include`, `sort`, `client-ids`, `full-replacement`,
/// `relationship-delete`), or `all`.
pub fn enforced(affordance: Affordance) -> bool {
    std::env::var("YAJAC_ENFORCE_OPTIONAL").is_ok_and(|list| {
        list.split(',')
            .map(str::trim)
            .any(|item| item == "all" || item == affordance.key())
    })
}
