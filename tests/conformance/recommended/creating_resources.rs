//! Creating Resources — recommended behaviour (spec §"Creating Resources").
//!
//! On a read-write resource these are SHOULD-level: a `201` SHOULD carry a
//! `Location`, and a conflict SHOULD carry enough detail to locate its source.

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};

fn new_article() -> Value {
    json!({
        "data": {
            "type": "articles",
            "attributes": { "title": "Fresh", "body": "New body", "published": false },
            "relationships": { "author": { "data": { "type": "authors", "id": "1" } } }
        }
    })
}

// "The response SHOULD include a Location header identifying the location of the
// newly created resource, in order to comply with RFC 7231."
#[test]
fn a_created_resource_carries_a_location_header() -> TestResult {
    let api = Api::new()?;
    let response = api.post(&collection_url("articles"), new_article())?;

    assert_eq!(response.status(), 201);
    let id = response
        .at("/data/id")
        .and_then(Value::as_str)
        .ok_or("created resource must carry an id")?;
    let location = response
        .header("Location")
        .ok_or("a 201 response should include a Location header")?;
    assert_eq!(location, resource_url("articles", id));
    Ok(())
}

// "A server SHOULD include error details and provide enough information to
// recognize the source of the conflict."
#[test]
fn a_creation_conflict_carries_error_detail() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": { "type": "comments", "attributes": { "content": "x" } } });
    let response = api.post(&collection_url("articles"), body)?;

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
