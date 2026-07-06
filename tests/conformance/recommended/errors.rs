//! Errors — recommended behaviour (spec §"Errors").
//!
//! `status` SHOULD be provided; when a `source` is present it SHOULD carry one
//! of `pointer`/`parameter`/`header`. Structure itself is a MUST, checked in the
//! mandatory tier.

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};

const SOURCE_KEYS: [&str; 3] = ["pointer", "parameter", "header"];

/// Every error object in the response, or an error if the document has none.
fn errors(response: &Res) -> Result<Vec<Value>, BoxError> {
    Ok(response
        .at("/errors")
        .and_then(Value::as_array)
        .ok_or("error document must carry an errors array")?
        .clone())
}

// "status: the HTTP status code applicable to this problem, expressed as a string
// value. This SHOULD be provided."
#[test]
fn a_not_found_error_provides_its_status() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&resource_url("authors", "999"))?;

    assert_eq!(response.status(), 404);
    for error in errors(&response)? {
        assert_eq!(error.get("status"), Some(&json!("404")));
    }
    Ok(())
}

// "status: the HTTP status code applicable to this problem, expressed as a string
// value. This SHOULD be provided."
#[test]
fn a_bad_request_error_provides_its_status() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?foobar=1", collection_url("authors")))?;

    assert_eq!(response.status(), 400);
    for error in errors(&response)? {
        assert_eq!(error.get("status"), Some(&json!("400")));
    }
    Ok(())
}

// "source: an object containing references to the primary source of the error.
// It SHOULD include one of the following members or be omitted: pointer [...],
// parameter [...], header [...]."
#[test]
fn an_error_source_names_its_origin() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?foobar=1", collection_url("authors")))?;

    assert_eq!(response.status(), 400);
    for error in errors(&response)? {
        if let Some(source) = error.get("source") {
            let source = source.as_object().ok_or("source must be an object")?;
            assert!(
                SOURCE_KEYS.iter().any(|key| source.contains_key(*key)),
                "a present source should name a pointer, parameter, or header"
            );
        }
    }
    Ok(())
}
