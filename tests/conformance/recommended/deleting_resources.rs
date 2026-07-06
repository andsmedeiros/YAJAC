//! Deleting Resources — recommended behaviour (spec §"Deleting Resources").
//!
//! Deleting a resource that does not exist SHOULD fail with `404`.

use crate::test_support::*;
use crate::validations::*;

// "A server SHOULD return a 404 Not Found status code if a deletion request fails
// due to the resource not existing."
#[test]
fn deleting_a_missing_resource_is_not_found() -> TestResult {
    let api = Api::new()?;
    let response = api.delete(&resource_url("articles", "999"))?;

    assert_eq!(response.status(), 404);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
