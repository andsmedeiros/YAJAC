use super::*;
use test_log::test;

#[test]
fn show_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/articles/1?{query}"))
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

            Ok(Articles
                .show(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn related_of_a_to_one_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/comments/1/article?{query}"))
                .body(stream)?;
            let uri: Uri = request.uri().clone().into();
            let context = routing::build_resource_context(
                &manager,
                &router,
                &base,
                &uri,
                request,
                manager.registry().schema("comments")?,
            )?;

            Ok(Comments
                .related(context, "article")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn linkage_of_a_to_one_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/articles/1/relationships/author?{query}"))
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

            Ok(Articles
                .linkage(context, "author")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn linkage_of_a_to_many_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::GET)
                .uri(format!("/articles/1/relationships/comments?{query}"))
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

            Ok(Articles
                .linkage(context, "comments")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn create_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("publishers", fixtures::publishers::acme()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let body =
                json!({ "data": { "type": "articles", "attributes": { "title": "Sorted" } } });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::POST)
                .uri(format!("/articles?{query}"))
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

            Ok(Articles
                .create(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn update_rejects_every_parameter_it_does_not_use() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = COLLECTION_PARAMETERS
        .iter()
        .map(|query| {
            let body = json!({
                "data": { "type": "articles", "id": "1", "attributes": { "title": "Paged" } }
            });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::PATCH)
                .uri(format!("/articles/1?{query}"))
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

            Ok(Articles
                .update(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn delete_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let stream: ByteStream = Box::new(empty());
            let request = http::Request::builder()
                .method(Method::DELETE)
                .uri(format!("/articles/1?{query}"))
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

            Ok(Articles
                .delete(context)
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn link_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("comments", fixtures::comments::praise()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::POST)
                .uri(format!("/articles/2/relationships/comments?{query}"))
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

            Ok(Articles
                .link(context, "comments")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn relink_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let body = json!({ "data": { "type": "authors", "id": "2" } });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::PATCH)
                .uri(format!("/articles/1/relationships/author?{query}"))
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

            Ok(Articles
                .relink(context, "author")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}

#[test]
fn unlink_rejects_every_parameter() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;

    let failures: Vec<_> = EVERY_PARAMETER
        .iter()
        .map(|query| {
            let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
            let stream: ByteStream = Box::new(Cursor::new(serde_json::to_vec(&body)?));
            let request = http::Request::builder()
                .method(Method::DELETE)
                .uri(format!("/articles/1/relationships/comments?{query}"))
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

            Ok(Articles
                .unlink(context, "comments")
                .err()
                .map(|error| (error.status, error.code.to_string())))
        })
        .collect::<Result<_>>()?;

    let unsupported = Some((
        StatusCode::BAD_REQUEST,
        "UnsupportedQueryParameter".to_string(),
    ));

    assert!(failures.iter().all(|failure| failure == &unsupported));

    Ok(())
}
