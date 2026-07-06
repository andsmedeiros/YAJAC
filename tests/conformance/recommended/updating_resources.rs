//! Updating Resources — recommended behaviour (spec §"Updating Resources").
//!
//! A conflict SHOULD carry enough detail to locate its source.

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};

// "A server SHOULD include error details and provide enough information to
// recognize the source of the conflict."
#[test]
fn an_update_conflict_carries_error_detail() -> TestResult {
    let api = Api::new()?;
    // A type that does not match the endpoint is a 409 conflict.
    let patch = json!({ "data": { "type": "comments", "id": "1" } });
    let response = api.patch(&resource_url("articles", "1"), patch)?;

    assert_eq!(response.status(), 409);
    let errors = response
        .at("/errors")
        .and_then(Value::as_array)
        .ok_or("a conflict must carry an errors array")?;
    assert!(
        errors
            .iter()
            .any(|error| error.get("detail").and_then(Value::as_str).is_some()),
        "a conflict should include a human-readable detail"
    );
    Ok(())
}
