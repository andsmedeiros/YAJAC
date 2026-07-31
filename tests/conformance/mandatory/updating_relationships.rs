//! Updating Relationships — spec §"Updating Relationships".
//!
//! `articles` is read-write, so its relationship-link URLs accept the full
//! PATCH/POST/DELETE set with the spec's replace/add/remove semantics. Full
//! replacement of a to-many relationship (PATCH) and member deletion (DELETE)
//! each carry a spec-defined `403` opt-out ("complete replacement is not allowed
//! by the server"; "or return a 403 Forbidden response"), so those guard on it as
//! the optional affordances do elsewhere. POST-add and to-one updates have no
//! such opt-out and stay unconditional.

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use test_log::test;

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

fn article(id: &str) -> (String, String) {
    ("articles".to_owned(), id.to_owned())
}

// "The PATCH request MUST include a top-level member named data containing [...]
// a resource identifier object corresponding to the new related resource." "If
// the relationship is updated successfully then the server MUST return a
// successful response."
#[test]
fn replacing_a_to_one_relationship_takes_effect() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": { "type": "authors", "id": "2" } });
    let response = api.patch(&relationship_url("articles", "1", "author"), body)?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let after = api.get(&relationship_url("articles", "1", "author"))?;
    assert_eq!(
        after.at("/data"),
        Some(&json!({ "type": "authors", "id": "2" }))
    );
    Ok(())
}

// "The PATCH request MUST include a top-level member named data containing [...]
// null, to remove the relationship."
#[test]
fn clearing_a_to_one_relationship_takes_effect() -> TestResult {
    let api = Api::new()?;
    // Comment 2's parent is nullable; a to-one is cleared with `PATCH` null.
    let response = api.patch(
        &relationship_url("comments", "2", "parent"),
        json!({ "data": null }),
    )?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let after = api.get(&relationship_url("comments", "2", "parent"))?;
    assert_eq!(after.at("/data"), Some(&Value::Null));
    Ok(())
}

// "If a client makes a POST request to a URL from a relationship link, the server
// MUST add the specified members to the relationship unless they are already
// present."
#[test]
fn adding_to_a_to_many_relationship_takes_effect() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": [{ "type": "comments", "id": "3" }] });
    let response = api.post(&relationship_url("articles", "1", "comments"), body)?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let after = api.get(&relationship_url("articles", "1", "comments"))?;
    assert_eq!(
        linkage_set(&after),
        BTreeSet::from([comment("1"), comment("2"), comment("3")])
    );
    Ok(())
}

// "If a given type and id is already in the relationship, the server MUST NOT add
// it again."
#[test]
fn adding_an_existing_member_does_not_duplicate_it() -> TestResult {
    let api = Api::new()?;
    // Comment 1 is already in the relationship.
    let body = json!({ "data": [{ "type": "comments", "id": "1" }] });
    let response = api.post(&relationship_url("articles", "1", "comments"), body)?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );

    let after = api.get(&relationship_url("articles", "1", "comments"))?;
    assert_eq!(
        linkage_set(&after),
        BTreeSet::from([comment("1"), comment("2")])
    );
    Ok(())
}

// "If a client makes a PATCH request to a URL from a to-many relationship link,
// the server MUST either completely replace every member of the relationship,
// return an appropriate error response if some resources cannot be found or
// accessed, or return a 403 Forbidden response if complete replacement is not
// allowed by the server."
#[test]
fn replacing_a_to_many_relationship_takes_effect() -> TestResult {
    let api = Api::new()?;
    // `authors/3/edited` is nullable, so full replacement can actually take effect.
    let body = json!({ "data": [{ "type": "articles", "id": "2" }] });
    let response = api.patch(&relationship_url("authors", "3", "edited"), body)?;

    if response.status() == 403 && !enforced(Affordance::FullReplacement) {
        log::info!("full to-many replacement unsupported (403); skipping");
        return Ok(());
    }

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let after = api.get(&relationship_url("authors", "3", "edited"))?;
    assert_eq!(linkage_set(&after), BTreeSet::from([article("2")]));
    Ok(())
}

// "For all request types, the body MUST contain a data member whose value is an
// empty array or an array of resource identifier objects." — `[]` is a complete
// replacement to empty, which the server MAY decline: "return a 403 Forbidden
// response if complete replacement is not allowed by the server."
#[test]
fn clearing_a_to_many_relationship_takes_effect() -> TestResult {
    let api = Api::new()?;
    let response = api.patch(
        &relationship_url("authors", "3", "edited"),
        json!({ "data": [] }),
    )?;

    if response.status() == 403 && !enforced(Affordance::FullReplacement) {
        log::info!("full to-many replacement unsupported (403); skipping");
        return Ok(());
    }

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let after = api.get(&relationship_url("authors", "3", "edited"))?;
    assert_eq!(after.at("/data"), Some(&json!([])));
    Ok(())
}

// "If the client makes a DELETE request to a URL from a relationship link the
// server MUST delete the specified members from the relationship or return a 403
// Forbidden response."
#[test]
fn deleting_named_members_leaves_the_rest() -> TestResult {
    let api = Api::new()?;
    let body = json!({ "data": [{ "type": "articles", "id": "1" }] });
    let response = api.request(
        "DELETE",
        &relationship_url("authors", "3", "edited"),
        body,
    )?;

    if response.status() == 403 && !enforced(Affordance::RelationshipDelete) {
        log::info!("relationship-member deletion unsupported (403); skipping");
        return Ok(());
    }

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    // Only article 1 is removed; article 2 remains.
    let after = api.get(&relationship_url("authors", "3", "edited"))?;
    assert_eq!(linkage_set(&after), BTreeSet::from([article("2")]));
    Ok(())
}

// "If the client makes a DELETE request to a URL from a relationship link the
// server MUST delete the specified members from the relationship or return a 403
// Forbidden response. If all of the specified resources are able to be removed
// from, or are already missing from, the relationship then the server MUST return
// a successful response."
#[test]
fn deleting_an_absent_member_still_succeeds() -> TestResult {
    let api = Api::new()?;
    // Comment 3 belongs to article 3, so it is not in article 1's relationship.
    let body = json!({ "data": [{ "type": "comments", "id": "3" }] });
    let response = api.request(
        "DELETE",
        &relationship_url("articles", "1", "comments"),
        body,
    )?;

    if response.status() == 403 && !enforced(Affordance::RelationshipDelete) {
        log::info!("relationship-member deletion unsupported (403); skipping");
        return Ok(());
    }

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "For all request types, the body MUST contain a data member whose value is an
// empty array or an array of resource identifier objects."
#[test]
fn a_to_many_update_without_a_data_member_is_rejected() -> TestResult {
    let api = Api::new()?;
    let response = api.post(
        &relationship_url("articles", "1", "comments"),
        json!({ "meta": {} }),
    )?;

    assert!(
        response.is_client_error(),
        "expected 4xx, got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
