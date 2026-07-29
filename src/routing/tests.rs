use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::connection_manager::ConnectionManager;
use crate::database::registry::Registry as DatabaseRegistry;
use crate::database::schema::{AttributeType, Related, SchemaBuilder};
use crate::json_api::document::Document;
use crate::routing::controller::{ResourceContext, ResourceController};
use crate::routing::responder::respond_with;
use crate::routing::{Context, Result as RouteResult, Router, RouterError, UnboundVerbs};
use http::{Response, StatusCode};
use serde_json::{Value, json};
use std::error::Error as StdError;

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

// An unbound leaf route: schema-oblivious, receives a bare `Context`.
fn health(_context: Context<'_, '_, SqliteAdapter>) -> RouteResult {
    respond_with(StatusCode::OK, None)
}

// The standard resourceful mount: every resource bare, so CRUD and the full set of relationship
// endpoints are auto-enumerated from each schema.
fn standard_router(manager: &Manager) -> Result<Router<'_, SqliteAdapter>, Box<dyn StdError>> {
    let articles = manager.registry().schema("articles")?;
    let comments = manager.registry().schema("comments")?;
    let drafts = manager.registry().schema("drafts")?;
    Ok(Router::try_new(|root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Comments>("comments", comments)
            .resource::<Drafts>("drafts", drafts)
    })?)
}

fn read_only_router(manager: &Manager) -> Result<Router<'_, SqliteAdapter>, Box<dyn StdError>> {
    let summaries = manager.registry().schema("summaries")?;
    Ok(Router::try_new(|root| {
        root.read_only_resource::<Summaries>("summaries", summaries)
    })?)
}

fn send<'a>(
    manager: &'a Manager,
    router: &Router<'a, SqliteAdapter>,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<Response<Option<Document>>, Box<dyn StdError>> {
    let request = http::Request::builder()
        .method(method)
        .uri(uri)
        .body(serde_json::to_vec(&body)?)?;

    Ok(router.handle(manager, request))
}

fn serve(
    manager: &Manager,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<Response<Option<Document>>, Box<dyn StdError>> {
    send(manager, &standard_router(manager)?, method, uri, body)
}

fn read_only_serve(
    manager: &Manager,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<Response<Option<Document>>, Box<dyn StdError>> {
    send(manager, &read_only_router(manager)?, method, uri, body)
}

fn body(response: &Response<Option<Document>>) -> Value {
    serde_json::to_value(response.body()).expect("a serialisable document")
}

fn data_ids(response: &Response<Option<Document>>) -> Vec<Value> {
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
    let router = Router::try_new(|root| {
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
            Value::Null
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
            Value::Null
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
    let router = Router::try_new(|root| {
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
            Value::Null
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
            Value::Null
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
    let router = Router::try_new(|root| {
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
            Value::Null
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
            Value::Null
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
            Value::Null
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
    let router = Router::try_new(|root| {
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
            Value::Null
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
    let router = Router::try_new(|root| {
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
            Value::Null
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
            Value::Null
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
    let router = Router::try_new(|root| {
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
            Value::Null
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
            Value::Null
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
    let router = Router::try_new(|root| {
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
    )?;

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    Ok(())
}

#[test]
fn test_collection_route_is_collection_scoped() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let router = Router::try_new(|root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles.collection(|collection| collection.get("search", search))
        })
    })?;

    let response = send(&manager, &router, "GET", "/articles/search", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn test_unbound_leaf_route() -> TestResult {
    let manager = manager()?;
    let articles = manager.registry().schema("articles")?;
    let router = Router::try_new(|root| {
        root.resource_with::<Articles>("articles", articles, |articles| {
            articles.get("health", health)
        })
    })?;

    let response = send(&manager, &router, "GET", "/articles/health", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);

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
    let result = Router::try_new(|root| {
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
    let result = Router::try_new(|root| {
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
    let result = Router::try_new(|root| {
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
    let result = Router::try_new(|root| {
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
    let result = Router::try_new(|root| root.resource::<Articles>("articles", articles));

    assert!(result.is_ok());

    Ok(())
}
