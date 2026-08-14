use super::*;
use test_log::test;

#[test]
fn linkage_of_a_to_many_yields_every_member_identifier() -> Result {
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

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
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

    let response = Authors.linkage(context, "articles")?;
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
fn linkage_of_a_belongs_to_yields_its_identifier() -> Result {
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

    let response = Articles.linkage(context, "author")?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }
    );

    Ok(())
}

#[test]
fn linkage_of_an_unset_belongs_to_is_empty() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
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

    let response = Articles.linkage(context, "author")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

/// `authors.profile` joins on `handle`, a unique text column that is not the primary key, so this
/// resolves the relationship without the identifier ever standing in for the join key.
#[test]
fn linkage_of_a_has_one_joined_by_a_non_primary_key_yields_its_identifier() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

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

    let response = Authors.linkage(context, "profile")?;
    let record = require_record(&response)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "profiles".to_string(),
            id: "1".to_string()
        }
    );

    Ok(())
}

#[test]
fn linkage_of_an_unset_has_one_is_empty() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

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

    let response = Authors.linkage(context, "profile")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

#[test]
fn linkage_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([("authors", fixtures::authors::ann()?)])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
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
        .linkage(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}

#[test]
fn related_of_a_to_many_serves_the_related_records() -> Result {
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

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/articles")
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

    let response = Authors.related(context, "articles")?;
    let records = require_collection(&response)?;
    let identifiers: Vec<&Identifier> = records
        .iter()
        .map(|resource| &resource.identifier)
        .collect();
    let titles: Vec<Option<&serde_json::Value>> = records
        .iter()
        .map(|resource| {
            resource
                .attributes
                .as_ref()
                .and_then(|attributes| attributes.get("title"))
        })
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
    assert_eq!(
        titles,
        vec![
            Some(&json!("On Borrowed Lifetimes")),
            Some(&json!("The Cost of a Clone"))
        ]
    );

    Ok(())
}

#[test]
fn related_of_a_belongs_to_serves_the_related_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
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

    let response = Articles.related(context, "author")?;
    let record = require_record(&response)?;
    let name = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("name"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(name, Some(&json!("Ann Sorensen")));

    Ok(())
}

#[test]
fn related_of_an_unset_belongs_to_is_empty() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::unattributed()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/3/author")
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

    let response = Articles.related(context, "author")?;
    let content = response.body().as_ref().map(|document| &document.content);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content, Some(&PrimaryContent::Empty { data: () }));

    Ok(())
}

/// The related record is reached through `handle`, so serving it exercises the non-primary-key join
/// on the read path rather than only in the linkage.
#[test]
fn related_of_a_has_one_joined_by_a_non_primary_key_serves_the_related_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("profiles", fixtures::profiles::anns()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/profile")
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

    let response = Authors.related(context, "profile")?;
    let record = require_record(&response)?;
    let bio = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("bio"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "profiles".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(
        bio,
        Some(&json!("Writes about systems, mostly the parts that leak."))
    );

    Ok(())
}

#[test]
fn related_includes_the_resources_the_query_solicits() -> Result {
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

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/articles?include=author")
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

    let response = Authors.related(context, "articles")?;
    let included: Vec<&Identifier> = response
        .body()
        .as_ref()
        .and_then(|document| document.included.as_ref())
        .map(|resources| {
            resources
                .iter()
                .map(|resource| &resource.identifier)
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        included,
        vec![&Identifier::Existing {
            kind: "authors".to_string(),
            id: "1".to_string()
        }]
    );

    Ok(())
}

#[test]
fn related_of_a_relationship_the_schema_does_not_declare_is_an_internal_error() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/authors/1/articles")
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
        .related(context, "ghost")
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("InvalidRelationshipAccess".to_string()));

    Ok(())
}
