use super::{Router, RouterError};
use crate::database::adapters::SqliteAdapter;
use crate::http_wrappers::{StatusCode, Uri};
use crate::json_api::document::Document;
use crate::json_api::identifier::Identifier;
use crate::json_api::primary_content::PrimaryContent;
use crate::json_api::resource::{Links, Resource};
use crate::routing::builders::{ResourceVerbs, RouteBuilder, UnboundVerbs};
use crate::routing::middleware::PrimaryMiddleware;
use crate::routing::{
    BaseUri, MountSlot, PrimaryContext, PrimaryResult, ResourceContext, RouteParameters,
    respond_with,
};
use crate::serialisation::ByteStream;
use crate::test_support::routing::Articles;
use crate::test_support::{Result, database};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Method};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Cursor, Read, empty};
use test_log::test;

/// A raw-tier guard admitting only requests that carry a given header.
struct RequireHeader(&'static str);
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for RequireHeader {
    fn matches(&self, headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        headers.contains_key(self.0)
    }
}

// --- assembly ---------------------------------------------------------------

#[test]
fn a_registered_resource_needs_no_mount() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::<SqliteAdapter>::try_new(BaseUri::Relative, |root| {
        root.resource::<Articles>("articles", articles)
    });

    assert_eq!(assembled.err(), None);

    Ok(())
}

#[test]
fn mounting_one_resource_twice_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::<SqliteAdapter>::try_new(BaseUri::Relative, |root| {
        root.resource::<Articles>("articles", articles)
            .resource::<Articles>("posts", articles)
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::DuplicateResource {
            kind: "articles".to_string()
        })
    );

    Ok(())
}

#[test]
fn mounting_one_relationship_endpoint_twice_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::<SqliteAdapter>::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.relationship("comments").linkage("comments")
        })
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::DuplicateRelationshipSlot {
            kind: "articles".to_string(),
            relationship: "comments".to_string(),
            slot: MountSlot::Linkage,
        })
    );

    Ok(())
}

#[test]
fn enumerating_every_relationship_over_a_mounted_one_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::<SqliteAdapter>::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.relationship("comments").all_relationships()
        })
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::DuplicateRelationshipSlot {
            kind: "articles".to_string(),
            relationship: "comments".to_string(),
            slot: MountSlot::Linkage,
        })
    );

    Ok(())
}

#[test]
fn mounting_a_relationship_the_schema_does_not_declare_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::<SqliteAdapter>::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.relationship("ghost")
        })
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::UnknownRelationship {
            kind: "articles".to_string(),
            relationship: "ghost".to_string(),
        })
    );

    Ok(())
}

#[test]
fn a_named_wildcard_is_refused() -> Result {
    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*rest",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::NamedGlob {
            path: "files/*rest".to_string()
        })
    );

    Ok(())
}

#[test]
fn a_wildcard_before_the_end_of_a_template_is_refused() -> Result {
    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "*/tail",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::MisplacedGlob {
            path: "*/tail".to_string()
        })
    );

    Ok(())
}

#[test]
fn capturing_one_parameter_twice_in_a_template_is_refused() -> Result {
    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            ":id/replies/:id",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::DuplicateParameter {
            path: ":id/replies/:id".to_string(),
            parameter: "id".to_string(),
        })
    );

    Ok(())
}

// --- dispatch ---------------------------------------------------------------

#[test]
fn a_request_reaches_the_handler_mounted_at_its_path() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "health",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
        .get(
            "status",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/status")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    Ok(())
}

#[test]
fn a_path_no_template_matches_is_answered_bare() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "health",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/widgets")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.body().is_none());
    assert_eq!(response.headers().get(CONTENT_TYPE), None);

    Ok(())
}

#[test]
fn a_method_no_template_answers_is_not_found() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "health",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/health")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn the_first_template_that_matches_wins() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "health",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
        .get(
            "health",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn a_template_matches_only_its_own_segment_count() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "items/:id",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let shorter_stream: ByteStream = Box::new(empty());
    let shorter = http::Request::builder()
        .method(Method::GET)
        .uri("/items")
        .body(shorter_stream)?;

    let longer_stream: ByteStream = Box::new(empty());
    let longer = http::Request::builder()
        .method(Method::GET)
        .uri("/items/1/replies")
        .body(longer_stream)?;

    assert_eq!(
        router.handle(&connection_manager, shorter)?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router.handle(&connection_manager, longer)?.status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn empty_segments_in_a_template_are_dropped() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "/health/",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

// --- captured segments ------------------------------------------------------

#[test]
fn a_dynamic_segment_is_captured_under_its_name() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "items/:id",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let captured = context
                    .route_parameters()
                    .get("id")
                    .map(|id| id.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/items/42")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(String::from_utf8(captured)?, "42");

    Ok(())
}

#[test]
fn a_dynamic_segment_is_captured_percent_decoded() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "items/:id",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let captured = context
                    .route_parameters()
                    .get("id")
                    .map(|id| id.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/items/a%20b")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(String::from_utf8(captured)?, "a b");

    Ok(())
}

#[test]
fn a_dynamic_segment_that_cannot_be_decoded_matches_nothing() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "items/:id",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/items/%FF")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn a_computed_static_segment_matches_literally() -> Result {
    let connection_manager = database::build_database([])?;
    let version = String::from("v2");
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.scope(version, |scoped| {
            scoped.get(
                "health",
                |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                    respond_with(StatusCode::OK, None).map_err(Into::into)
                },
            )
        })
    })?;

    let matched_stream: ByteStream = Box::new(empty());
    let matched = http::Request::builder()
        .method(Method::GET)
        .uri("/v2/health")
        .body(matched_stream)?;

    let missed_stream: ByteStream = Box::new(empty());
    let missed = http::Request::builder()
        .method(Method::GET)
        .uri("/v3/health")
        .body(missed_stream)?;

    assert_eq!(
        router.handle(&connection_manager, matched)?.status(),
        StatusCode::OK
    );
    assert_eq!(
        router.handle(&connection_manager, missed)?.status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn a_computed_dynamic_segment_is_captured_under_its_name() -> Result {
    let connection_manager = database::build_database([])?;
    let tenant = String::from(":tenant");
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.scope(tenant, |scoped| {
            scoped.get(
                "health",
                |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                    let captured = context
                        .route_parameters()
                        .get("tenant")
                        .map(|tenant| tenant.to_string())
                        .unwrap_or_default();
                    let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                    respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
                },
            )
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/acme/health")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(String::from_utf8(captured)?, "acme");

    Ok(())
}

// --- wildcards --------------------------------------------------------------

#[test]
fn a_wildcard_captures_every_segment_after_it() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let captured = context
                    .route_parameters()
                    .get_glob()
                    .map(|tail| tail.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/notes/2018/march")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(String::from_utf8(captured)?, "notes/2018/march");

    Ok(())
}

#[test]
fn a_wildcard_captures_a_single_segment() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let captured = context
                    .route_parameters()
                    .get_glob()
                    .map(|tail| tail.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/notes")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(String::from_utf8(captured)?, "notes");

    Ok(())
}

#[test]
fn a_wildcard_needs_at_least_one_segment() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn a_wildcard_captures_its_tail_undecoded() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let captured = context
                    .route_parameters()
                    .get_glob()
                    .map(|tail| tail.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/march%20notes")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(String::from_utf8(captured)?, "march%20notes");

    Ok(())
}

#[test]
fn a_wildcard_does_not_split_an_encoded_slash() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "files/*",
            |context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let captured = context
                    .route_parameters()
                    .get_glob()
                    .map(|tail| tail.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/notes/2018%2F03/march")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let mut captured = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut captured)?;
    }

    assert_eq!(String::from_utf8(captured)?, "notes/2018%2F03/march");

    Ok(())
}

// --- guarded routes ---------------------------------------------------------

#[test]
fn a_route_whose_guard_refuses_falls_through_to_the_next() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireHeader("x-key"), |guarded| {
            guarded.get(
                "health",
                |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                    respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
                },
            )
        })
        .get(
            "health",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            },
        )
    })?;

    let unkeyed_stream: ByteStream = Box::new(empty());
    let unkeyed = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(unkeyed_stream)?;

    let keyed_stream: ByteStream = Box::new(empty());
    let keyed = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header("x-key", "present")
        .body(keyed_stream)?;

    assert_eq!(
        router.handle(&connection_manager, unkeyed)?.status(),
        StatusCode::OK
    );
    assert_eq!(
        router.handle(&connection_manager, keyed)?.status(),
        StatusCode::ACCEPTED
    );

    Ok(())
}

#[test]
fn a_route_whose_guard_refuses_with_no_successor_is_not_found() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireHeader("x-key"), |guarded| {
            guarded.get(
                "health",
                |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                    respond_with(StatusCode::OK, None).map_err(Into::into)
                },
            )
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

// --- the crossing -----------------------------------------------------------

#[test]
fn a_document_crosses_into_the_response_as_bytes() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let self_link: Uri = "/articles/1".parse()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.get(
                "marked",
                move |_context: ResourceContext<'_, '_, SqliteAdapter>| {
                    respond_with(
                        StatusCode::OK,
                        Some(Document {
                            content: PrimaryContent::Record {
                                data: Box::new(Resource {
                                    identifier: Identifier::Existing {
                                        kind: "articles".to_string(),
                                        id: "1".to_string(),
                                    },
                                    attributes: Some(HashMap::from([(
                                        "title".to_string(),
                                        json!("On Borrowed Lifetimes"),
                                    )])),
                                    relationships: None,
                                    links: Some(Links {
                                        this: self_link.clone(),
                                    }),
                                    meta: None,
                                }),
                            },
                            meta: None,
                            jsonapi: None,
                            links: None,
                            included: None,
                        }),
                    )
                },
            )
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/marked")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let mut served = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut served)?;
    }

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&served)?,
        json!({
            "data": {
                "type": "articles",
                "id": "1",
                "attributes": { "title": "On Borrowed Lifetimes" },
                "links": { "self": "/articles/1" }
            }
        })
    );

    Ok(())
}

#[test]
fn a_documentless_response_crosses_carrying_no_bytes() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.get(
                "marked",
                |_context: ResourceContext<'_, '_, SqliteAdapter>| {
                    respond_with(StatusCode::NO_CONTENT, None)
                },
            )
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/marked")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.body().is_none());

    Ok(())
}

#[test]
fn a_raw_response_reaches_the_client_untouched() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "download",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| {
                let stream: ByteStream =
                    Box::new(Cursor::new(b"the bytes a handler streamed".to_vec()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/download")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let mut served = Vec::new();
    if let Some(mut stream) = body {
        stream.read_to_end(&mut served)?;
    }

    assert_eq!(served, b"the bytes a handler streamed");
    assert_eq!(parts.headers.get(CONTENT_TYPE), None);

    Ok(())
}

#[test]
fn a_raw_handler_s_failure_escapes_to_the_embedder() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get(
            "boom",
            |_context: PrimaryContext<'_, '_, SqliteAdapter>| -> PrimaryResult {
                Err("the handler could not answer".into())
            },
        )
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/boom")
        .body(stream)?;

    assert!(router.handle(&connection_manager, request).is_err());

    Ok(())
}
