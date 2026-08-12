use super::*;
use serde_json::json;

#[test]
fn a_routing_fault_drains_with_everything_it_named() {
    let routing = RoutingError::UnknownAttribute {
        kind: "articles".to_string(),
        attribute: "subtitle".to_string(),
    };
    let error = Error::from(routing.clone());

    assert_eq!(error.status, routing.status());
    assert_eq!(error.code, routing.code());
    assert_eq!(error.title, routing.title());
    assert_eq!(error.detail, routing.to_string());
    assert_eq!(error.source.as_deref(), routing.source().as_ref());
    assert_eq!(error.meta, None);
}

#[test]
fn a_database_fault_drains_without_naming_a_source() {
    let database = DatabaseError::RecordNotFound;
    let error = Error::from(database.clone());

    assert_eq!(error.status, database.status());
    assert_eq!(error.code, database.code());
    assert_eq!(error.detail, database.to_string());
    assert_eq!(error.source, None);
}

/// The detail is what distinguishes one occurrence from another; the title never moves.
#[test]
fn draining_keeps_the_title_generic_and_the_detail_specific() {
    let error = Error::from(RoutingError::ResourceIdMismatch {
        expected: "1".to_string(),
        actual: "2".to_string(),
    });

    assert_ne!(error.detail, error.title);
}

#[test]
fn the_wire_projection_carries_every_member_across() {
    let error = Error {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: Cow::Borrowed("UnknownAttribute"),
        title: Cow::Borrowed("The resource has no such attribute"),
        detail: "The resource type 'articles' has no attribute named 'subtitle'".to_string(),
        source: Some(Box::new(pointer::for_attribute("subtitle"))),
        meta: Some(Box::new(json!({ "line": 2 }))),
    };

    let wire = JsonApiError::from(error);

    assert_eq!(wire.status, Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(wire.code.as_deref(), Some("UnknownAttribute"));
    assert_eq!(
        wire.title.as_deref(),
        Some("The resource has no such attribute")
    );
    assert_eq!(
        wire.detail.as_deref(),
        Some("The resource type 'articles' has no attribute named 'subtitle'")
    );
    assert_eq!(wire.source, Some(pointer::for_attribute("subtitle")));
    assert_eq!(wire.meta, Some(json!({ "line": 2 })));
}

/// Neither member has a framework-side source, so the projection leaves both for a consumer.
#[test]
fn the_wire_projection_adds_no_id_or_links() {
    let wire = JsonApiError::from(Error::from(DatabaseError::RecordNotFound));

    assert!(wire.id.is_none());
    assert!(wire.links.is_none());
}

/// The whole point of the funnel: a source named at the raising site reaches the serialised
/// document under the member the standard names it by.
#[test]
fn a_named_source_survives_to_the_serialised_document() -> Result<(), serde_json::Error> {
    let error = Error::from(RoutingError::ResourceTypeMismatch {
        expected: "articles".to_string(),
        actual: "comments".to_string(),
    });

    let document = serde_json::to_value(JsonApiError::from(error))?;

    assert_eq!(document["source"], json!({ "pointer": "/data/type" }));
    assert_eq!(document["status"], json!("409"));
    Ok(())
}
