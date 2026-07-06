//! Sorting — spec §"Sorting".
//!
//! Supporting `sort` is a MAY, so the ordering invariants are guarded on the
//! spec-defined non-support `400` (skipped unless the `sort` affordance is
//! enforced). An unsupported field is a universal `400` regardless, so that one
//! is unguarded.

use crate::test_support::*;
use crate::validations::*;
use serde_json::Value;
use test_log::test;

fn ordered_ids(response: &Res) -> Vec<String> {
    response
        .doc()
        .pointer("/data")
        .and_then(Value::as_array)
        .map(|data| {
            data.iter()
                .filter_map(|resource| resource.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

// "If the server does not support sorting as specified in the query parameter
// sort, it MUST return 400 Bad Request."
#[test]
fn an_unsupported_sort_field_is_rejected() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?sort=nonexistent", collection_url("authors")))?;

    assert_eq!(response.status(), 400);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If sorting is supported by the server and requested by the client via query
// parameter sort, the server MUST return elements of the top-level data array of
// the response ordered according to the criteria specified."
#[test]
fn ascending_sort_orders_by_name() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?sort=name", collection_url("authors")))?;

    if response.status() == 400 && !enforced(Affordance::Sort) {
        log::info!("`sort` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(ordered_ids(&response), ["2", "1", "5", "3", "4"]);
    Ok(())
}

// "The sort order for each sort field MUST be ascending unless it is prefixed
// with a minus (U+002D HYPHEN-MINUS, '-'), in which case it MUST be descending."
#[test]
fn a_leading_minus_sorts_by_name_descending() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?sort=-name", collection_url("authors")))?;

    if response.status() == 400 && !enforced(Affordance::Sort) {
        log::info!("`sort` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(ordered_ids(&response), ["4", "3", "5", "1", "2"]);
    Ok(())
}

// "[...] the server MUST return elements of the top-level data array of the
// response ordered according to the criteria specified."
#[test]
fn ascending_sort_orders_by_age() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?sort=age", collection_url("authors")))?;

    if response.status() == 400 && !enforced(Affordance::Sort) {
        log::info!("`sort` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(ordered_ids(&response), ["5", "2", "3", "4", "1"]);
    Ok(())
}

// "The sort order for each sort field MUST be ascending unless it is prefixed
// with a minus [...], in which case it MUST be descending."
#[test]
fn a_leading_minus_sorts_by_age_descending() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?sort=-age", collection_url("authors")))?;

    if response.status() == 400 && !enforced(Affordance::Sort) {
        log::info!("`sort` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(ordered_ids(&response), ["1", "4", "3", "2", "5"]);
    Ok(())
}
