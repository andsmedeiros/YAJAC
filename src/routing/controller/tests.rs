use super::{Configuration, ResourceContext, ResourceController};
use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::connection_manager::ConnectionManager;
use crate::database::registry::Registry;
use crate::database::schema::{AttributeType, Related, Schema, SchemaBuilder};
use crate::http_wrappers::Uri;
use crate::json_api::document::Document;
use crate::routing::ControllerLookup;
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

// A controller that accepts client-generated ids, for the two tests below.
#[derive(Default)]
struct ClientIdBooks;
impl<'sch> ResourceController<'sch, SqliteAdapter> for ClientIdBooks {
    fn configuration(&self) -> Configuration {
        Configuration {
            accepts_client_ids: true,
        }
    }
}

#[test]
fn test_create_refuses_unaccepted_client_generated_id() -> TestResult {
    let manager = manager()?;
    let request = build_request(
        "POST",
        "/books",
        json!({ "data": { "type": "books", "id": "42", "attributes": { "title": "Four" } } }),
    )?;
    let uri: Uri = request.uri().clone().into();
    let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

    match Books::default().create(ResourceContext::new(schema(&manager, "books"), context)) {
        Ok(_) => Err("an unaccepted client-generated id must be refused".into()),
        Err(error) => {
            let status: StatusCode = error.status_code().into();
            assert_eq!(status, StatusCode::FORBIDDEN);
            Ok(())
        }
    }
}

#[test]
fn test_create_honours_accepted_client_generated_id() -> TestResult {
    let manager = manager()?;
    let request = build_request(
        "POST",
        "/books",
        json!({ "data": { "type": "books", "id": "42", "attributes": { "title": "Four" } } }),
    )?;
    let uri: Uri = request.uri().clone().into();
    let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

    let created =
        ClientIdBooks::default().create(ResourceContext::new(schema(&manager, "books"), context))?;

    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(body(&created)["data"]["id"], json!("42"));

    // The client's id resolves to the stored record on a fresh read.
    let request = build_request("GET", "/books/42", Value::Null)?;
    let uri: Uri = request.uri().clone().into();
    let context = Context::from_request(&manager, &uri, route_id("42"), request);
    let fetched =
        Books::default().show(ResourceContext::new(schema(&manager, "books"), context))?;

    assert_eq!(fetched.status(), StatusCode::OK);
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
fn test_create_rejects_non_resource_document() -> TestResult {
    let manager = manager()?;
    // A well-formed document whose primary data is a collection, not a single resource.
    let request = build_request("POST", "/books", json!({ "data": [] }))?;
    let uri: Uri = request.uri().clone().into();
    let context = Context::from_request(&manager, &uri, RouteParameters::new(), request);

    match Books::default().create(ResourceContext::new(schema(&manager, "books"), context)) {
        Ok(_) => Err("a non-resource document must error".into()),
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
fn controllers<'sch>() -> ControllerLookup<'sch, SqliteAdapter> {
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
