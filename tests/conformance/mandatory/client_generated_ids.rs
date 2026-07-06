//! Client-Generated IDs — spec §"Client-Generated IDs", §"Creating Resources".
//!
//! Accepting a client-supplied `id` is a MAY, so both invariants guard on the
//! spec-defined non-support `403` (skipped unless the `client-ids` affordance is
//! enforced). When supported, the id is honoured and a collision is a `409`.

use crate::test_support::*;
use crate::validations::*;
use serde_json::json;
use test_log::test;

// "A server MAY accept a client-generated ID along with a request to create a
// resource." — when it does, the resource exists at the client's id. (Non-
// support: "A server MUST return 403 Forbidden in response to an unsupported
// request to create a resource with a client-generated ID.")
#[test]
fn a_client_generated_id_is_honoured() -> TestResult {
    let api = Api::new()?;
    let body =
        json!({ "data": { "type": "tags", "id": "swift", "attributes": { "label": "Swift" } } });
    let response = api.post(&collection_url("tags"), body)?;

    if response.status() == 403 && !enforced(Affordance::ClientIds) {
        log::info!("`client-ids` unsupported (403); skipping");
        return Ok(());
    }

    // Creation without a server change may be reported as 201 (with or without a
    // document) or 204; either way the resource must exist at the client's id.
    assert!(
        matches!(response.status(), 201 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    if response.at("/data").is_some() {
        assert_eq!(response.at("/data/id"), Some(&json!("swift")));
    }

    let fetched = api.get(&resource_url("tags", "swift"))?;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.at("/data/id"), Some(&json!("swift")));
    Ok(())
}

// "A server MUST return 409 Conflict when processing a POST request to create a
// resource with a client-generated ID that already exists."
#[test]
fn a_duplicate_client_generated_id_is_a_conflict() -> TestResult {
    let api = Api::new()?;
    // Tag `rust` is seeded.
    let body =
        json!({ "data": { "type": "tags", "id": "rust", "attributes": { "label": "Dup" } } });
    let response = api.post(&collection_url("tags"), body)?;

    if response.status() == 403 && !enforced(Affordance::ClientIds) {
        log::info!("`client-ids` unsupported (403); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 409);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
