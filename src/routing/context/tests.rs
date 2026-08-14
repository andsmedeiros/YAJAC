use crate::database::adapters::SqliteAdapter;
use crate::database::attributes::{Attribute, Identifier};
use crate::database::query_parameters::{SortDirection, SortingAttribute};
use crate::database::relationships::Relationship;
use crate::database::table::Table;
use crate::http_wrappers::{StatusCode, Uri};
use crate::json_api::identifier::Identifier as JsonApiIdentifier;
use crate::json_api::relationship::Linkage;
use crate::json_api::resource::Resource;
use crate::routing::{BaseUri, Error, Router};
use crate::serialisation::ByteStream;
use crate::serialisation::uri_generator::UriGenerator;
use crate::test_support::database::ConnectionManager;
use crate::test_support::routing::{Articles, Publishers};
use crate::test_support::{Result, database, fixtures, routing};
use http::Method;
use serde_json::json;
use std::borrow::Cow;
use std::cell::LazyCell;
use std::collections::HashMap;
use std::io::{Cursor, empty};
use test_log::test;

/// A router mounting an integer-keyed resource and a text-keyed one, so a request can be resolved
/// against either kind of primary key.
fn build_router<'sch>(
    connection_manager: &'sch ConnectionManager<'sch>,
    base_uri: BaseUri<'sch>,
) -> Result<Router<'sch, SqliteAdapter>> {
    let articles = connection_manager.registry().schema("articles")?;
    let publishers = connection_manager.registry().schema("publishers")?;

    Router::try_new(base_uri, |root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Publishers>("publishers", publishers)
    })
    .map_err(Into::into)
}

#[test]
fn a_context_lends_the_schema_and_the_uri_it_was_built_from() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1?fields[articles]=title")
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

    assert_eq!(context.schema().name(), "articles");
    assert_eq!(context.uri(), &uri);

    Ok(())
}

#[test]
fn a_context_lends_a_generator_rooted_at_the_base_it_was_built_with() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Absolute(Cow::Borrowed("https://api.example.com"));
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "id": "1" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let record = context.require_record()?;
    let generated = context.uri_generator().uri_for_resource(&record)?;

    assert_eq!(
        generated,
        Some("https://api.example.com/articles/1".parse()?)
    );

    Ok(())
}

#[test]
fn query_parameters_are_parsed_against_the_schema_and_cached() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?sort=-title")
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

    assert!(LazyCell::get(&context.query_parameters).is_none());

    assert_eq!(
        context.query_parameters()?.sort,
        Some(vec![SortingAttribute {
            attribute: "title",
            direction: SortDirection::Descending,
        }])
    );

    assert!(LazyCell::get(&context.query_parameters).is_some_and(|parsed| parsed.is_ok()));

    Ok(())
}

#[test]
fn a_query_the_schema_refuses_fails_the_same_way_on_every_access() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?sort=ghost")
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

    let first = context.query_parameters().err();
    let (status, code) = first
        .as_ref()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::BAD_REQUEST));
    assert_eq!(code, Some("QueryValidationFailure"));
    assert_eq!(first, context.query_parameters().err());

    Ok(())
}

#[test]
fn require_id_resolves_an_integer_primary_key() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/12")
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

    assert_eq!(context.require_id()?, Identifier::Integer(12));

    Ok(())
}

#[test]
fn require_id_resolves_a_text_primary_key() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

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

    assert_eq!(
        context.require_id()?,
        Identifier::Text("acme-press".to_string())
    );

    Ok(())
}

#[test]
fn require_id_of_an_unparseable_integer_key_fails_to_parse() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/acme-press")
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

    let (status, code) = context
        .require_id()
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::BAD_REQUEST));
    assert_eq!(code, Some("FailedToParseRouteParameter"));

    Ok(())
}

#[test]
fn require_id_at_an_endpoint_that_targets_no_record_is_missing() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
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

    assert_eq!(
        context.require_id().err(),
        Some(Error::RequiredRouteParameterMissing {
            parameter: "id".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_resource_yields_the_submitted_resource() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "attributes": { "title": "Provenance" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource()?,
        Resource {
            identifier: JsonApiIdentifier::New {
                kind: "articles".to_string(),
                lid: None,
            },
            attributes: Some(HashMap::from([("title".to_string(), json!("Provenance"))])),
            relationships: None,
            links: None,
            meta: None,
        }
    );

    Ok(())
}

#[test]
fn require_resource_without_a_body_is_missing() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource().err(),
        Some(Error::MissingResourceBody)
    );

    Ok(())
}

#[test]
fn require_resource_of_a_collection_is_not_a_resource() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource().err(),
        Some(Error::PrimaryDataIsNotAResource)
    );

    Ok(())
}

#[test]
fn require_resource_of_an_errors_document_carries_no_primary_data() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "errors": [{ "status": "422", "title": "Unprocessable Content" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource().err(),
        Some(Error::ErrorDocumentSubmitted)
    );

    Ok(())
}

#[test]
fn require_resource_of_another_type_is_a_mismatch() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "publishers", "id": "acme-press" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource().err(),
        Some(Error::ResourceTypeMismatch {
            expected: "articles".to_string(),
            actual: "publishers".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_resource_of_an_id_the_endpoint_does_not_target_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "id": "7" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource().err(),
        Some(Error::ResourceIdMismatch {
            expected: "1".to_string(),
            actual: "7".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_resource_at_a_targeted_endpoint_requires_the_id() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "attributes": { "title": "Untitled" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_resource().err(),
        Some(Error::ResourceIdMissing {
            expected: "1".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_record_binds_the_submitted_resource_to_the_schema() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "articles",
            "id": "1",
            "attributes": { "title": "Provenance", "views": 12 },
            "relationships": { "publisher": { "data": { "type": "publishers", "id": "acme-press" } } }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let record = context.require_record()?;
    let attributes: HashMap<&str, Attribute> = record.attributes.clone().into_iter().collect();

    assert_eq!(record.id, Some(Identifier::Integer(1)));
    assert_eq!(
        attributes,
        HashMap::from([
            ("title", Attribute::Text("Provenance".to_string())),
            ("views", Attribute::Integer(12)),
        ])
    );
    assert_eq!(
        record.relationships,
        HashMap::from([(
            "publisher",
            Relationship::BelongsTo(Identifier::Text("acme-press".to_string()))
        )])
    );

    Ok(())
}

#[test]
fn require_record_of_an_attribute_the_schema_does_not_declare_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "attributes": { "subtitle": "Unknown" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_record().err(),
        Some(Error::UnknownAttribute {
            kind: "articles".to_string(),
            attribute: "subtitle".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_record_of_a_relationship_the_schema_does_not_declare_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "articles",
            "relationships": { "sponsor": { "data": { "type": "authors", "id": "1" } } }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = context
        .require_record()
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure"));

    Ok(())
}

#[test]
fn require_relationship_of_absent_linkage_clears_it() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("publisher")
        .ok_or("articles declares no 'publisher' relationship")?;

    assert_eq!(
        context.require_relationship(None, descriptor)?,
        Relationship::Empty
    );

    Ok(())
}

#[test]
fn require_relationship_of_null_linkage_clears_it() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("publisher")
        .ok_or("articles declares no 'publisher' relationship")?;

    assert_eq!(
        context.require_relationship(Some(Linkage::Empty), descriptor)?,
        Relationship::Empty
    );

    Ok(())
}

#[test]
fn require_relationship_materialises_a_belongs_to_against_a_text_key() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("publisher")
        .ok_or("articles declares no 'publisher' relationship")?;
    let linkage = Linkage::ToOne(JsonApiIdentifier::Existing {
        kind: "publishers".to_string(),
        id: "acme-press".to_string(),
    });

    assert_eq!(
        context.require_relationship(Some(linkage), descriptor)?,
        Relationship::BelongsTo(Identifier::Text("acme-press".to_string()))
    );

    Ok(())
}

#[test]
fn require_relationship_materialises_every_identifier_of_a_has_many() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("comments")
        .ok_or("articles declares no 'comments' relationship")?;
    let linkage = Linkage::ToMany(vec![
        JsonApiIdentifier::Existing {
            kind: "comments".to_string(),
            id: "1".to_string(),
        },
        JsonApiIdentifier::Existing {
            kind: "comments".to_string(),
            id: "2".to_string(),
        },
    ]);

    assert_eq!(
        context.require_relationship(Some(linkage), descriptor)?,
        Relationship::HasMany(vec![Identifier::Integer(1), Identifier::Integer(2)])
    );

    Ok(())
}

#[test]
fn require_relationship_of_linkage_contradicting_the_direction_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("publisher")
        .ok_or("articles declares no 'publisher' relationship")?;
    let linkage = Linkage::ToMany(vec![JsonApiIdentifier::Existing {
        kind: "publishers".to_string(),
        id: "acme-press".to_string(),
    }]);

    let (status, code) = context
        .require_relationship(Some(linkage), descriptor)
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure"));

    Ok(())
}

#[test]
fn require_relationship_of_an_identifier_naming_another_type_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("publisher")
        .ok_or("articles declares no 'publisher' relationship")?;
    let linkage = Linkage::ToOne(JsonApiIdentifier::Existing {
        kind: "authors".to_string(),
        id: "1".to_string(),
    });

    assert_eq!(
        context
            .require_relationship(Some(linkage), descriptor)
            .err(),
        Some(Error::IdentifierTypeMismatch {
            expected: "publishers".to_string(),
            actual: "authors".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_relationship_of_an_identifier_awaiting_creation_resolves_to_nothing() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("publisher")
        .ok_or("articles declares no 'publisher' relationship")?;
    let linkage = Linkage::ToOne(JsonApiIdentifier::New {
        kind: "publishers".to_string(),
        lid: Some("draft-press".to_string()),
    });

    assert_eq!(
        context
            .require_relationship(Some(linkage), descriptor)
            .err(),
        Some(Error::UnresolvableIdentifier)
    );

    Ok(())
}

#[test]
fn require_relationship_of_a_non_integer_identifier_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("comments")
        .ok_or("articles declares no 'comments' relationship")?;
    let linkage = Linkage::ToMany(vec![JsonApiIdentifier::Existing {
        kind: "comments".to_string(),
        id: "first".to_string(),
    }]);

    assert_eq!(
        context
            .require_relationship(Some(linkage), descriptor)
            .err(),
        Some(Error::InvalidIntegerIdentifier {
            id: "first".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_linkage_reads_null_data_as_empty() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": null });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/publisher")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(context.require_linkage()?, Linkage::Empty);

    Ok(())
}

#[test]
fn require_linkage_reads_a_single_identifier() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "publishers", "id": "acme-press" } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/publisher")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_linkage()?,
        Linkage::ToOne(JsonApiIdentifier::Existing {
            kind: "publishers".to_string(),
            id: "acme-press".to_string(),
        })
    );

    Ok(())
}

#[test]
fn require_linkage_reads_an_identifier_collection() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body =
        json!({ "data": [{ "type": "comments", "id": "1" }, { "type": "comments", "id": "2" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles/1/relationships/comments")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_linkage()?,
        Linkage::ToMany(vec![
            JsonApiIdentifier::Existing {
                kind: "comments".to_string(),
                id: "1".to_string(),
            },
            JsonApiIdentifier::Existing {
                kind: "comments".to_string(),
                id: "2".to_string(),
            },
        ])
    );

    Ok(())
}

#[test]
fn require_linkage_of_a_full_resource_object_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({
        "data": {
            "type": "publishers",
            "id": "acme-press",
            "attributes": { "name": "Acme Press" }
        }
    });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/publisher")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(context.require_linkage().err(), Some(Error::InvalidLinkage));

    Ok(())
}

#[test]
fn require_linkage_without_a_body_is_missing() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/publisher")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_linkage().err(),
        Some(Error::MissingLinkageBody)
    );

    Ok(())
}

#[test]
fn contains_body_answers_the_same_on_every_probe() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": { "type": "articles", "attributes": { "title": "Probed" } } });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert!(context.contains_body()?);
    assert!(context.contains_body()?);

    let resource = context.require_resource()?;

    assert_eq!(
        resource.attributes,
        Some(HashMap::from([("title".to_string(), json!("Probed"))]))
    );

    Ok(())
}

#[test]
fn contains_body_reports_an_empty_body() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert!(!context.contains_body()?);

    Ok(())
}

#[test]
fn a_body_taken_twice_is_an_internal_fault() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    context.require_body()?;

    assert_eq!(
        context.require_body().err(),
        Some(Error::RequestBodyConsumed)
    );

    Ok(())
}

#[test]
fn the_store_reaches_the_records_the_request_serves() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, context.query_parameters()?)?
        .content;

    assert_eq!(record.get_id(), Some(&Identifier::Integer(1)));

    Ok(())
}

#[test]
fn require_relationship_materialises_a_has_one() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("summary")
        .ok_or("articles declares no 'summary' relationship")?;
    let linkage = Linkage::ToOne(JsonApiIdentifier::Existing {
        kind: "summaries".to_string(),
        id: "1".to_string(),
    });

    assert_eq!(
        context.require_relationship(Some(linkage), descriptor)?,
        Relationship::HasOne(Identifier::Integer(1))
    );

    Ok(())
}

#[test]
fn require_relationship_of_a_lone_identifier_for_a_collection_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let descriptor = schema
        .relationship("comments")
        .ok_or("articles declares no 'comments' relationship")?;
    let linkage = Linkage::ToOne(JsonApiIdentifier::Existing {
        kind: "comments".to_string(),
        id: "1".to_string(),
    });

    let (status, code) = context
        .require_relationship(Some(linkage), descriptor)
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("ResourceValidationFailure"));

    Ok(())
}

#[test]
fn require_linkage_of_an_errors_document_carries_no_linkage() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "errors": [{ "status": "422", "title": "Unprocessable Content" }] });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/publisher")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    assert_eq!(
        context.require_linkage().err(),
        Some(Error::ErrorDocumentSubmitted)
    );

    Ok(())
}

#[test]
fn a_body_that_is_not_json_is_a_bad_request() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(Cursor::new(b"{ \"data\":".to_vec()));
    let request = http::Request::builder()
        .method(Method::PATCH)
        .uri("/articles/1/relationships/publisher")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = context
        .require_linkage()
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::BAD_REQUEST));
    assert_eq!(code, Some("MalformedRequestBody"));

    Ok(())
}

#[test]
fn a_body_of_json_that_models_no_document_is_unprocessable() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let body = json!({ "data": 7 });
    let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/articles")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let mut context = routing::build_resource_context(
        &manager,
        &router,
        &base,
        &uri,
        request,
        manager.registry().schema("articles")?,
    )?;

    let (status, code) = context
        .require_resource()
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(code, Some("InvalidRequestBodyContent"));

    Ok(())
}

#[test]
fn a_table_is_bound_to_the_schema_it_names() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

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

    assert_eq!(context.table("publishers")?.schema().name(), "publishers");

    Ok(())
}

#[test]
fn a_table_the_registry_does_not_hold_is_refused() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

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

    let (status, code) = context
        .table("ghosts")
        .err()
        .map(|error| (error.status(), error.code()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("UnknownSchema"));

    Ok(())
}

#[test]
fn parse_query_reads_the_request_against_the_schema_it_is_given() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles?sort=name")
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

    let publishers = manager.registry().schema("publishers")?;

    assert_eq!(
        context.parse_query(publishers)?.sort,
        Some(vec![SortingAttribute {
            attribute: "name",
            direction: SortDirection::Ascending,
        }])
    );
    assert!(context.query_parameters().is_err());

    Ok(())
}

#[test]
fn the_headers_are_the_ones_the_request_carried() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .header("x-tenant", "acme")
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

    assert_eq!(
        context
            .headers()
            .get("x-tenant")
            .map(|value| value.as_bytes()),
        Some("acme".as_bytes())
    );

    Ok(())
}

#[test]
fn the_route_parameters_are_the_ones_the_template_captured() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/12")
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

    assert_eq!(
        context.route_parameters().get("id"),
        Some(&Cow::Borrowed("12"))
    );

    Ok(())
}

#[test]
fn the_connection_is_acquired_on_first_use_and_kept() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

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

    assert!(LazyCell::get(&context.context.connection).is_none());

    context.connection()?;

    assert!(LazyCell::get(&context.context.connection).is_some_and(|acquired| acquired.is_ok()));

    Ok(())
}
