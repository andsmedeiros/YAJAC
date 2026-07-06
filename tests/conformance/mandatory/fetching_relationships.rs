//! Fetching Relationships — spec §"Fetching Relationships".
//!
//! The contract exposes relationship and related URLs, so the server MUST answer
//! `GET` on them: relationship URLs return resource linkage, related URLs return
//! the related resource(s).

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn identity_set(resources: &[Value]) -> BTreeSet<(String, String)> {
    resources
        .iter()
        .filter_map(|identifier| {
            let kind = identifier.get("type").and_then(Value::as_str)?;
            let id = identifier.get("id").and_then(Value::as_str)?;
            Some((kind.to_owned(), id.to_owned()))
        })
        .collect()
}

fn linkage_set(response: &Res) -> BTreeSet<(String, String)> {
    response
        .doc()
        .pointer("/data")
        .and_then(Value::as_array)
        .map(|linkage| identity_set(linkage))
        .unwrap_or_default()
}

fn comment(id: &str) -> (String, String) {
    ("comments".to_owned(), id.to_owned())
}

// "The primary data in the response document MUST match the appropriate value
// for resource linkage" — "a single resource identifier object for non-empty
// to-one relationships."
#[test]
fn fetching_a_to_one_relationship_yields_its_linkage() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&relationship_url("articles", "1", "author"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(
        response.at("/data"),
        Some(&json!({ "type": "authors", "id": "1" }))
    );
    Ok(())
}

// "The primary data in the response document MUST match the appropriate value
// for resource linkage" — "an array of resource identifier objects for non-empty
// to-many relationships."
#[test]
fn fetching_a_to_many_relationship_yields_its_linkage() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&relationship_url("articles", "1", "comments"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(
        linkage_set(&response),
        BTreeSet::from([comment("1"), comment("2")])
    );
    Ok(())
}

// "The primary data in the response document MUST match the appropriate value
// for resource linkage" — "null for empty to-one relationships."
#[test]
fn fetching_an_empty_to_one_relationship_yields_null() -> TestResult {
    let api = Api::new()?;
    // Comment 1 has no parent.
    let response = api.get(&relationship_url("comments", "1", "parent"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/data"), Some(&Value::Null));
    Ok(())
}

// "The primary data in the response document MUST match the appropriate value
// for resource linkage" — "an empty array ([]) for empty to-many relationships."
#[test]
fn fetching_an_empty_to_many_relationship_yields_an_empty_array() -> TestResult {
    let api = Api::new()?;
    // Article 2 has no comments.
    let response = api.get(&relationship_url("articles", "2", "comments"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/data"), Some(&json!([])));
    Ok(())
}

// "A server MUST return 404 Not Found when processing a request to fetch a
// relationship link URL that does not exist."
#[test]
fn fetching_an_unknown_relationship_is_not_found() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&relationship_url("articles", "1", "nonexistent"))?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST return 404 Not Found when processing a request to fetch a
// relationship link URL that does not exist." — the URL does not exist because
// its parent resource (article 999) does not.
#[test]
fn fetching_a_relationship_of_a_missing_resource_is_not_found() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&relationship_url("articles", "999", "author"))?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "When fetched, the related resource object(s) are returned as the response's
// primary data."
#[test]
fn fetching_a_related_to_one_resource_yields_the_resource() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&related_url("articles", "1", "author"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    // The related link resolves to the related resource itself, not its linkage.
    assert_eq!(response.at("/data/type"), Some(&json!("authors")));
    assert_eq!(response.at("/data/id"), Some(&json!("1")));
    Ok(())
}

// "When fetched, the related resource object(s) are returned as the response's
// primary data."
#[test]
fn fetching_a_related_to_many_resource_yields_a_collection() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&related_url("articles", "1", "comments"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(
        linkage_set(&response),
        BTreeSet::from([comment("1"), comment("2")])
    );
    Ok(())
}

// "null is only an appropriate response when the requested URL is one that might
// correspond to a single resource, but doesn't currently."
#[test]
fn fetching_an_empty_related_to_one_resource_yields_null() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&related_url("comments", "1", "parent"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/data"), Some(&Value::Null));
    Ok(())
}

// "A logical collection of resources MUST be represented as an array, even if it
// only contains one item or is empty." — an empty to-many related collection is
// therefore `[]`, not null.
#[test]
fn fetching_an_empty_related_to_many_resource_yields_an_empty_array() -> TestResult {
    let api = Api::new()?;
    // Article 2 has no comments.
    let response = api.get(&related_url("articles", "2", "comments"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/data"), Some(&json!([])));
    Ok(())
}
