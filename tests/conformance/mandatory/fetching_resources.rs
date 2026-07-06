//! Fetching Resources — spec §"Fetching Resources".

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

// "A server MUST respond to a successful request to fetch an individual resource
// [...] with a 200 OK response" and "with a resource object or null provided as
// the response document's primary data."
#[test]
fn fetching_an_existing_resource_yields_a_conformant_200() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&resource_url("authors", "1"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_urls(response.doc()), None);

    let data = response.at("/data").ok_or("missing primary data")?;
    assert!(
        data.is_object(),
        "individual data must be a single resource object"
    );
    assert_eq!(response.at("/data/type"), Some(&json!("authors")));
    assert_eq!(response.at("/data/id"), Some(&json!("1")));

    // Attributes are optional per spec; when present they must match the seed.
    if let Some(name) = response.at("/data/attributes/name") {
        assert_eq!(name, &json!("Carol"));
    }
    if let Some(active) = response.at("/data/attributes/active") {
        assert_eq!(active, &json!(true));
    }
    Ok(())
}

// "A server MUST respond to a successful request to fetch a resource collection
// with an array of resource objects or an empty array ([]) as the response
// document's primary data."
#[test]
fn fetching_a_collection_yields_a_conformant_200_array() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&collection_url("authors"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_urls(response.doc()), None);

    // Order is server-defined absent `sort`, so assert the set, not the order.
    let data = response
        .at("/data")
        .and_then(Value::as_array)
        .ok_or("collection data must be an array")?;
    let ids: BTreeSet<&str> = data
        .iter()
        .filter_map(|resource| resource.get("id").and_then(Value::as_str))
        .collect();
    let types: BTreeSet<&str> = data
        .iter()
        .filter_map(|resource| resource.get("type").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, BTreeSet::from(["1", "2", "3", "4", "5"]));
    assert_eq!(types, BTreeSet::from(["authors"]));
    Ok(())
}

// "A logical collection of resources MUST be represented as an array, even if it
// only contains one item or is empty."
#[test]
fn an_empty_collection_is_an_empty_array() -> TestResult {
    let api = Api::new()?;
    api.delete(&resource_url("tags", "rust"))?;
    api.delete(&resource_url("tags", "web"))?;

    let response = api.get(&collection_url("tags"))?;
    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/data"), Some(&json!([])));
    Ok(())
}

// "A server MUST respond with 404 Not Found when processing a request to fetch a
// single resource that does not exist, except when the request warrants a 200 OK
// response with null as the primary data."
#[test]
fn fetching_a_missing_resource_yields_a_conformant_404() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&resource_url("authors", "999"))?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
