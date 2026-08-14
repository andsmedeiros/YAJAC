use super::{EndpointHandler, Route, RouterError, serve_resource};
use crate::database::adapters::SqliteAdapter;
use crate::http_wrappers::{StatusCode, Uri};
use crate::json_api::document::Document;
use crate::json_api::identifier::Identifier;
use crate::json_api::primary_content::PrimaryContent;
use crate::json_api::resource::{Links, Resource};
use crate::routing::builders::{ResourceVerbs, RouteBuilder, UnboundVerbs};
use crate::routing::middleware::{Middleware, PrimaryMiddleware, ResourceMiddleware};
use crate::routing::{
    BaseUri, MountSlot, PrimaryResult, ResourceContext, RouteParameters, respond_with,
};
use crate::serialisation::ByteStream;
use crate::test_support::routing::{Articles, Comments, Router};
use crate::test_support::{Result, database, routing};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, Cursor, empty};
use std::sync::Arc;
use std::thread;
use test_log::test;

/// A guard admitting only requests that carry a given header, mountable on either tier.
struct RequireHeader(&'static str);
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for RequireHeader {
    fn matches(&self, headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        headers.contains_key(self.0)
    }
}
impl<'sch> ResourceMiddleware<'sch, SqliteAdapter> for RequireHeader {
    fn matches(&self, headers: &HeaderMap, _uri: &Uri, _route: &RouteParameters) -> bool {
        headers.contains_key(self.0)
    }
}

/// A guard admitting only requests whose URI carries a query string.
struct RequireQuery;
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for RequireQuery {
    fn matches(&self, _headers: &HeaderMap, uri: &Uri, _route: &RouteParameters) -> bool {
        uri.query().is_some()
    }
}

/// A guard admitting only requests whose matching template captured a given parameter value.
struct RequireParameter(&'static str, &'static str);
impl<'sch> PrimaryMiddleware<'sch, SqliteAdapter> for RequireParameter {
    fn matches(&self, _headers: &HeaderMap, _uri: &Uri, route: &RouteParameters) -> bool {
        route.get(self.0).is_some_and(|value| value == self.1)
    }
}

// --- assembly ---------------------------------------------------------------

#[test]
fn a_registered_resource_needs_no_mount() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.resource::<Articles>("articles", articles)
    });

    assert_eq!(assembled.err(), None);

    Ok(())
}

#[test]
fn mounting_one_resource_twice_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::try_new(BaseUri::Relative, |root| {
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

    let assembled = Router::try_new(BaseUri::Relative, |root| {
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

    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.relationship("comments").all_relationships()
        })
    });
    let either_slot = [MountSlot::Linkage, MountSlot::Related].map(|slot| {
        Some(RouterError::DuplicateRelationshipSlot {
            kind: "articles".to_string(),
            relationship: "comments".to_string(),
            slot,
        })
    });

    assert!(either_slot.contains(&assembled.err()));

    Ok(())
}

#[test]
fn mounting_a_relationship_the_schema_does_not_declare_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;

    let assembled = Router::try_new(BaseUri::Relative, |root| {
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
        root.get("files/*rest", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
        root.get("*/tail", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
        root.get(":id/replies/:id", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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

#[test]
fn one_faulty_route_refuses_the_whole_assembly() -> Result {
    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
        .get("*/tail", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
fn a_fault_inside_a_scope_refuses_the_assembly() -> Result {
    let assembled = Router::try_new(BaseUri::Relative, |root| {
        root.scope("api", |api| {
            api.get("*/tail", |_context| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            })
        })
    });

    assert_eq!(
        assembled.err(),
        Some(RouterError::MisplacedGlob {
            path: "api/*/tail".to_string()
        })
    );

    Ok(())
}

// --- dispatch ---------------------------------------------------------------

#[test]
fn a_request_reaches_the_handler_mounted_at_its_path() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
        .get("status", |_context| {
            respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
        })
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
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
        .get("health", |_context| {
            respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
        })
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
fn a_raw_route_and_a_resource_route_contend_by_mount_order() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("articles/featured", |_context| {
            let stream: ByteStream = Box::new(Cursor::new(b"served raw".to_vec()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
        .resource::<Articles>("articles", articles)
    })?;

    let raw_stream: ByteStream = Box::new(empty());
    let raw = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/featured")
        .body(raw_stream)?;

    let resourceful_stream: ByteStream = Box::new(empty());
    let resourceful = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(resourceful_stream)?;

    let (raw_parts, raw_body) = router.handle(&connection_manager, raw)?.into_parts();
    let served = raw_body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();
    let resourceful = router.handle(&connection_manager, resourceful)?;

    assert_eq!(served, "served raw");
    assert_eq!(raw_parts.headers.get(CONTENT_TYPE), None);
    assert_eq!(
        resourceful.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/vnd.api+json"))
    );

    Ok(())
}

#[test]
fn a_template_matches_only_its_own_segment_count() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("items/:id", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
        root.get("/health/", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
fn a_query_string_takes_no_part_in_matching() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/health?verbose=true")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn a_trailing_slash_on_the_request_takes_no_part_in_matching() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/health/")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn a_template_of_no_segments_answers_the_root() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[test]
fn a_literal_segment_matches_case_sensitively() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/Health")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[test]
fn every_verb_dispatches_to_the_handler_mounted_for_it() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("records", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
        .post("records", |_context| {
            respond_with(StatusCode::CREATED, None).map_err(Into::into)
        })
        .put("records", |_context| {
            respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
        })
        .patch("records", |_context| {
            respond_with(StatusCode::NO_CONTENT, None).map_err(Into::into)
        })
        .delete("records", |_context| {
            respond_with(StatusCode::RESET_CONTENT, None).map_err(Into::into)
        })
    })?;

    let answered: Vec<StatusCode> = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ]
    .into_iter()
    .map(|method| {
        let stream: ByteStream = Box::new(empty());
        let request = http::Request::builder()
            .method(method)
            .uri("/records")
            .body(stream)?;

        Ok(router.handle(&connection_manager, request)?.status().into())
    })
    .collect::<Result<Vec<StatusCode>>>()?;

    assert_eq!(
        answered,
        vec![
            StatusCode::OK,
            StatusCode::CREATED,
            StatusCode::ACCEPTED,
            StatusCode::NO_CONTENT,
            StatusCode::RESET_CONTENT,
        ]
    );

    Ok(())
}

#[test]
fn one_router_serves_several_threads_at_once() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
    })?;

    let answered: Vec<std::result::Result<StatusCode, String>> = thread::scope(|threads| {
        let dispatches: Vec<_> = (0..4)
            .map(|_| {
                threads.spawn(|| {
                    let stream: ByteStream = Box::new(empty());
                    let request = http::Request::builder()
                        .method(Method::GET)
                        .uri("/health")
                        .body(stream)
                        .map_err(|error| error.to_string())?;

                    router
                        .handle(&connection_manager, request)
                        .map(|response| response.status().into())
                        .map_err(|error| error.to_string())
                })
            })
            .collect();

        dispatches
            .into_iter()
            .map(|dispatch| {
                dispatch
                    .join()
                    .unwrap_or_else(|_| Err("a dispatching thread panicked".to_string()))
            })
            .collect()
    });

    assert_eq!(answered, vec![Ok(StatusCode::OK); 4]);

    Ok(())
}

// --- captured segments ------------------------------------------------------

#[test]
fn a_dynamic_segment_is_captured_under_its_name() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("items/:id", |context| {
            let captured = context
                .route_parameters()
                .get("id")
                .map(|id| id.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/items/42")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(captured, "42");

    Ok(())
}

#[test]
fn a_dynamic_segment_is_captured_percent_decoded() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("items/:id", |context| {
            let captured = context
                .route_parameters()
                .get("id")
                .map(|id| id.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/items/a%20b")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "a b");

    Ok(())
}

#[test]
fn a_dynamic_segment_decodes_an_encoded_slash_into_its_value() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("items/:id", |context| {
            let captured = context
                .route_parameters()
                .get("id")
                .map(|id| id.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/items/a%2Fb")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "a/b");

    Ok(())
}

#[test]
fn a_template_captures_every_dynamic_segment_it_declares() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("tenants/:tenant/items/:id", |context| {
            let parameters = context.route_parameters();
            let captured = format!(
                "{}/{}",
                parameters
                    .get("tenant")
                    .map(|tenant| tenant.to_string())
                    .unwrap_or_default(),
                parameters
                    .get("id")
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
            );
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/tenants/acme/items/42")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "acme/42");

    Ok(())
}

#[test]
fn a_dynamic_segment_that_cannot_be_decoded_matches_nothing() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("items/:id", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
            scoped.get("health", |_context| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            })
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
            scoped.get("health", |context| {
                let captured = context
                    .route_parameters()
                    .get("tenant")
                    .map(|tenant| tenant.to_string())
                    .unwrap_or_default();
                let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
                respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
            })
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/acme/health")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "acme");

    Ok(())
}

// --- wildcards --------------------------------------------------------------

#[test]
fn a_wildcard_captures_every_segment_after_it() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("files/*", |context| {
            let captured = context
                .route_parameters()
                .get_glob()
                .map(|tail| tail.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/notes/2018/march")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(captured, "notes/2018/march");

    Ok(())
}

#[test]
fn a_wildcard_captures_a_single_segment() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("files/*", |context| {
            let captured = context
                .route_parameters()
                .get_glob()
                .map(|tail| tail.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/notes")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "notes");

    Ok(())
}

#[test]
fn a_wildcard_needs_at_least_one_segment() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("files/*", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
        root.get("files/*", |context| {
            let captured = context
                .route_parameters()
                .get_glob()
                .map(|tail| tail.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/march%20notes")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "march%20notes");

    Ok(())
}

#[test]
fn a_wildcard_does_not_split_an_encoded_slash() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("files/*", |context| {
            let captured = context
                .route_parameters()
                .get_glob()
                .map(|tail| tail.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/files/notes/2018%2F03/march")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(captured, "notes/2018%2F03/march");

    Ok(())
}

#[test]
fn a_wildcard_at_the_root_claims_every_path() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("*", |context| {
            let captured = context
                .route_parameters()
                .get_glob()
                .map(|tail| tail.to_string())
                .unwrap_or_default();
            let stream: ByteStream = Box::new(Cursor::new(captured.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/anything/at/all")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let captured = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(captured, "anything/at/all");

    Ok(())
}

// --- guarded routes ---------------------------------------------------------

#[test]
fn a_route_whose_guard_refuses_falls_through_to_the_next() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireHeader("x-key"), |guarded| {
            guarded.get("health", |_context| {
                respond_with(StatusCode::ACCEPTED, None).map_err(Into::into)
            })
        })
        .get("health", |_context| {
            respond_with(StatusCode::OK, None).map_err(Into::into)
        })
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
            guarded.get("health", |_context| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            })
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

#[test]
fn every_guard_on_a_route_must_admit_the_request() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireHeader("x-key"), |keyed| {
            keyed.middleware(RequireHeader("x-tenant"), |tenanted| {
                tenanted.get("health", |_context| {
                    respond_with(StatusCode::OK, None).map_err(Into::into)
                })
            })
        })
    })?;

    let partial_stream: ByteStream = Box::new(empty());
    let partial = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header("x-key", "present")
        .body(partial_stream)?;

    let complete_stream: ByteStream = Box::new(empty());
    let complete = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header("x-key", "present")
        .header("x-tenant", "acme")
        .body(complete_stream)?;

    assert_eq!(
        router.handle(&connection_manager, partial)?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router.handle(&connection_manager, complete)?.status(),
        StatusCode::OK
    );

    Ok(())
}

#[test]
fn a_guard_reads_the_uri_the_path_match_ignored() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireQuery, |guarded| {
            guarded.get("health", |_context| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            })
        })
    })?;

    let bare_stream: ByteStream = Box::new(empty());
    let bare = http::Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(bare_stream)?;

    let queried_stream: ByteStream = Box::new(empty());
    let queried = http::Request::builder()
        .method(Method::GET)
        .uri("/health?verbose=true")
        .body(queried_stream)?;

    assert_eq!(
        router.handle(&connection_manager, bare)?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router.handle(&connection_manager, queried)?.status(),
        StatusCode::OK
    );

    Ok(())
}

#[test]
fn a_guard_reads_the_parameters_its_template_captured() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.middleware(RequireParameter("id", "42"), |guarded| {
            guarded.get("items/:id", |_context| {
                respond_with(StatusCode::OK, None).map_err(Into::into)
            })
        })
    })?;

    let admitted_stream: ByteStream = Box::new(empty());
    let admitted = http::Request::builder()
        .method(Method::GET)
        .uri("/items/42")
        .body(admitted_stream)?;

    let refused_stream: ByteStream = Box::new(empty());
    let refused = http::Request::builder()
        .method(Method::GET)
        .uri("/items/7")
        .body(refused_stream)?;

    assert_eq!(
        router.handle(&connection_manager, admitted)?.status(),
        StatusCode::OK
    );
    assert_eq!(
        router.handle(&connection_manager, refused)?.status(),
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[test]
fn a_guard_on_the_resource_tier_gates_matching_too() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.middleware(RequireHeader("x-key"), |guarded| {
                guarded.get("marked", |_context| {
                    respond_with(StatusCode::NO_CONTENT, None)
                })
            })
        })
    })?;

    let unkeyed_stream: ByteStream = Box::new(empty());
    let unkeyed = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/marked")
        .body(unkeyed_stream)?;

    let keyed_stream: ByteStream = Box::new(empty());
    let keyed = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/marked")
        .header("x-key", "present")
        .body(keyed_stream)?;

    assert_eq!(
        router.handle(&connection_manager, unkeyed)?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router.handle(&connection_manager, keyed)?.status(),
        StatusCode::NO_CONTENT
    );

    Ok(())
}

// --- backstops --------------------------------------------------------------
//
// Both guards below cover an invariant the builders uphold but the type system cannot carry, so
// their triggering state is reachable only by assembling a route by hand.

#[test]
fn a_route_carrying_a_named_wildcard_matches_nothing() -> Result {
    let route: Route<SqliteAdapter> = Route::new(
        Method::GET,
        vec![Cow::Borrowed("files"), Cow::Borrowed("*rest")],
        Vec::new(),
        EndpointHandler::primary(|_context| respond_with(StatusCode::OK, None).map_err(Into::into)),
    );

    assert!(
        route
            .match_path(&Method::GET, &["files", "*rest"])
            .is_none()
    );

    Ok(())
}

#[test]
fn a_primary_middleware_reaching_the_resource_tier_is_refused() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let base = BaseUri::Relative;
    let router = Router::try_new(base.clone(), |root| {
        root.resource::<Articles>("articles", articles)
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/1")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(
        &connection_manager,
        &router,
        &base,
        &uri,
        request,
        articles,
    )?;

    let misordered = [Middleware::Primary(Arc::new(RequireHeader("x-key")))];
    let handler =
        |_context: ResourceContext<'_, '_, SqliteAdapter>| respond_with(StatusCode::OK, None);

    let (status, code) = serve_resource(&misordered, &handler, context)
        .err()
        .map(|error| (error.status, error.code.to_string()))
        .unzip();

    assert_eq!(status, Some(StatusCode::INTERNAL_SERVER_ERROR));
    assert_eq!(code, Some("MisorderedMiddleware".to_string()));

    Ok(())
}

// --- the crossing -----------------------------------------------------------

#[test]
fn a_request_body_reaches_its_handler_whole() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.post("echo", |mut context| {
            let received = io::read_to_string(context.require_body()?)?;
            let stream: ByteStream = Box::new(Cursor::new(received.into_bytes()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(Cursor::new(b"the bytes a client sent".to_vec()));
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/echo")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let echoed = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(echoed, "the bytes a client sent");

    Ok(())
}

#[test]
fn a_document_crosses_into_the_response_as_bytes() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let self_link: Uri = "/articles/1".parse()?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.get("marked", move |_context| {
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
            })
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/marked")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let served = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&served)?,
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
            resource.get("marked", |_context| {
                respond_with(StatusCode::NO_CONTENT, None)
            })
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
fn a_handlers_status_and_headers_survive_the_crossing() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.get("marked", |_context| {
                respond_with(
                    StatusCode::CREATED,
                    Some(Document {
                        content: PrimaryContent::Empty { data: () },
                        meta: None,
                        jsonapi: None,
                        links: None,
                        included: None,
                    }),
                )
                .map(|mut response| {
                    response
                        .headers_mut()
                        .insert("x-mark", HeaderValue::from_static("seen"));
                    response
                })
            })
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/marked")
        .body(stream)?;

    let response = router.handle(&connection_manager, request)?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get("x-mark"),
        Some(&HeaderValue::from_static("seen"))
    );

    Ok(())
}

#[test]
fn a_resource_handler_is_bound_to_the_schema_it_was_mounted_with() -> Result {
    let connection_manager = database::build_database([])?;
    let articles = connection_manager.registry().schema("articles")?;
    let comments = connection_manager.registry().schema("comments")?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.resource_with::<Articles>("articles", articles, |resource| {
            resource.get("marked", |context| {
                respond_with(
                    StatusCode::OK,
                    Some(Document {
                        content: PrimaryContent::Record {
                            data: Box::new(Resource {
                                identifier: Identifier::Existing {
                                    kind: context.schema().name().to_string(),
                                    id: "1".to_string(),
                                },
                                attributes: None,
                                relationships: None,
                                links: None,
                                meta: None,
                            }),
                        },
                        meta: None,
                        jsonapi: None,
                        links: None,
                        included: None,
                    }),
                )
            })
        })
        .resource_with::<Comments>("comments", comments, |resource| {
            resource.get("marked", |context| {
                respond_with(
                    StatusCode::OK,
                    Some(Document {
                        content: PrimaryContent::Record {
                            data: Box::new(Resource {
                                identifier: Identifier::Existing {
                                    kind: context.schema().name().to_string(),
                                    id: "1".to_string(),
                                },
                                attributes: None,
                                relationships: None,
                                links: None,
                                meta: None,
                            }),
                        },
                        meta: None,
                        jsonapi: None,
                        links: None,
                        included: None,
                    }),
                )
            })
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/comments/marked")
        .body(stream)?;

    let body = router.handle(&connection_manager, request)?.into_body();
    let served = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(
        serde_json::from_str::<Value>(&served)?,
        json!({ "data": { "type": "comments", "id": "1" } })
    );

    Ok(())
}

#[test]
fn a_raw_response_reaches_the_client_untouched() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("download", |_context| {
            let stream: ByteStream =
                Box::new(Cursor::new(b"the bytes a handler streamed".to_vec()));
            respond_with(StatusCode::OK, Some(stream)).map_err(Into::into)
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/download")
        .body(stream)?;

    let (parts, body) = router.handle(&connection_manager, request)?.into_parts();
    let served = body
        .map(io::read_to_string)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(served, "the bytes a handler streamed");
    assert_eq!(parts.headers.get(CONTENT_TYPE), None);

    Ok(())
}

#[test]
fn a_raw_handlers_failure_escapes_to_the_embedder() -> Result {
    let connection_manager = database::build_database([])?;
    let router = Router::try_new(BaseUri::Relative, |root| {
        root.get("boom", |_context| -> PrimaryResult {
            Err("the handler could not answer".into())
        })
    })?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/boom")
        .body(stream)?;

    assert!(router.handle(&connection_manager, request).is_err());

    Ok(())
}
