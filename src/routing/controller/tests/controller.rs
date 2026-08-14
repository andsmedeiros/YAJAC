use super::*;
use test_log::test;

#[test]
fn parameters_for_route_resolves_the_id_from_the_record() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/articles/2")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Articles.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["id"],
    );

    assert_eq!(resolved, HashMap::from([("id", Cow::Borrowed("2"))]));

    Ok(())
}

#[test]
fn parameters_for_route_echoes_the_parameters_the_request_carries() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
    ])?;
    let base = BaseUri::Relative;
    let router = build_scoped_router(&manager, base.clone())?;
    let schema = manager.registry().schema("articles")?;

    let stream: ByteStream = Box::new(empty());
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/acme-press/articles/2")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Articles.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["id", "tenant"],
    );

    assert_eq!(
        resolved,
        HashMap::from([
            ("id", Cow::Borrowed("2")),
            ("tenant", Cow::Borrowed("acme-press"))
        ])
    );

    Ok(())
}

#[test]
fn parameters_for_route_omits_a_parameter_it_cannot_resolve() -> Result {
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
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Articles.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["id", "tenant"],
    );

    assert_eq!(resolved, HashMap::from([("id", Cow::Borrowed("1"))]));

    Ok(())
}

#[test]
fn parameters_for_route_resolves_what_a_controller_overrides() -> Result {
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
        .header("tenant", "acme-press")
        .body(stream)?;
    let uri: Uri = request.uri().clone().into();
    let context = routing::build_resource_context(&manager, &router, &base, &uri, request, schema)?;

    let record = context
        .store()?
        .fetch_record(schema, context.require_id()?, &QueryParameters::new(schema))?
        .content;
    let resolved = Tenanted.parameters_for_route(
        &record,
        context.route_parameters(),
        context.headers(),
        &["tenant"],
    );

    assert_eq!(
        resolved,
        HashMap::from([("tenant", Cow::Borrowed("acme-press"))])
    );

    Ok(())
}

#[test]
fn a_controller_refuses_client_generated_ids_unless_it_says_otherwise() -> Result {
    assert!(!Articles.configuration().accepts_client_ids);
    assert!(Publishers.configuration().accepts_client_ids);

    Ok(())
}
