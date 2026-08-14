use super::*;
use test_log::test;

#[test]
fn create_persists_the_submitted_record() -> Result {
    let manager = database::build_database([("authors", fixtures::authors::ann()?)])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "articles",
            "attributes": { "title": "A Late Draft", "body": "Written in one sitting." },
            "relationships": { "author": { "data": { "type": "authors", "id": "1" } } }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let response = Articles.create(context)?;
    let record = require_record(&response)?;
    let title = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("title"));
    let author = record
        .relationships
        .as_ref()
        .and_then(|relationships| relationships.get("author"))
        .map(serde_json::to_value)
        .transpose()?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(title, Some(&json!("A Late Draft")));
    assert_eq!(
        author,
        Some(json!({
            "links": {
                "self": "/articles/1/relationships/author",
                "related": "/articles/1/author"
            },
            "data": { "type": "authors", "id": "1" }
        }))
    );

    Ok(())
}

#[test]
fn create_rejects_a_type_that_is_not_the_addressed_resource() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "attributes": { "title": "Wrong" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceTypeMismatch".to_string()));

    Ok(())
}

#[test]
fn create_rejects_an_attribute_the_schema_does_not_declare() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "attributes": { "headline": "Nope" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("UnknownAttribute".to_string()));

    Ok(())
}

#[test]
fn create_refuses_a_client_generated_id_a_controller_does_not_accept() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "id": "42", "attributes": { "title": "Numbered" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::FORBIDDEN));
    assert_eq!(code, Some("ClientGeneratedIdNotSupported".to_string()));

    Ok(())
}

#[test]
fn create_honours_a_client_generated_id_a_controller_accepts() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "publishers",
            "id": "verso-books",
            "attributes": { "name": "Verso Books" }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/publishers")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let response = Publishers.create(context)?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "publishers".to_string(),
            id: "verso-books".to_string()
        }
    );

    Ok(())
}

#[test]
fn update_changes_the_submitted_attributes() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "id": "1", "attributes": { "title": "Retitled" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let response = Articles.update(context)?;
    let record = require_record(&response)?;
    let attributes = record.attributes.as_ref();
    let title = attributes.and_then(|attributes| attributes.get("title"));
    let body = attributes.and_then(|attributes| attributes.get("body"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(title, Some(&json!("Retitled")));
    assert_eq!(
        body,
        Some(&json!("A study of provenance in layered systems."))
    );

    Ok(())
}

#[test]
fn update_rejects_an_id_that_is_not_the_addressed_one() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceIdMismatch".to_string()));

    Ok(())
}

#[test]
fn delete_removes_the_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let deleted = Articles.delete(context)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;
    let (status, code) = Articles
        .show(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(deleted.body().as_ref(), None);
    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn create_attaches_a_to_many_the_submitted_record_carries() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "authors",
            "attributes": { "name": "Cleo Nakamura", "handle": "cleo" },
            "relationships": {
                "articles": { "data": [{ "type": "articles", "id": "3" }] }
            }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.create(context)?;
    let record = require_record(&response)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/3/relationships/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;
    let linkage = Authors.linkage(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&linkage)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "3".to_string()
        }
    );
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "3".to_string()
        }]
    );

    Ok(())
}

/// The new author's `handle` is written into the profile's `author_handle`, a key the submitted
/// linkage never names.
#[test]
fn create_attaches_a_has_one_joined_by_a_non_primary_key() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "authors",
            "attributes": { "name": "Cleo Nakamura", "handle": "cleo" },
            "relationships": {
                "profile": { "data": { "type": "profiles", "id": "1" } }
            }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("authors")?,
    )?;

    let response = Authors.create(context)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/profiles/1/author")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("profiles")?,
    )?;
    let related = Profiles.related(context, "author")?;
    let author = require_record(&related)?;
    let handle = author
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("handle"));

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        author.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "3".to_string()
        }
    );
    assert_eq!(handle, Some(&json!("cleo")));

    Ok(())
}

#[test]
fn create_rejects_a_document_whose_primary_data_is_not_a_resource() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("PrimaryDataIsNotAResource".to_string()));

    Ok(())
}

#[test]
fn create_rejects_an_errors_document() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "errors": [{ "title": "Nothing went wrong here" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ErrorDocumentSubmitted".to_string()));

    Ok(())
}

#[test]
fn create_rejects_a_body_that_is_not_json() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(Cursor::new(br#"{"data": }"#.to_vec()));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::BAD_REQUEST));
    assert_eq!(code, Some("MalformedRequestBody".to_string()));

    Ok(())
}

#[test]
fn create_rejects_json_that_is_not_a_document() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": 7 } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidRequestBodyContent".to_string()));

    Ok(())
}

#[test]
fn create_without_a_body_is_rejected() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .create(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingResourceBody".to_string()));

    Ok(())
}

#[test]
fn update_changes_a_belongs_to_the_submitted_record_carries() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "articles",
            "id": "1",
            "relationships": {
                "author": { "data": { "type": "authors", "id": "2" } }
            }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let response = Articles.update(context)?;
    let record = require_record(&response)?;
    let title = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("title"));

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1/relationships/author")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;
    let linkage = Articles.linkage(context, "author")?;
    let author = require_record(&linkage)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(title, Some(&json!("On Borrowed Lifetimes")));
    assert_eq!(
        author.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "2".to_string()
        }
    );

    Ok(())
}

#[test]
fn update_of_an_absent_record_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "id": "404", "attributes": { "title": "Ghostwritten" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/404")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn update_rejects_a_type_that_is_not_the_addressed_resource() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceTypeMismatch".to_string()));

    Ok(())
}

#[test]
fn update_rejects_a_resource_carrying_no_id() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": { "type": "articles", "attributes": { "title": "Anonymous" } }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::CONFLICT));
    assert_eq!(code, Some("ResourceIdMissing".to_string()));

    Ok(())
}

#[test]
fn update_without_a_body_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .update(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingResourceBody".to_string()));

    Ok(())
}

#[test]
fn delete_of_an_absent_record_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/articles/404")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = Articles
        .delete(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn update_of_a_text_keyed_record_changes_the_submitted_attributes() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "publishers",
            "id": "acme-press",
            "attributes": { "name": "Acme Press International" }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let response = Publishers.update(context)?;
    let record = require_record(&response)?;
    let name = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("name"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "publishers".to_string(),
            id: "acme-press".to_string()
        }
    );
    assert_eq!(name, Some(&json!("Acme Press International")));

    Ok(())
}

#[test]
fn delete_of_a_text_keyed_record_removes_it() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("publishers", fixtures::publishers::acme()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;

    let deleted = Publishers.delete(context)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/publishers/acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("publishers")?,
    )?;
    let (status, code) = Publishers
        .show(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(deleted.body().as_ref(), None);
    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}
