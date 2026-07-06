//! Read-only resources — spec §"Updating Resources" (403 Forbidden).
//!
//! The contract marks `profiles` read-only: reads are served, but an attempt to
//! update the resource or one of its relationships is an unsupported update,
//! which the spec requires be answered with `403 Forbidden`.

use crate::test_support::*;
use crate::validations::*;
use serde_json::json;

// "A server MUST respond to a successful request to fetch an individual resource
// [...] with a 200 OK response" — read-only still serves reads.
#[test]
fn a_read_only_resource_is_still_fetchable() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&resource_url("profiles", "1"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/data/type"), Some(&json!("profiles")));
    assert_eq!(response.at("/data/id"), Some(&json!("1")));
    Ok(())
}

// "A server MUST return 403 Forbidden in response to an unsupported request to
// update a resource or relationship."
#[test]
fn updating_a_read_only_resource_is_forbidden() -> TestResult {
    let api = Api::new()?;
    let patch = json!({
        "data": { "type": "profiles", "id": "1", "attributes": { "bio": "Rewritten" } }
    });
    let response = api.patch(&resource_url("profiles", "1"), patch)?;

    assert_eq!(response.status(), 403);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST return 403 Forbidden in response to an unsupported request to
// update a resource or relationship."
#[test]
fn updating_a_read_only_resources_relationship_is_forbidden() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let response = api.patch(&relationship_url("profiles", "1", "author"), body)?;

    assert_eq!(response.status(), 403);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
