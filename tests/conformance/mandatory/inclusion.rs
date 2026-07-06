//! Inclusion of Related Resources — spec §"Inclusion of Related Resources",
//! §"Compound Documents".
//!
//! Supporting `include` is a MAY, so the happy-path invariants are guarded: when
//! the server signals non-support with the spec-defined `400`, the test skips
//! (unless the `include` affordance is enforced). An unresolvable path is a
//! universal `400` regardless of support, so those two are unguarded.

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use test_log::test;

fn identity_set(resources: &[Value]) -> BTreeSet<(String, String)> {
    resources
        .iter()
        .filter_map(|resource| {
            let kind = resource.get("type").and_then(Value::as_str)?;
            let id = resource.get("id").and_then(Value::as_str)?;
            Some((kind.to_owned(), id.to_owned()))
        })
        .collect()
}

fn included(response: &Res) -> Result<BTreeSet<(String, String)>, BoxError> {
    let array = response
        .at("/included")
        .and_then(Value::as_array)
        .ok_or("compound document must carry included")?;
    Ok(identity_set(array))
}

fn author(id: &str) -> (String, String) {
    ("authors".to_owned(), id.to_owned())
}

fn comment(id: &str) -> (String, String) {
    ("comments".to_owned(), id.to_owned())
}

// "If a server is unable to identify a relationship path or does not support
// inclusion of resources from a path, it MUST respond with 400 Bad Request."
#[test]
fn an_unresolvable_include_path_is_rejected() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?include=nonexistent",
        resource_url("articles", "1")
    ))?;

    assert_eq!(response.status(), 400);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If a server is unable to identify a relationship path [...] it MUST respond
// with 400 Bad Request."
#[test]
fn an_unresolvable_nested_include_path_is_rejected() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?include=author.bogus",
        resource_url("articles", "1")
    ))?;

    assert_eq!(response.status(), 400);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "The server's response MUST be a compound document with an included key [...].
// The server MUST NOT include unrequested resource objects in the included
// section of the compound document."
#[test]
fn including_a_to_one_relationship_produces_a_compound_document() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?include=author", resource_url("articles", "1")))?;

    if response.status() == 400 && !enforced(Affordance::Include) {
        log::info!("`include` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_full_linkage(response.doc()), None);
    // Article 1's author is author 1 (seed), included exactly once.
    assert_eq!(included(&response)?, BTreeSet::from([author("1")]));
    Ok(())
}

// "A compound document MUST NOT include more than one resource object for each
// type and id pair."
#[test]
fn included_resources_are_deduplicated_across_the_collection() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?include=author", collection_url("articles")))?;

    if response.status() == 400 && !enforced(Affordance::Include) {
        log::info!("`include` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_full_linkage(response.doc()), None);
    // Articles 1 and 2 share author 1; author 2 owns article 3.
    assert_eq!(
        included(&response)?,
        BTreeSet::from([author("1"), author("2")])
    );
    Ok(())
}

// "Every included resource object MUST be identified via a chain of
// relationships originating in a document's primary data."
#[test]
fn a_multi_level_path_includes_each_step() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?include=comments.author",
        resource_url("articles", "1")
    ))?;

    if response.status() == 400 && !enforced(Affordance::Include) {
        log::info!("`include` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_full_linkage(response.doc()), None);
    // Article 1's comments are 1 and 2; their authors are 2 and 1.
    assert_eq!(
        included(&response)?,
        BTreeSet::from([comment("1"), comment("2"), author("1"), author("2")])
    );
    Ok(())
}

// "Every included resource object MUST be identified via a chain of
// relationships originating in a document's primary data."
#[test]
fn a_self_referential_path_is_included() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?include=replies",
        resource_url("comments", "1")
    ))?;

    if response.status() == 400 && !enforced(Affordance::Include) {
        log::info!("`include` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(validate_full_linkage(response.doc()), None);
    // Comment 2 replies to comment 1 (seed).
    assert_eq!(included(&response)?, BTreeSet::from([comment("2")]));
    Ok(())
}

// "The server's response MUST be a compound document with an included key — even
// if that included key holds an empty array (because the requested
// relationships are empty)."
#[test]
fn an_empty_include_yields_an_empty_included() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?include=", resource_url("articles", "1")))?;

    if response.status() == 400 && !enforced(Affordance::Include) {
        log::info!("`include` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(response.at("/included"), Some(&json!([])));
    Ok(())
}
