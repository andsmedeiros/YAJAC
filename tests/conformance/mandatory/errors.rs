//! Errors — spec §"Errors".
//!
//! Error-object *structure* is validated on every error response by
//! `validate_document`. Including error details is itself a MAY ("A server MAY
//! include error details with error responses"), so a bare error response with no
//! `errors` array is conformant. What is mandatory is the `status` *value*: when
//! an error object carries one, it is the applicable HTTP code as a string. (That
//! it be provided at all is a SHOULD, tested in the recommended tier.)

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};

// "status: the HTTP status code applicable to this problem, expressed as a
// string value." — when present, it is the response's own code.
#[test]
fn a_not_found_error_status_is_404() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&resource_url("authors", "999"))?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    // Error details are optional; assert the status value only where they exist.
    if let Some(errors) = response.at("/errors").and_then(Value::as_array) {
        for error in errors {
            if let Some(status) = error.get("status") {
                assert_eq!(status, &json!("404"));
            }
        }
    }
    Ok(())
}

// "status: the HTTP status code applicable to this problem, expressed as a
// string value."
#[test]
fn a_bad_request_error_status_is_400() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?foobar=1", collection_url("authors")))?;

    assert_eq!(response.status(), 400);
    assert_eq!(validate_document(response.doc()), None);
    if let Some(errors) = response.at("/errors").and_then(Value::as_array) {
        for error in errors {
            if let Some(status) = error.get("status") {
                assert_eq!(status, &json!("400"));
            }
        }
    }
    Ok(())
}
