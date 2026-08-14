use super::*;
use test_log::test;

#[test]
fn link_adds_the_targets_keeping_the_members_already_linked() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "3" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let response = Authors.link(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "1".to_string()
            },
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "2".to_string()
            },
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "3".to_string()
            }
        ]
    );

    Ok(())
}

#[test]
fn link_of_a_target_that_does_not_exist_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "404" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RelatedRecordNotFound".to_string()));

    Ok(())
}

#[test]
fn link_of_an_absent_parent_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/404/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn link_of_a_to_one_linkage_on_a_to_many_relationship_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "id": "3" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure".to_string()));

    Ok(())
}

#[test]
fn link_of_an_identifier_naming_another_resource_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("IdentifierTypeMismatch".to_string()));

    Ok(())
}

#[test]
fn link_of_an_identifier_that_is_not_an_integer_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "the-first-one" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidIntegerIdentifier".to_string()));

    Ok(())
}

/// Linkage carries resource identifier objects; a resource object bearing attributes is not one.
#[test]
fn link_of_linkage_that_is_not_a_bare_identifier_is_rejected() -> Result {
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
        "data": [{ "type": "articles", "id": "3", "attributes": { "title": "Renamed" } }]
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidLinkage".to_string()));

    Ok(())
}

#[test]
fn link_without_a_body_is_rejected() -> Result {
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
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingLinkageBody".to_string()));

    Ok(())
}

#[test]
fn link_on_a_to_one_relationship_is_a_kind_mismatch() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_miswired_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles/1/author")
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
        .link(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("MismatchedRelationshipKind".to_string()));

    Ok(())
}

#[test]
fn link_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([("authors", fixtures::authors::ann()?)])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .link(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn relink_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": null });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/1/relationships/profile")
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

    let (status, code) = Authors
        .relink(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn relink_replaces_the_members_of_a_to_many() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "3" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/1/relationships/articles")
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

    let response = Authors.relink(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

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
    let detached = Articles.linkage(context, "author")?;
    let content = detached.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "3".to_string()
        }]
    );
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

#[test]
fn relink_of_an_empty_collection_clears_a_to_many() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/1/relationships/articles")
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

    let response = Authors.relink(context, "articles")?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(require_collection(&response)?, []);

    Ok(())
}

#[test]
fn relink_sets_a_belongs_to() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/3/relationships/author")
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

    let response = Articles.relink(context, "author")?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "2".to_string()
        }
    );

    Ok(())
}

#[test]
fn relink_of_null_linkage_clears_a_belongs_to() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": null });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let response = Articles.relink(context, "author")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

/// `authors.profile` is held by the profile's `author_handle`, so setting it writes the far side's
/// key from a value the identifier never carries.
#[test]
fn relink_sets_a_has_one_joined_by_a_non_primary_key() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "profiles", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/authors/2/relationships/profile")
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

    let response = Authors.relink(context, "profile")?;
    let record = require_record(&response)?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/relationships/profile")
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
    let detached = Authors.linkage(context, "profile")?;
    let content = detached.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "profiles".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

#[test]
fn relink_of_a_to_many_linkage_on_a_to_one_relationship_is_rejected() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "authors", "id": "2" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure".to_string()));

    Ok(())
}

#[test]
fn relink_of_a_target_that_does_not_exist_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "404" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RelatedRecordNotFound".to_string()));

    Ok(())
}

#[test]
fn relink_of_an_absent_parent_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/404/relationships/author")
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
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn relink_without_a_body_is_rejected() -> Result {
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

    let (status, code) = Articles
        .relink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingLinkageBody".to_string()));

    Ok(())
}

#[test]
fn unlink_on_a_to_one_relationship_is_a_kind_mismatch() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_miswired_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "authors", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/articles/1/author")
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
        .unlink(context, "author")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("MismatchedRelationshipKind".to_string()));

    Ok(())
}

#[test]
fn unlink_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .unlink(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn unlink_removes_the_targets_from_the_collection() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
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

    let response = Authors.unlink(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "2".to_string()
        }]
    );

    Ok(())
}

#[test]
fn unlink_of_a_target_that_is_not_a_member_leaves_the_collection_whole() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "3" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/1/relationships/articles")
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

    let response = Authors.unlink(context, "articles")?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "1".to_string()
            },
            &Identifier::Existing {
                kind: "articles".to_string(),
                id: "2".to_string()
            }
        ]
    );

    Ok(())
}

#[test]
fn unlink_of_an_absent_parent_is_not_found() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::DELETE)
        .uri("/authors/404/relationships/articles")
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

    let (status, code) = Authors
        .unlink(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn unlink_without_a_body_is_rejected() -> Result {
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
        .uri("/authors/1/relationships/articles")
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

    let (status, code) = Authors
        .unlink(context, "articles")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("MissingLinkageBody".to_string()));

    Ok(())
}
