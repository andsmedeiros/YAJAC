use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::connection_manager::ConnectionManager;
use crate::database::registry::Registry as DatabaseRegistry;
use crate::database::schema::{AttributeType, Related, SchemaBuilder};
use crate::json_api::document::Document;
use crate::routing::RouterBuilder;
use crate::routing::controller::{ReadOnlyResourceController, ResourceController};
use http::StatusCode;
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

struct Articles;
struct Comments;
struct Drafts;
struct Summaries;

impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {}

impl<'sch> ResourceController<'sch, SqliteAdapter> for Comments {}

impl<'sch> ResourceController<'sch, SqliteAdapter> for Drafts {}

impl<'sch> ReadOnlyResourceController<'sch, SqliteAdapter> for Summaries {}

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

fn serve(
    manager: &Manager,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<http::Response<Option<Document>>, Box<dyn StdError>> {
    let mut builder = RouterBuilder::new();
    builder
        .resource::<Articles>("articles", manager.registry().schema("articles")?)
        .resource::<Comments>("comments", manager.registry().schema("comments")?)
        .resource::<Drafts>("drafts", manager.registry().schema("drafts")?);
    let router = builder.build();

    let request = http::Request::builder()
        .method(method)
        .uri(uri)
        .body(serde_json::to_vec(&body)?)?;

    Ok(router.handle(manager, request))
}

fn serve_read_only(
    manager: &Manager,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<http::Response<Option<Document>>, Box<dyn StdError>> {
    let mut builder = RouterBuilder::new();
    builder.read_only_resource::<Summaries>("summaries", manager.registry().schema("summaries")?);
    let router = builder.build();

    let request = http::Request::builder()
        .method(method)
        .uri(uri)
        .body(serde_json::to_vec(&body)?)?;

    Ok(router.handle(manager, request))
}

fn body(response: &http::Response<Option<Document>>) -> Value {
    serde_json::to_value(response.body()).expect("a serialisable document")
}

fn linkage_id(response: &http::Response<Option<Document>>, relationship: &str) -> Value {
    body(response)["data"]["relationships"][relationship]["data"]["id"].clone()
}

fn linkage_ids(response: &http::Response<Option<Document>>, relationship: &str) -> Vec<Value> {
    body(response)["data"]["relationships"][relationship]["data"]
        .as_array()
        .map(|members| members.iter().map(|member| member["id"].clone()).collect())
        .unwrap_or_default()
}

#[test]
fn test_index() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    let data = document["data"].as_array().expect("a data array");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["type"], json!("articles"));

    Ok(())
}

#[test]
fn test_show() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    let data = &document["data"];
    assert_eq!(data["type"], json!("articles"));
    assert_eq!(data["id"], json!("1"));
    assert_eq!(data["attributes"]["title"], json!("First"));

    Ok(())
}

#[test]
fn test_show_missing_record() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/999", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_show_includes_to_many() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/articles/1?include=comments", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    let included = document["included"].as_array().expect("an included array");
    assert_eq!(included.len(), 2);
    assert!(
        included
            .iter()
            .all(|resource| resource["type"] == json!("comments"))
    );

    Ok(())
}

#[test]
fn test_show_includes_to_one() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/comments/1?include=article", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    let included = document["included"].as_array().expect("an included array");
    assert_eq!(included.len(), 1);
    assert_eq!(included[0]["type"], json!("articles"));
    assert_eq!(included[0]["id"], json!("1"));

    Ok(())
}

#[test]
fn test_unknown_route() -> TestResult {
    let manager = manager()?;
    let response = serve(&manager, "GET", "/widgets", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_invalid_query_field() -> TestResult {
    let manager = manager()?;
    let response = serve(
        &manager,
        "GET",
        "/articles?fields[articles]=bogus",
        Value::Null,
    )?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[test]
fn test_delete() -> TestResult {
    let manager = manager()?;

    let deleted = serve(&manager, "DELETE", "/comments/1", Value::Null)?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let fetched = serve(&manager, "GET", "/comments/1", Value::Null)?;
    assert_eq!(fetched.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_create() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "POST",
        "/articles",
        json!({
            "data": {
                "type": "articles",
                "attributes": { "title": "Third", "body": "Body three" }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        body(&response)["data"]["attributes"]["title"],
        json!("Third")
    );

    let fetched = serve(&manager, "GET", "/articles/3", Value::Null)?;
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        body(&fetched)["data"]["attributes"]["title"],
        json!("Third")
    );

    Ok(())
}

#[test]
fn test_create_with_belongs_to_relationship() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "POST",
        "/comments",
        json!({
            "data": {
                "type": "comments",
                "attributes": { "content": "Linked" },
                "relationships": {
                    "article": { "data": { "type": "articles", "id": "2" } }
                }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::CREATED);

    let fetched = serve(&manager, "GET", "/comments/3", Value::Null)?;
    assert_eq!(linkage_id(&fetched, "article"), json!("2"));

    Ok(())
}

#[test]
fn test_create_with_to_many_relationship() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "POST",
        "/articles",
        json!({
            "data": {
                "type": "articles",
                "attributes": { "title": "Third", "body": "Body three" },
                "relationships": {
                    "comments": { "data": [{ "type": "comments", "id": "1" }] }
                }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::CREATED);

    let comment = serve(&manager, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&comment, "article"), json!("3"));

    Ok(())
}

#[test]
fn test_create_rejects_type_mismatch() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "POST",
        "/articles",
        json!({ "data": { "type": "comments", "attributes": { "title": "Wrong" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_create_rejects_unknown_attribute() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "POST",
        "/articles",
        json!({ "data": { "type": "articles", "attributes": { "bogus": "x" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    Ok(())
}

#[test]
fn test_create_rejects_malformed_document() -> TestResult {
    let manager = manager()?;

    let response = serve(&manager, "POST", "/articles", json!({ "title": "Naked" }))?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    Ok(())
}

#[test]
fn test_update() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({
            "data": {
                "type": "articles",
                "id": "1",
                "attributes": { "title": "Updated" }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body(&response)["data"]["attributes"]["title"],
        json!("Updated")
    );

    Ok(())
}

#[test]
fn test_update_missing_record() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/999",
        json!({
            "data": { "type": "articles", "id": "999", "attributes": { "title": "Ghost" } }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_patch_belongs_to_relationship() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/comments/1",
        json!({
            "data": {
                "type": "comments",
                "id": "1",
                "relationships": {
                    "article": { "data": { "type": "articles", "id": "2" } }
                }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    let fetched = serve(&manager, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&fetched, "article"), json!("2"));

    Ok(())
}

#[test]
fn test_patch_clears_nullable_belongs_to() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/drafts/1",
        json!({
            "data": {
                "type": "drafts",
                "id": "1",
                "relationships": { "article": { "data": null } }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    let fetched = serve(&manager, "GET", "/drafts/1", Value::Null)?;
    assert!(linkage_id(&fetched, "article").is_null());

    Ok(())
}

#[test]
fn test_patch_replaces_to_many_relationship() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/2",
        json!({
            "data": {
                "type": "articles",
                "id": "2",
                "relationships": {
                    "comments": { "data": [{ "type": "comments", "id": "1" }] }
                }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    let moved = serve(&manager, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&moved, "article"), json!("2"));

    let kept = serve(&manager, "GET", "/comments/2", Value::Null)?;
    assert_eq!(linkage_id(&kept, "article"), json!("1"));

    Ok(())
}

#[test]
fn test_patch_replaces_nullable_to_many_detaching_dropped_members() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({
            "data": {
                "type": "articles",
                "id": "1",
                "relationships": {
                    "drafts": { "data": [{ "type": "drafts", "id": "1" }] }
                }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    let article = serve(&manager, "GET", "/articles/1", Value::Null)?;
    assert_eq!(linkage_ids(&article, "drafts"), vec![json!("1")]);

    let kept = serve(&manager, "GET", "/drafts/1", Value::Null)?;
    assert_eq!(linkage_id(&kept, "article"), json!("1"));

    let dropped = serve(&manager, "GET", "/drafts/2", Value::Null)?;
    assert!(linkage_id(&dropped, "article").is_null());

    Ok(())
}

#[test]
fn test_patch_clears_nullable_to_many() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({
            "data": {
                "type": "articles",
                "id": "1",
                "relationships": { "drafts": { "data": [] } }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    let first = serve(&manager, "GET", "/drafts/1", Value::Null)?;
    assert!(linkage_id(&first, "article").is_null());

    let second = serve(&manager, "GET", "/drafts/2", Value::Null)?;
    assert!(linkage_id(&second, "article").is_null());

    Ok(())
}

#[test]
fn test_patch_clearing_required_to_many_conflicts() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({
            "data": {
                "type": "articles",
                "id": "1",
                "relationships": { "comments": { "data": [] } }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_assign_has_one_to_owned_record_conflicts() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({
            "data": {
                "type": "articles",
                "id": "1",
                "relationships": {
                    "summary": { "data": { "type": "summaries", "id": "2" } }
                }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_patch_leaves_omitted_relationship_unchanged() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/comments/1",
        json!({
            "data": {
                "type": "comments",
                "id": "1",
                "attributes": { "content": "Edited" }
            }
        }),
    )?;

    assert_eq!(response.status(), StatusCode::OK);

    let fetched = serve(&manager, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&fetched, "article"), json!("1"));

    Ok(())
}

#[test]
fn test_patch_rejects_type_mismatch() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({ "data": { "type": "comments", "id": "1" } }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_patch_rejects_id_mismatch() -> TestResult {
    let manager = manager()?;

    let response = serve(
        &manager,
        "PATCH",
        "/articles/1",
        json!({ "data": { "type": "articles", "id": "2" } }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_read_only_index() -> TestResult {
    let manager = manager()?;
    let response = serve_read_only(&manager, "GET", "/summaries", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    let data = document["data"].as_array().expect("a data array");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["type"], json!("summaries"));

    Ok(())
}

#[test]
fn test_read_only_show() -> TestResult {
    let manager = manager()?;
    let response = serve_read_only(&manager, "GET", "/summaries/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    assert_eq!(document["data"]["id"], json!("1"));
    assert_eq!(
        document["data"]["attributes"]["abstract"],
        json!("About first")
    );

    Ok(())
}

#[test]
fn test_read_only_rejects_create() -> TestResult {
    let manager = manager()?;
    let response = serve_read_only(
        &manager,
        "POST",
        "/summaries",
        json!({ "data": { "type": "summaries", "attributes": { "abstract": "New" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_read_only_rejects_delete() -> TestResult {
    let manager = manager()?;
    let response = serve_read_only(&manager, "DELETE", "/summaries/1", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
