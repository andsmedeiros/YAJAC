//! Sorting — recommended behaviour (spec §"Sorting").
//!
//! Multi-field precedence is a SHOULD, and rides on the MAY `sort` affordance,
//! so it guards on the spec-defined non-support `400`.

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

// "Sort fields SHOULD be applied in the order specified."
#[test]
fn sort_fields_apply_in_the_order_specified() -> TestResult {
    let api = Api::new()?;
    // `active` ascending groups the inactive (Alice, Zed) ahead of the active
    // (Carol, Dave, Eve); `name` breaks ties within each group.
    let response = api.get(&format!("{}?sort=active,name", collection_url("authors")))?;

    if response.status() == 400 && !enforced(Affordance::Sort) {
        log::info!("`sort` unsupported (400); skipping");
        return Ok(());
    }

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    assert_eq!(ordered_ids(&response), ["2", "4", "1", "5", "3"]);
    Ok(())
}
