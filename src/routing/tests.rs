use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::connection_manager::ConnectionManager;
use crate::database::registry::Registry as DatabaseRegistry;
use crate::database::schema::{AttributeType, Related, SchemaBuilder};
use crate::http_wrappers::Uri;
use crate::routing::controller::{ResourceContext, ResourceController};
use crate::routing::middleware::{PrimaryMiddleware, ResourceMiddleware};
use crate::routing::responder::respond_with;
use crate::routing::{
    BaseUri, PrimaryContext, PrimaryHandler, PrimaryResult, ResourceHandler,
    ResourceResult as RouteResult, ResourceVerbs, RouteParameters, Router, RouterError,
    UnboundVerbs,
};
use crate::serialisation::ByteStream;
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::error::Error as StdError;
use std::io::{Cursor, Read};

type Manager = ConnectionManager<'static, SqliteAdapter>;
type TestResult = Result<(), Box<dyn StdError>>;

fn articles_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("articles")
        .attribute("title", AttributeType::Text)
        .attribute("body", AttributeType::Text)
        .has_many(
            "comments",
            Related::to("comments")
                .pointing_related("article_id")
                .to_own("id"),
        )
        .has_many(
            "drafts",
            Related::to("drafts")
                .pointing_related("article_id")
                .to_own("id"),
        )
        .has_one(
            "summary",
            Related::to("summaries")
                .pointing_related("article_id")
                .to_own("id"),
        )
}

fn comments_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("comments")
        .attribute("content", AttributeType::Text)
        .foreign_key("article_id", AttributeType::Integer)
        .belongs_to(
            "article",
            Related::to("articles")
                .pointing_own("article_id")
                .to_related("id"),
        )
}

fn drafts_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("drafts")
        .attribute("title", AttributeType::Text)
        .foreign_key("article_id", AttributeType::Integer)
        .belongs_to(
            "article",
            Related::to("articles")
                .pointing_own("article_id")
                .to_related("id"),
        )
}

fn summaries_schema() -> SchemaBuilder<'static> {
    SchemaBuilder::table("summaries")
        .attribute("abstract", AttributeType::Text)
        .foreign_key("article_id", AttributeType::Integer)
        .belongs_to(
            "article",
            Related::to("articles")
                .pointing_own("article_id")
                .to_related("id"),
        )
}

fn schemas() -> [SchemaBuilder<'static>; 4] {
    [
        articles_schema(),
        comments_schema(),
        drafts_schema(),
        summaries_schema(),
    ]
}

#[derive(Default)]
struct Articles;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {}

#[derive(Default)]
struct Comments;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Comments {}

#[derive(Default)]
struct Drafts;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Drafts {}

#[derive(Default)]
struct Summaries;
impl<'sch> ResourceController<'sch, SqliteAdapter> for Summaries {}

fn manager() -> Result<Manager, Box<dyn StdError>> {
    let manager: Manager =
        ConnectionManager::new(DatabaseRegistry::try_new(schemas())?, Pool::memory()?);

    manager.acquire()?.execute_batch(
        "CREATE TABLE articles (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT); \
         CREATE TABLE comments ( \
           id INTEGER PRIMARY KEY, \
           article_id INTEGER NOT NULL, \
           content TEXT NOT NULL, \
           FOREIGN KEY(article_id) REFERENCES articles(id) \
         ); \
         CREATE TABLE drafts ( \
           id INTEGER PRIMARY KEY, \
           article_id INTEGER, \
           title TEXT NOT NULL, \
           FOREIGN KEY(article_id) REFERENCES articles(id) \
         ); \
         CREATE TABLE summaries ( \
           id INTEGER PRIMARY KEY, \
           article_id INTEGER NOT NULL UNIQUE, \
           abstract TEXT NOT NULL, \
           FOREIGN KEY(article_id) REFERENCES articles(id) \
         ); \
         INSERT INTO articles (id, title, body) \
           VALUES (1, 'First', 'Body one'), (2, 'Second', 'Body two'); \
         INSERT INTO comments (id, article_id, content) \
           VALUES (1, 1, 'Nice'), (2, 1, 'Agreed'); \
         INSERT INTO drafts (id, article_id, title) \
           VALUES (1, 1, 'Draft A'), (2, 1, 'Draft B'); \
         INSERT INTO summaries (id, article_id, abstract) \
           VALUES (1, 1, 'About first'), (2, 2, 'About second');",
    )?;

    Ok(manager)
}

// A record-scoped custom route: proves the handler receives a `ResourceContext` bound to the
// member's `:id`.
fn publish(context: ResourceContext<'_, '_, SqliteAdapter>) -> RouteResult {
    context.require_id()?;
    respond_with(StatusCode::ACCEPTED, None)
}

// A collection-scoped custom route: query parameters parse against the resource schema.
fn search(context: ResourceContext<'_, '_, SqliteAdapter>) -> RouteResult {
    context.query_parameters()?;
    respond_with(StatusCode::OK, None)
}

// An unbound leaf route: schema-oblivious, works the raw byte tier.
fn health(_context: PrimaryContext<'_, '_, SqliteAdapter>) -> PrimaryResult {
    respond_with(StatusCode::OK, None).map_err(Into::into)
}

// The standard resourceful mount: every resource bare, so CRUD and the full set of relationship
// endpoints are auto-enumerated from each schema.
fn standard_router(manager: &Manager) -> Result<Router<'_, SqliteAdapter>, Box<dyn StdError>> {
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let drafts = manager.registry().schema("drafts")?;
    Ok(Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Comments>("comments", comments)
            .resource::<Drafts>("drafts", drafts)
    })?)
}

fn read_only_router(manager: &Manager) -> Result<Router<'_, SqliteAdapter>, Box<dyn StdError>> {
    let summaries = manager.registry().schema("summaries")?;
    Ok(Router::try_new(BaseUri::Relative, |root| {
        root.read_only_resource::<Summaries>("summaries", summaries)
    })?)
}

/// Dispatches a request against `router`, threading `headers` onto it and buffering the streamed
/// response body so a test can read its status, headers, and document repeatedly.
fn send<'a>(
    manager: &'a Manager,
    router: &Router<'a, SqliteAdapter>,
    method: &str,
    uri: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> Result<Response<Vec<u8>>, Box<dyn StdError>> {
    // Stand in for the client: send `Content-Type` alongside any body unless a test set one, and
    // treat a `null` body as no body (streamed as zero bytes).
    let carries_body = !body.is_null();
    let sets_content_type = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));

    let mut builder = headers.iter().fold(
        http::Request::builder().method(method).uri(uri),
        |builder, (name, value)| builder.header(*name, *value),
    );
    if carries_body && !sets_content_type {
        builder = builder.header("content-type", "application/vnd.api+json");
    }
    let stream: ByteStream = if carries_body {
        Box::new(Cursor::new(serde_json::to_vec(&body)?))
    } else {
        Box::new(Cursor::new(Vec::new()))
    };
    let request = builder.body(stream)?;

    let (parts, body) = router.handle(manager, request)?.into_parts();
    let mut buffer = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut buffer)?;
    }
    Ok(Response::from_parts(parts, buffer))
}

fn serve(
    manager: &Manager,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<Response<Vec<u8>>, Box<dyn StdError>> {
    send(manager, &standard_router(manager)?, method, uri, body, &[])
}

fn read_only_serve(
    manager: &Manager,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<Response<Vec<u8>>, Box<dyn StdError>> {
    send(manager, &read_only_router(manager)?, method, uri, body, &[])
}

/// The response body as JSON — `null` when the body is empty (a no-content or bodyless response).
fn body(response: &Response<Vec<u8>>) -> Value {
    if response.body().is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(response.body()).expect("a serialisable document")
    }
}

fn data_ids(response: &Response<Vec<u8>>) -> Vec<Value> {
    let mut ids: Vec<Value> = body(response)["data"]
        .as_array()
        .expect("a data array")
        .iter()
        .map(|member| member["id"].clone())
        .collect();
    ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    ids
}

// --- resourceful routes: CRUD dispatch -------------------------------------

#[test]
fn test_mount_captures_link_templates() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    let mount = router
        .mount_table
        .get("articles")
        .expect("articles is mounted");

    // The base is the collection prefix; the resource path is the base plus `:id`, derived at render.
    assert_eq!(mount.base, vec![Cow::Borrowed("articles")]);

    // Every relationship the schema declares is captured, in definition order.
    assert_eq!(
        mount.relationships.keys().copied().collect::<Vec<_>>(),
        vec!["comments", "drafts", "summary"]
    );

    // Each mounted slot's template mirrors its route's path exactly.
    let comments = &mount.relationships["comments"];
    assert_eq!(
        comments.linkage,
        Some(vec![
            Cow::Borrowed("articles"),
            Cow::Borrowed(":id"),
            Cow::Borrowed("relationships"),
            Cow::Borrowed("comments"),
        ])
    );
    assert_eq!(
        comments.related,
        Some(vec![
            Cow::Borrowed("articles"),
            Cow::Borrowed(":id"),
            Cow::Borrowed("comments"),
        ])
    );
    Ok(())
}

#[test]
fn test_index_yields_collection() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);
    assert!(
        body(&response)["data"]
            .as_array()
            .expect("a data array")
            .iter()
            .all(|resource| resource["type"] == json!("articles"))
    );

    Ok(())
}

#[test]
fn test_show_yields_record() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(&response)["data"]["type"], json!("articles"));
    assert_eq!(body(&response)["data"]["id"], json!("1"));
    assert_eq!(
        body(&response)["data"]["attributes"]["title"],
        json!("First")
    );

    Ok(())
}

#[test]
fn test_show_missing_is_not_found() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/999", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_create_yields_record() -> TestResult {
    let manager = manager()?;
    let response = serve(
        &manager,
        "POST",
        "/articles",
        json!({ "data": { "type": "articles", "attributes": { "title": "Third", "body": "Three" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(body(&response)["data"]["type"], json!("articles"));
    assert_eq!(
        body(&response)["data"]["attributes"]["title"],
        json!("Third")
    );

    Ok(())
}

#[test]
fn test_update_yields_record() -> TestResult {
    let manager = manager()?;
    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({ "data": { "type": "articles", "id": "1", "attributes": { "title": "Updated" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body(&response)["data"]["attributes"]["title"],
        json!("Updated")
    );

    Ok(())
}

#[test]
fn test_delete_removes_record() -> TestResult {
    let manager = manager()?;

    let deleted = serve(&manager, "DELETE", "/comments/1", Value::Null)?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let fetched = serve(&manager, "GET", "/comments/1", Value::Null)?;
    assert_eq!(fetched.status(), StatusCode::NOT_FOUND);

    Ok(())
}

// --- resourceful routes: relationship endpoints ----------------------------

#[test]
fn test_linkage_self_link_yields_identifiers() -> TestResult {
    let manager = manager()?;
    let response = serve(
        &manager,
        "GET",
        "/articles/1/relationships/comments",
        Value::Null,
    )?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);
    assert!(
        body(&response)["data"]
            .as_array()
            .expect("a linkage array")
            .iter()
            .all(|identifier| identifier["type"] == json!("comments"))
    );

    Ok(())
}

#[test]
fn test_related_link_yields_primary_collection() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/1/comments", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);
    assert!(
        body(&response)["data"]
            .as_array()
            .expect("a data array")
            .iter()
            .all(|resource| resource["type"] == json!("comments")
                && resource["attributes"]["content"].is_string())
    );

    Ok(())
}

#[test]
fn test_to_one_linkage_self_link() -> TestResult {
    let manager = manager()?;
    let response = serve(
        &manager,
        "GET",
        "/comments/1/relationships/article",
        Value::Null,
    )?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(&response)["data"]["type"], json!("articles"));
    assert_eq!(body(&response)["data"]["id"], json!("1"));

    Ok(())
}

#[test]
fn test_to_one_related_link_yields_record() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/comments/1/article", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(&response)["data"]["type"], json!("articles"));
    assert_eq!(body(&response)["data"]["id"], json!("1"));
    assert_eq!(
        body(&response)["data"]["attributes"]["title"],
        json!("First")
    );

    Ok(())
}

#[test]
fn test_link_reaches_handler() -> TestResult {
    let manager = manager()?;
    let response = serve(
        &manager,
        "POST",
        "/articles/2/relationships/comments",
        json!({ "data": [{ "type": "comments", "id": "1" }] }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_relink_reaches_handler() -> TestResult {
    let manager = manager()?;
    // `drafts.article_id` is nullable, so replacement can detach the dropped member.
    let response = serve(
        &manager,
        "PATCH",
        "/articles/1/relationships/drafts",
        json!({ "data": [{ "type": "drafts", "id": "2" }] }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_unlink_reaches_handler() -> TestResult {
    let manager = manager()?;
    // `drafts.article_id` is nullable, so removal can detach the member.
    let response = serve(
        &manager,
        "DELETE",
        "/articles/1/relationships/drafts",
        json!({ "data": [{ "type": "drafts", "id": "1" }] }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

// --- resourceful routes: read-only mounts ----------------------------------

#[test]
fn test_read_only_index() -> TestResult {
    let manager = manager()?;
    let response = read_only_serve(&manager, "GET", "/summaries", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(data_ids(&response), vec![json!("1"), json!("2")]);

    Ok(())
}

#[test]
fn test_read_only_show() -> TestResult {
    let manager = manager()?;
    let response = read_only_serve(&manager, "GET", "/summaries/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(&response)["data"]["id"], json!("1"));

    Ok(())
}

#[test]
fn test_read_only_create_is_forbidden() -> TestResult {
    let manager = manager()?;
    let response = read_only_serve(
        &manager,
        "POST",
        "/summaries",
        json!({ "data": { "type": "summaries", "attributes": { "abstract": "New" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[test]
fn test_read_only_delete_is_forbidden() -> TestResult {
    let manager = manager()?;
    let response = read_only_serve(&manager, "DELETE", "/summaries/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[test]
fn test_read_only_relationship_read_is_allowed() -> TestResult {
    let manager = manager()?;
    let response = read_only_serve(
        &manager,
        "GET",
        "/summaries/1/relationships/article",
        Value::Null,
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_read_only_relationship_write_is_forbidden() -> TestResult {
    let manager = manager()?;
    let response = read_only_serve(
        &manager,
        "PATCH",
        "/summaries/1/relationships/article",
        json!({ "data": { "type": "articles", "id": "2" } }),
    )?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

// --- relationship families and configuration -------------------------------

#[test]
fn test_linkage_only_family_omits_related_link() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.linkage("comments")
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn test_related_only_family_omits_self_link() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.related("comments")
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn test_at_override_relocates_relationship_paths() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles
                    .relationship_with("comments", |config| config.at("commentaries", "relations"))
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relations/commentaries",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/commentaries",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn test_read_only_relationship_config_forbids_writes() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.relationship_with("comments", |config| config.read_only())
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "POST",
            "/articles/1/relationships/comments",
            json!({ "data": [{ "type": "comments", "id": "1" }] }),
            &[],
        )?
        .status(),
        StatusCode::FORBIDDEN
    );

    Ok(())
}

#[test]
fn test_relationships_subset_mounts_only_named() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.relationships(&["comments"])
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/drafts",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn test_all_relationships_enumerates_every_relationship() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let drafts = manager.registry().schema("drafts")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource::<Drafts>("drafts", drafts)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.all_relationships()
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/comments",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/relationships/drafts",
            Value::Null,
            &[],
        )?
        .status(),
        StatusCode::OK
    );

    Ok(())
}

// --- non-resourceful routes: custom routes ---------------------------------

#[test]
fn test_member_route_is_record_scoped() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles.member(|member| member.post("publish", publish))
        })
    })?;

    let response = send(
        &manager,
        &router,
        "POST",
        "/articles/1/publish",
        Value::Null,
        &[],
    )?;

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    Ok(())
}

#[test]
fn test_collection_route_is_collection_scoped() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    // A resource builder's own verbs mount collection-scoped schema-bound routes.
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles.get("search", search)
        })
    })?;

    let response = send(
        &manager,
        &router,
        "GET",
        "/articles/search",
        Value::Null,
        &[],
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_unbound_leaf_route() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    // A raw route side-mounted into a resource's path space, declared before the resource so it wins
    // the shared `/articles/:id` slot. It must reach its handler, and the resource must keep serving
    // its own routes alongside.
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("articles/health", health)
            .resource::<Articles>("articles", articles)
    })?;

    let raw = send(
        &manager,
        &router,
        "GET",
        "/articles/health",
        Value::Null,
        &[],
    )?;
    assert_eq!(raw.status(), StatusCode::OK);

    let collection = send(&manager, &router, "GET", "/articles", Value::Null, &[])?;
    assert_eq!(collection.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_unknown_route_is_not_found() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/widgets", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

// --- try_new validation ----------------------------------------------------

#[test]
fn test_duplicate_resource_is_rejected() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let result = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Articles>("posts", articles)
    });

    assert!(matches!(result, Err(RouterError::DuplicateResource { .. })));

    Ok(())
}

#[test]
fn test_duplicate_relationship_slot_is_rejected() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let result = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.relationship("comments").linkage("comments")
            })
    });

    assert!(matches!(
        result,
        Err(RouterError::DuplicateRelationshipSlot { .. })
    ));

    Ok(())
}

#[test]
fn test_all_relationships_after_individual_is_rejected() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let result = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.relationship("comments").all_relationships()
            })
    });

    assert!(matches!(
        result,
        Err(RouterError::DuplicateRelationshipSlot { .. })
    ));

    Ok(())
}

#[test]
fn test_unknown_relationship_is_rejected() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let result = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles.relationship("ghost")
        })
    });

    assert!(matches!(
        result,
        Err(RouterError::UnknownRelationship { .. })
    ));

    Ok(())
}

#[test]
fn test_unmounted_resource_is_allowed() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    // `comments`, `drafts`, and `summaries` are registered but never mounted.
    let result = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Articles>("articles", articles)
    });

    assert!(result.is_ok());

    Ok(())
}

// --- middleware ------------------------------------------------------------

/// A primary guard admitting only requests that carry a given header.
struct RequireHeader(&'static str);
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for RequireHeader {
    fn matches(&self, headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        headers.contains_key(self.0)
    }
}

/// A primary around-filter stamping a marker header on the response coming back through it.
struct StampResponse;
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for StampResponse {
    fn handle<'req>(
        &self,
        context: PrimaryContext<'sch, 'req, SqliteAdapter>,
        next: &PrimaryHandler<'sch, 'req, SqliteAdapter>,
    ) -> PrimaryResult
    where
        'sch: 'req,
    {
        let mut response = next(context)?;
        response
            .headers_mut()
            .insert("x-stamp", HeaderValue::from_static("seen"));
        Ok(response)
    }
}

/// A primary middleware short-circuiting the chain — it answers 401 without invoking `next`.
struct Deny;
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for Deny {
    fn handle<'req>(
        &self,
        _context: PrimaryContext<'sch, 'req, SqliteAdapter>,
        _next: &PrimaryHandler<'sch, 'req, SqliteAdapter>,
    ) -> PrimaryResult
    where
        'sch: 'req,
    {
        respond_with(StatusCode::UNAUTHORIZED, None).map_err(Into::into)
    }
}

/// A resource guard admitting only requests that carry a given header, on the schema-bound tier.
struct RequireResourceHeader(&'static str);
impl<'sch> ResourceMiddleware<'sch, SqliteAdapter> for RequireResourceHeader {
    fn matches(&self, headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        headers.contains_key(self.0)
    }
}

/// A resource around-filter stamping a marker header on the document response coming back — the
/// header must survive the crossing's serialisation to the byte response.
struct StampResourceResponse;
impl<'sch> ResourceMiddleware<'sch, SqliteAdapter> for StampResourceResponse {
    fn handle<'req>(
        &self,
        context: ResourceContext<'sch, 'req, SqliteAdapter>,
        next: &ResourceHandler<'sch, 'req, SqliteAdapter>,
    ) -> RouteResult
    where
        'sch: 'req,
    {
        let mut response = next(context)?;
        response
            .headers_mut()
            .insert("x-stamp", HeaderValue::from_static("seen"));
        Ok(response)
    }
}

/// A resource middleware short-circuiting the chain — it refuses with 403 without invoking `next`.
struct DenyResource;
impl<'sch> ResourceMiddleware<'sch, SqliteAdapter> for DenyResource {
    fn handle<'req>(
        &self,
        _context: ResourceContext<'sch, 'req, SqliteAdapter>,
        _next: &ResourceHandler<'sch, 'req, SqliteAdapter>,
    ) -> RouteResult
    where
        'sch: 'req,
    {
        respond_with(StatusCode::FORBIDDEN, None)
    }
}

#[test]
fn test_primary_guard_falls_through_when_unmet() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireHeader("x-key"), |root| root.get("health", health))
    })?;

    let missing = send(&manager, &router, "GET", "/health", Value::Null, &[])?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let present = send(
        &manager,
        &router,
        "GET",
        "/health",
        Value::Null,
        &[("x-key", "present")],
    )?;
    assert_eq!(present.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_primary_middleware_rewrites_response() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(StampResponse, |root| root.get("health", health))
    })?;

    let response = send(&manager, &router, "GET", "/health", Value::Null, &[])?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-stamp")
            .and_then(|value| value.to_str().ok()),
        Some("seen")
    );

    Ok(())
}

#[test]
fn test_primary_middleware_short_circuits_handler() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(Deny, |root| root.get("health", health))
    })?;

    let response = send(&manager, &router, "GET", "/health", Value::Null, &[])?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[test]
fn test_middleware_scope_is_bounded() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(Deny, |denied| denied.get("locked", health))
            .get("open", health)
    })?;

    assert_eq!(
        send(&manager, &router, "GET", "/locked", Value::Null, &[])?.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&manager, &router, "GET", "/open", Value::Null, &[])?.status(),
        StatusCode::OK
    );

    Ok(())
}

#[test]
fn test_middleware_at_scopes_and_guards() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware_at("api", RequireHeader("x-key"), |api| {
            api.get("health", health)
        })
    })?;

    assert_eq!(
        send(&manager, &router, "GET", "/api/health", Value::Null, &[])?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/api/health",
            Value::Null,
            &[("x-key", "present")],
        )?
        .status(),
        StatusCode::OK
    );

    Ok(())
}

#[test]
fn test_resource_middleware_guard_falls_through() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.middleware(RequireResourceHeader("x-key"), |guarded| {
                    guarded.related("comments")
                })
            })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/comments",
            Value::Null,
            &[]
        )?
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/1/comments",
            Value::Null,
            &[("x-key", "present")],
        )?
        .status(),
        StatusCode::OK
    );

    Ok(())
}

#[test]
fn test_resource_middleware_rewrites_response() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.middleware(StampResourceResponse, |wrapped| wrapped.related("comments"))
            })
    })?;

    let response = send(
        &manager,
        &router,
        "GET",
        "/articles/1/comments",
        Value::Null,
        &[],
    )?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-stamp")
            .and_then(|value| value.to_str().ok()),
        Some("seen")
    );

    Ok(())
}

#[test]
fn test_resource_middleware_short_circuits_handler() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Comments>("comments", comments)
            .resource_with::<Articles>("articles", articles, |articles| {
                articles.middleware(DenyResource, |wrapped| wrapped.related("comments"))
            })
    })?;

    let response = send(
        &manager,
        &router,
        "GET",
        "/articles/1/comments",
        Value::Null,
        &[],
    )?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

// --- JSON:API boundary (the crossing) --------------------------------------

#[test]
fn test_crossing_stamps_jsonapi_content_type() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/vnd.api+json")
    );

    Ok(())
}

#[test]
fn test_crossing_rejects_parameterised_content_type() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    let response = send(
        &manager,
        &router,
        "POST",
        "/articles",
        json!({ "data": { "type": "articles", "attributes": { "title": "X", "body": "Y" } } }),
        &[("Content-Type", "application/vnd.api+json; charset=utf-8")],
    )?;

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    Ok(())
}

#[test]
fn test_crossing_rejects_parameterised_accept() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    let response = send(
        &manager,
        &router,
        "GET",
        "/articles",
        Value::Null,
        &[("Accept", "application/vnd.api+json; charset=utf-8")],
    )?;

    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);

    Ok(())
}

#[test]
fn test_crossing_renders_error_as_document() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/999", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(body(&response)["errors"].is_array());
    assert_eq!(body(&response).get("data"), None);

    Ok(())
}

#[test]
fn test_crossing_rejects_unparseable_body() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    // Genuinely malformed JSON (not merely the wrong shape) is a syntax error — a 400 at the parse.
    // Built by hand: `send` serialises a `Value`, which is always well-formed.
    let stream: ByteStream = Box::new(Cursor::new(b"{ this is not json".to_vec()));
    let request = http::Request::builder()
        .method("POST")
        .uri("/articles")
        .header("content-type", "application/vnd.api+json")
        .body(stream)?;

    let status = router.handle(&manager, request)?.status();

    assert_eq!(status, StatusCode::BAD_REQUEST);

    Ok(())
}

#[test]
fn test_crossing_accepts_a_clean_jsonapi_instance_among_parameterised() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    // One instance is parameterised (and so ignored), but a clean instance remains — acceptable.
    let response = send(
        &manager,
        &router,
        "GET",
        "/articles",
        Value::Null,
        &[(
            "Accept",
            "application/vnd.api+json; charset=utf-8, application/vnd.api+json",
        )],
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_crossing_accepts_a_profile_in_content_type() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    // `profile` is an allowed media type parameter; an unrecognised one is ignored, not rejected.
    let response = send(
        &manager,
        &router,
        "POST",
        "/articles",
        json!({ "data": { "type": "articles", "attributes": { "title": "P", "body": "Q" } } }),
        &[(
            "Content-Type",
            "application/vnd.api+json; profile=\"https://example.com/p\"",
        )],
    )?;

    assert_eq!(response.status(), StatusCode::CREATED);

    Ok(())
}

#[test]
fn test_crossing_accepts_a_profile_in_accept() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    // An unrecognised `profile` in `Accept` must be ignored, leaving the instance acceptable.
    let response = send(
        &manager,
        &router,
        "GET",
        "/articles",
        Value::Null,
        &[(
            "Accept",
            "application/vnd.api+json; profile=\"https://example.com/p\"",
        )],
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_crossing_rejects_an_unsupported_extension_in_content_type() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    // We support no extensions, so any `ext` URI is unsupported and the content type is refused.
    let response = send(
        &manager,
        &router,
        "POST",
        "/articles",
        json!({ "data": { "type": "articles", "attributes": { "title": "X", "body": "Y" } } }),
        &[(
            "Content-Type",
            "application/vnd.api+json; ext=\"https://example.com/ext\"",
        )],
    )?;

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    Ok(())
}

#[test]
fn test_crossing_rejects_an_unsupported_extension_in_accept() -> TestResult {
    let manager = manager()?;
    let router = standard_router(&manager)?;
    // The sole acceptable instance demands an unsupported extension, so none is acceptable.
    let response = send(
        &manager,
        &router,
        "GET",
        "/articles",
        Value::Null,
        &[(
            "Accept",
            "application/vnd.api+json; ext=\"https://example.com/ext\"",
        )],
    )?;

    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);

    Ok(())
}

// --- middleware composition & the byte tier --------------------------------

// A raw route streaming an actual byte payload — the byte tier's reason for being.
fn download(_context: PrimaryContext<'_, '_, SqliteAdapter>) -> PrimaryResult {
    let payload: ByteStream = Box::new(Cursor::new(b"raw payload".to_vec()));
    respond_with(StatusCode::OK, Some(payload)).map_err(Into::into)
}

/// A primary middleware that fails outright — its error must escape `handle` to the embedder rather
/// than being rendered as a response.
struct Fault;
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for Fault {
    fn handle<'req>(
        &self,
        _context: PrimaryContext<'sch, 'req, SqliteAdapter>,
        _next: &PrimaryHandler<'sch, 'req, SqliteAdapter>,
    ) -> PrimaryResult
    where
        'sch: 'req,
    {
        Err("middleware fault".into())
    }
}

/// A primary around-filter appending its mark to an order header on the way back, so a stack of
/// them records the order in which the chain unwinds.
struct AppendMark(&'static str);
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for AppendMark {
    fn handle<'req>(
        &self,
        context: PrimaryContext<'sch, 'req, SqliteAdapter>,
        next: &PrimaryHandler<'sch, 'req, SqliteAdapter>,
    ) -> PrimaryResult
    where
        'sch: 'req,
    {
        let mut response = next(context)?;
        let order = response
            .headers()
            .get("x-order")
            .and_then(|value| value.to_str().ok())
            .map(|existing| format!("{existing},{}", self.0))
            .unwrap_or_else(|| self.0.to_string());
        response
            .headers_mut()
            .insert("x-order", HeaderValue::from_str(&order)?);
        Ok(response)
    }
}

#[test]
fn test_primary_fault_escapes_to_embedder() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(Fault, |root| root.get("health", health))
    })?;

    let stream: ByteStream = Box::new(Cursor::new(Vec::new()));
    let request = http::Request::builder()
        .method("GET")
        .uri("/health")
        .body(stream)?;

    assert!(router.handle(&manager, request).is_err());

    Ok(())
}

#[test]
fn test_middleware_stacks_in_nesting_order() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(AppendMark("outer"), |outer| {
            outer.middleware(AppendMark("inner"), |inner| inner.get("health", health))
        })
    })?;

    let response = send(&manager, &router, "GET", "/health", Value::Null, &[])?;
    // The chain unwinds innermost-first, so `inner` marks the response before `outer`.
    assert_eq!(
        response
            .headers()
            .get("x-order")
            .and_then(|value| value.to_str().ok()),
        Some("inner,outer")
    );

    Ok(())
}

#[test]
fn test_primary_middleware_wraps_resource() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireHeader("x-key"), |guarded| {
            guarded.resource::<Articles>("articles", articles)
        })
    })?;

    // The primary guard fronts the resource's own CRUD: without the header, the collection 404s.
    assert_eq!(
        send(&manager, &router, "GET", "/articles", Value::Null, &[])?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles",
            Value::Null,
            &[("x-key", "present")],
        )?
        .status(),
        StatusCode::OK
    );

    Ok(())
}

#[test]
fn test_raw_route_streams_body() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| root.get("download", download))?;

    let response = send(&manager, &router, "GET", "/download", Value::Null, &[])?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(String::from_utf8(response.body().clone())?, "raw payload");

    Ok(())
}

#[test]
fn test_raw_route_omits_jsonapi_content_type() -> TestResult {
    let manager = manager()?;
    let router = Router::try_new(BaseUri::Relative, |root| root.get("health", health))?;

    let response = send(&manager, &router, "GET", "/health", Value::Null, &[])?;
    assert_eq!(response.status(), StatusCode::OK);
    // The primary tier carries no implicit JSON:API headers; the content type is the crossing's.
    assert_eq!(response.headers().get("content-type"), None);

    Ok(())
}

#[test]
fn test_resource_middleware_guards_custom_route() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles.middleware(RequireResourceHeader("x-key"), |guarded| {
                guarded.get("search", search)
            })
        })
    })?;

    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/search",
            Value::Null,
            &[]
        )?
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &manager,
            &router,
            "GET",
            "/articles/search",
            Value::Null,
            &[("x-key", "present")],
        )?
        .status(),
        StatusCode::OK
    );

    Ok(())
}

// --- wildcard routes -------------------------------------------------------

#[test]
fn test_wildcard_matches_the_tail_and_captures_it() -> TestResult {
    let manager = manager()?;
    // `*path` captures one-or-more trailing segments, joined, under `path`; the handler echoes them.
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*path",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let tail = context
                    .route_parameters()
                    .get("path")
                    .cloned()
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(tail.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let deep = send(&manager, &router, "GET", "/files/a/b/c", Value::Null, &[])?;
    assert_eq!(deep.status(), StatusCode::OK);
    assert_eq!(String::from_utf8(deep.body().clone())?, "a/b/c");

    // A single trailing segment still matches.
    assert_eq!(
        send(&manager, &router, "GET", "/files/a", Value::Null, &[])?.status(),
        StatusCode::OK
    );

    // Zero trailing segments do not: a wildcard is one-or-more.
    assert_eq!(
        send(&manager, &router, "GET", "/files", Value::Null, &[])?.status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn test_wildcard_reaches_the_resource_tier() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    // A resource-tier catch-all, declared last so it only claims paths no canonical route matched.
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles
                .default_endpoints()
                .all_relationships()
                .get("*rest", search)
        })
    })?;

    // A canonical route still wins its slot.
    assert_eq!(
        send(&manager, &router, "GET", "/articles/1", Value::Null, &[])?.status(),
        StatusCode::OK
    );

    // A deep path no canonical route claims falls to the wildcard and crosses the JSON:API boundary.
    let caught = send(
        &manager,
        &router,
        "GET",
        "/articles/1/no/such/path",
        Value::Null,
        &[],
    )?;
    assert_eq!(caught.status(), StatusCode::OK);
    assert_eq!(
        caught
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/vnd.api+json")
    );

    Ok(())
}

#[test]
fn test_misplaced_wildcard_is_rejected() -> TestResult {
    let result = Router::try_new(BaseUri::Relative, |root| root.get("*mid/tail", health));

    assert!(matches!(result, Err(RouterError::MisplacedGlob { .. })));

    Ok(())
}
