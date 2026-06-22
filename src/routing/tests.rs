use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::registry::Registry as DatabaseRegistry;
use crate::database::schema::{
    AttributeType, IdentifierType, PrimaryKey, RelatedResource, Relationship, RelationshipKeys,
    TableSchema,
};
use crate::json_api::document::Document;
use crate::routing::RouterBuilder;
use crate::routing::controller::ResourceController;
use http::StatusCode;
use serde_json::{Value, json};
use std::error::Error as StdError;

type Registry = DatabaseRegistry<'static, SqliteAdapter>;

type TestResult = Result<(), Box<dyn StdError>>;

static ARTICLES: TableSchema = TableSchema {
    name: "articles",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[
        ("title", AttributeType::Text),
        ("body", AttributeType::Text),
    ],
    foreign_keys: &[],
    relationships: &[
        (
            "comments",
            Relationship::HasMany(RelatedResource {
                resource: "comments",
                keys: RelationshipKeys {
                    own: "id",
                    related: "article_id",
                },
            }),
        ),
        (
            "drafts",
            Relationship::HasMany(RelatedResource {
                resource: "drafts",
                keys: RelationshipKeys {
                    own: "id",
                    related: "article_id",
                },
            }),
        ),
        (
            "summary",
            Relationship::HasOne(RelatedResource {
                resource: "summaries",
                keys: RelationshipKeys {
                    own: "id",
                    related: "article_id",
                },
            }),
        ),
    ],
    text_index: false,
};

static COMMENTS: TableSchema = TableSchema {
    name: "comments",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[("content", AttributeType::Text)],
    foreign_keys: &[("article_id", AttributeType::Integer)],
    relationships: &[(
        "article",
        Relationship::BelongsTo(RelatedResource {
            resource: "articles",
            keys: RelationshipKeys {
                own: "article_id",
                related: "id",
            },
        }),
    )],
    text_index: false,
};

static DRAFTS: TableSchema = TableSchema {
    name: "drafts",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[("title", AttributeType::Text)],
    foreign_keys: &[("article_id", AttributeType::Integer)],
    relationships: &[(
        "article",
        Relationship::BelongsTo(RelatedResource {
            resource: "articles",
            keys: RelationshipKeys {
                own: "article_id",
                related: "id",
            },
        }),
    )],
    text_index: false,
};

static SUMMARIES: TableSchema = TableSchema {
    name: "summaries",
    primary_key: PrimaryKey {
        name: "id",
        kind: IdentifierType::Integer,
    },
    attributes: &[("abstract", AttributeType::Text)],
    foreign_keys: &[("article_id", AttributeType::Integer)],
    relationships: &[(
        "article",
        Relationship::BelongsTo(RelatedResource {
            resource: "articles",
            keys: RelationshipKeys {
                own: "article_id",
                related: "id",
            },
        }),
    )],
    text_index: false,
};

static SCHEMAS: [&TableSchema; 4] = [&ARTICLES, &COMMENTS, &DRAFTS, &SUMMARIES];

struct Articles;
struct Comments;
struct Drafts;

impl<'sch> ResourceController<'sch, SqliteAdapter> for Articles {
    fn resource_schema() -> &'sch TableSchema<'sch> {
        &ARTICLES
    }
}

impl<'sch> ResourceController<'sch, SqliteAdapter> for Comments {
    fn resource_schema() -> &'sch TableSchema<'sch> {
        &COMMENTS
    }
}

impl<'sch> ResourceController<'sch, SqliteAdapter> for Drafts {
    fn resource_schema() -> &'sch TableSchema<'sch> {
        &DRAFTS
    }
}

fn registry() -> Result<Registry, Box<dyn StdError>> {
    let registry = Registry::try_new(Pool::memory()?, &SCHEMAS)?;

    registry.acquire()?.execute_batch(
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

    Ok(registry)
}

fn serve(
    registry: &Registry,
    method: &str,
    uri: &str,
    body: Value,
) -> Result<http::Response<Option<Document>>, Box<dyn StdError>> {
    let mut builder = RouterBuilder::new();
    builder
        .resource::<Articles>("articles")
        .resource::<Comments>("comments")
        .resource::<Drafts>("drafts");
    let router = builder.build();

    let request = http::Request::builder()
        .method(method)
        .uri(uri)
        .body(serde_json::to_vec(&body)?)?;

    Ok(router.handle(registry, request))
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
    let registry = registry()?;
    let response = serve(&registry, "GET", "/articles", Value::Null)?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body(&response);
    let data = document["data"].as_array().expect("a data array");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["type"], json!("articles"));

    Ok(())
}

#[test]
fn test_show() -> TestResult {
    let registry = registry()?;
    let response = serve(&registry, "GET", "/articles/1", Value::Null)?;

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
    let registry = registry()?;
    let response = serve(&registry, "GET", "/articles/999", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_show_includes_to_many() -> TestResult {
    let registry = registry()?;
    let response = serve(
        &registry,
        "GET",
        "/articles/1?include=comments",
        Value::Null,
    )?;

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
    let registry = registry()?;
    let response = serve(&registry, "GET", "/comments/1?include=article", Value::Null)?;

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
    let registry = registry()?;
    let response = serve(&registry, "GET", "/widgets", Value::Null)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_invalid_query_field() -> TestResult {
    let registry = registry()?;
    let response = serve(
        &registry,
        "GET",
        "/articles?fields[articles]=bogus",
        Value::Null,
    )?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[test]
fn test_delete() -> TestResult {
    let registry = registry()?;

    let deleted = serve(&registry, "DELETE", "/comments/1", Value::Null)?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let fetched = serve(&registry, "GET", "/comments/1", Value::Null)?;
    assert_eq!(fetched.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn test_create() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let fetched = serve(&registry, "GET", "/articles/3", Value::Null)?;
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        body(&fetched)["data"]["attributes"]["title"],
        json!("Third")
    );

    Ok(())
}

#[test]
fn test_create_with_belongs_to_relationship() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let fetched = serve(&registry, "GET", "/comments/3", Value::Null)?;
    assert_eq!(linkage_id(&fetched, "article"), json!("2"));

    Ok(())
}

#[test]
fn test_create_with_to_many_relationship() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let comment = serve(&registry, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&comment, "article"), json!("3"));

    Ok(())
}

#[test]
fn test_create_rejects_type_mismatch() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
        "POST",
        "/articles",
        json!({ "data": { "type": "comments", "attributes": { "title": "Wrong" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_create_rejects_unknown_attribute() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
        "POST",
        "/articles",
        json!({ "data": { "type": "articles", "attributes": { "bogus": "x" } } }),
    )?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[test]
fn test_create_rejects_malformed_document() -> TestResult {
    let registry = registry()?;

    let response = serve(&registry, "POST", "/articles", json!({ "title": "Naked" }))?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    Ok(())
}

#[test]
fn test_update() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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
    let registry = registry()?;

    let response = serve(
        &registry,
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
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let fetched = serve(&registry, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&fetched, "article"), json!("2"));

    Ok(())
}

#[test]
fn test_patch_clears_nullable_belongs_to() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let fetched = serve(&registry, "GET", "/drafts/1", Value::Null)?;
    assert!(linkage_id(&fetched, "article").is_null());

    Ok(())
}

#[test]
fn test_patch_replaces_to_many_relationship() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let moved = serve(&registry, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&moved, "article"), json!("2"));

    let kept = serve(&registry, "GET", "/comments/2", Value::Null)?;
    assert_eq!(linkage_id(&kept, "article"), json!("1"));

    Ok(())
}

#[test]
fn test_patch_replaces_nullable_to_many_detaching_dropped_members() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let article = serve(&registry, "GET", "/articles/1", Value::Null)?;
    assert_eq!(linkage_ids(&article, "drafts"), vec![json!("1")]);

    let kept = serve(&registry, "GET", "/drafts/1", Value::Null)?;
    assert_eq!(linkage_id(&kept, "article"), json!("1"));

    let dropped = serve(&registry, "GET", "/drafts/2", Value::Null)?;
    assert!(linkage_id(&dropped, "article").is_null());

    Ok(())
}

#[test]
fn test_patch_clears_nullable_to_many() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let first = serve(&registry, "GET", "/drafts/1", Value::Null)?;
    assert!(linkage_id(&first, "article").is_null());

    let second = serve(&registry, "GET", "/drafts/2", Value::Null)?;
    assert!(linkage_id(&second, "article").is_null());

    Ok(())
}

#[test]
fn test_patch_clearing_required_to_many_conflicts() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
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
    let registry = registry()?;

    let response = serve(
        &registry,
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
    let registry = registry()?;

    let response = serve(
        &registry,
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

    let fetched = serve(&registry, "GET", "/comments/1", Value::Null)?;
    assert_eq!(linkage_id(&fetched, "article"), json!("1"));

    Ok(())
}

#[test]
fn test_patch_rejects_type_mismatch() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
        "PATCH",
        "/articles/1",
        json!({ "data": { "type": "comments", "id": "1" } }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[test]
fn test_patch_rejects_id_mismatch() -> TestResult {
    let registry = registry()?;

    let response = serve(
        &registry,
        "PATCH",
        "/articles/1",
        json!({ "data": { "type": "articles", "id": "2" } }),
    )?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}
