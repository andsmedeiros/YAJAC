//! Creating Resources — spec §"Creating Resources".
//!
//! The `articles` resource is contractually read-write, so creation must
//! succeed and its spec-mandated failure codes apply. The spec mandates the
//! request document's structure but not a status for a malformed one, so
//! malformed requests assert a client error (`4xx`), while the explicitly-coded
//! conflicts and not-founds assert their exact code.

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};

fn new_article(author_id: &str) -> Value {
    json!({
        "data": {
            "type": "articles",
            "attributes": { "title": "Fresh", "body": "New body", "published": false },
            "relationships": {
                "author": { "data": { "type": "authors", "id": author_id } }
            }
        }
    })
}

// "If the requested resource has been created successfully and the server changes
// the resource in any way (for example, by assigning an id), the server MUST
// return a 201 Created response and a document that contains the resource as
// primary data." "If the resource object returned by the response contains a self
// key in its links member and a Location header is provided, the value of the
// self member MUST match the value of the Location header."
#[test]
fn creating_a_resource_yields_a_conformant_201() -> TestResult {
    let api = Api::new()?;
    let response = api.post(&collection_url("articles"), new_article("1"))?;

    assert_eq!(response.status(), 201);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_urls(response.doc()), None);

    // The created resource is the primary data, with a server-assigned id.
    assert_eq!(response.at("/data/type"), Some(&json!("articles")));
    let id = response
        .at("/data/id")
        .and_then(Value::as_str)
        .ok_or("created resource must carry an id")?
        .to_owned();

    // When a self link and a Location header are both present, they must agree.
    if let Some(location) = response.header("Location")
        && let Some(self_link) = response.at("/data/links/self").and_then(Value::as_str)
    {
        assert_eq!(self_link, location);
    }

    // The resource now exists and is fetchable.
    let fetched = api.get(&resource_url("articles", &id))?;
    assert_eq!(fetched.status(), 200);
    Ok(())
}

// "A server MUST return 409 Conflict when processing a POST request in which the
// resource object's type is not among the type(s) that constitute the collection
// represented by the endpoint."
#[test]
fn a_type_mismatch_is_a_conflict() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": { "type": "comments", "attributes": { "content": "x" } } });
    let response = api.post(&collection_url("articles"), body)?;

    assert_eq!(response.status(), 409);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST return 404 Not Found when processing a request that references a
// related resource that does not exist."
#[test]
fn referencing_a_missing_related_resource_is_not_found() -> TestResult {
    let api = Api::new()?;
    let response = api.post(&collection_url("articles"), new_article("999"))?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "The resource object MUST contain at least a type member."
#[test]
fn a_resource_object_without_a_type_is_rejected() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": { "attributes": { "title": "No type" } } });
    let response = api.post(&collection_url("articles"), body)?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "The request MUST include a single resource object as primary data."
#[test]
fn primary_data_that_is_not_a_single_resource_is_rejected() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": [{ "type": "articles", "attributes": { "title": "Array" } }] });
    let response = api.post(&collection_url("articles"), body)?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If a relationship is provided in the relationships member of the resource
// object, its value MUST be a relationship object with a data member."
#[test]
fn a_relationship_without_a_data_member_is_rejected() -> TestResult {
    let api = Api::new()?;
    let body = json!({
        "data": {
            "type": "articles",
            "attributes": { "title": "T", "body": "B", "published": true },
            "relationships": { "author": { "links": { "self": "x" } } }
        }
    });
    let response = api.post(&collection_url("articles"), body)?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
