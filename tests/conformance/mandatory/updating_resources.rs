//! Updating Resources — spec §"Updating Resources".

use crate::test_support::*;
use crate::validations::*;
use serde_json::json;

// "If a request does not include all of the attributes for a resource, the
// server MUST interpret the missing attributes as if they were included with
// their current values. The server MUST NOT interpret missing attributes as null
// values."
#[test]
fn omitted_attributes_keep_their_current_values() -> TestResult {
    let api = Api::new()?;
    let patch = json!({
        "data": { "type": "articles", "id": "1", "attributes": { "title": "Renamed" } }
    });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    // The untouched `body` must survive; the patched `title` must change.
    let refreshed = api.get(&resource_url("articles", "1"))?;
    if let Some(title) = refreshed.at("/data/attributes/title") {
        assert_eq!(title, &json!("Renamed"));
    }
    if let Some(body) = refreshed.at("/data/attributes/body") {
        assert_eq!(body, &json!("Body one"));
    }
    Ok(())
}

// "If a request does not include all of the relationships for a resource, the
// server MUST interpret the missing relationships as if they were included with
// their current values. It MUST NOT interpret them as null or empty values."
#[test]
fn omitted_relationships_keep_their_current_values() -> TestResult {
    let api = Api::new()?;
    let patch = json!({
        "data": { "type": "articles", "id": "1", "attributes": { "title": "Renamed" } }
    });
    api.patch(&resource_url("articles", "1"), patch)?;

    let refreshed = api.get(&resource_url("articles", "1"))?;
    if let Some(linkage) = refreshed.at("/data/relationships/author/data") {
        assert_eq!(linkage, &json!({ "type": "authors", "id": "1" }));
    }
    Ok(())
}

// "If a relationship is provided in the relationships member of a resource object
// in a PATCH request, its value MUST be a relationship object with a data member.
// The relationship's value will be replaced with the value specified [...]."
#[test]
fn a_supplied_relationship_replaces_the_current_value() -> TestResult {
    let api = Api::new()?;
    let patch = json!({
        "data": {
            "type": "articles",
            "id": "1",
            "relationships": { "author": { "data": { "type": "authors", "id": "2" } } }
        }
    });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let refreshed = api.get(&resource_url("articles", "1"))?;
    if let Some(linkage) = refreshed.at("/data/relationships/author/data") {
        assert_eq!(linkage, &json!({ "type": "authors", "id": "2" }));
    }
    Ok(())
}

// "A server MUST return 409 Conflict when processing a PATCH request in which the
// resource object's type or id do not match the server's endpoint."
#[test]
fn a_type_mismatch_is_a_conflict() -> TestResult {
    let api = Api::new()?;
    let patch = json!({ "data": { "type": "comments", "id": "1" } });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert_eq!(response.status(), 409);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST return 409 Conflict when processing a PATCH request in which the
// resource object's type or id do not match the server's endpoint."
#[test]
fn an_id_mismatch_is_a_conflict() -> TestResult {
    let api = Api::new()?;
    let patch = json!({ "data": { "type": "articles", "id": "2" } });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert_eq!(response.status(), 409);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "The PATCH request MUST include a single resource object as primary data. The
// resource object MUST contain type and id members."
#[test]
fn a_body_without_a_type_is_rejected() -> TestResult {
    let api = Api::new()?;
    let patch = json!({ "data": { "id": "1", "attributes": { "title": "X" } } });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "The resource object MUST contain type and id members."
#[test]
fn a_body_without_an_id_is_rejected() -> TestResult {
    let api = Api::new()?;
    let patch = json!({ "data": { "type": "articles", "attributes": { "title": "X" } } });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If a relationship is provided in the relationships member of a resource
// object in a PATCH request, its value MUST be a relationship object with a data
// member."
#[test]
fn a_relationship_without_a_data_member_is_rejected() -> TestResult {
    let api = Api::new()?;
    let patch = json!({
        "data": {
            "type": "articles",
            "id": "1",
            "relationships": { "author": { "links": { "self": "x" } } }
        }
    });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST return 404 Not Found when processing a request that references a
// related resource that does not exist."
#[test]
fn referencing_a_missing_related_resource_is_not_found() -> TestResult {
    let api = Api::new()?;
    let patch = json!({
        "data": {
            "type": "articles",
            "id": "1",
            "relationships": { "author": { "data": { "type": "authors", "id": "999" } } }
        }
    });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST return 404 Not Found when processing a request to modify a
// resource that does not exist."
#[test]
fn updating_a_missing_resource_is_not_found() -> TestResult {
    let api = Api::new()?;
    let patch = json!({ "data": { "type": "articles", "id": "999" } });
    let response = api.patch(&resource_url("articles", "999"), patch)?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
