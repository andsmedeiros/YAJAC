use super::*;
use test_log::test;

#[test]
fn index_yields_every_record_of_the_resource() -> Result {
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

    let response = Articles.index(context)?;
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
fn show_yields_the_addressed_record() -> Result {
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

    let response = Articles.show(context)?;
    let record = require_record(&response)?;
    let title = record
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.get("title"));

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        record.identifier,
        Identifier::Existing {
            kind: "articles".to_string(),
            id: "1".to_string()
        }
    );
    assert_eq!(title, Some(&json!("On Borrowed Lifetimes")));

    Ok(())
}

#[test]
fn show_of_an_absent_record_is_not_found() -> Result {
    let manager = database::build_database([])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
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
        .show(context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::NOT_FOUND));
    assert_eq!(code, Some("RecordNotFound".to_string()));

    Ok(())
}

#[test]
fn index_serves_only_the_fields_the_query_solicits() -> Result {
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
        .uri("/articles?fields[articles]=title")
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

    let response = Articles.index(context)?;
    let attributes: Vec<_> = require_collection(&response)?
        .iter()
        .map(|resource| serde_json::to_value(&resource.attributes))
        .collect::<std::result::Result<_, _>>()?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        attributes,
        vec![
            json!({ "title": "On Borrowed Lifetimes" }),
            json!({ "title": "The Cost of a Clone" })
        ]
    );

    Ok(())
}

#[test]
fn index_includes_the_resources_the_query_solicits() -> Result {
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
        .uri("/articles?include=author")
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

    let response = Articles.index(context)?;
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
fn index_orders_the_collection_the_query_sorts_by() -> Result {
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

    let response = Articles.index(context)?;
    let titles: Vec<_> = require_collection(&response)?
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
        titles,
        vec![
            Some(&json!("The Cost of a Clone")),
            Some(&json!("On Borrowed Lifetimes")),
            Some(&json!("Notes Found in a Drawer"))
        ]
    );

    Ok(())
}

#[test]
fn index_scopes_the_collection_the_query_filters() -> Result {
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
        .uri("/articles?filter[published]=eq:true")
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

    let response = Articles.index(context)?;
    let identifiers: Vec<&Identifier> = require_collection(&response)?
        .iter()
        .map(|resource| &resource.identifier)
        .collect();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        identifiers,
        vec![&Identifier::Existing {
            kind: "articles".to_string(),
            id: "1".to_string()
        }]
    );

    Ok(())
}

#[test]
fn index_truncates_the_collection_the_query_pages() -> Result {
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
        .uri("/articles?page[size]=1&page[number]=2")
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

    let response = Articles.index(context)?;
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
fn index_scopes_the_collection_the_query_searches() -> Result {
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
        .uri("/articles?search=bottleneck")
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

    let response = Articles.index(context)?;
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
fn show_serves_only_the_fields_the_query_solicits() -> Result {
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
        .uri("/articles/1?fields[articles]=title,views")
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

    let response = Articles.show(context)?;
    let attributes = serde_json::to_value(&require_record(&response)?.attributes)?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        attributes,
        json!({ "title": "On Borrowed Lifetimes", "views": 1204 })
    );

    Ok(())
}

#[test]
fn show_includes_the_resources_the_query_solicits() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1?include=comments")
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

    let response = Articles.show(context)?;
    let included: HashSet<Identifier> = response
        .body()
        .as_ref()
        .and_then(|document| document.included.as_ref())
        .map(|resources| {
            resources
                .iter()
                .map(|resource| resource.identifier.clone())
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        included,
        HashSet::from([
            Identifier::Existing {
                kind: "comments".to_string(),
                id: "1".to_string()
            },
            Identifier::Existing {
                kind: "comments".to_string(),
                id: "2".to_string()
            }
        ])
    );

    Ok(())
}

#[test]
fn show_of_a_text_keyed_record_yields_the_addressed_record() -> Result {
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

    let response = Publishers.show(context)?;
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
    assert_eq!(name, Some(&json!("Acme Press")));

    Ok(())
}
