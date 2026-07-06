//! Deleting Resources — spec §"Deleting Resources".
//!
//! A failed delete of a missing resource is only a SHOULD (`404`), so it lives
//! in the recommended tier; only the mandatory success path is here.

use crate::test_support::*;
use crate::validations::*;

// "If a deletion request is successful, the server MUST return either a 200 OK
// status code and response document [...] or a 204 No Content status code with
// no response document." Removal is then confirmed by a follow-up fetch: "A
// server MUST respond with 404 Not Found when processing a request to fetch a
// single resource that does not exist [...]."
#[test]
fn deleting_a_resource_succeeds_and_removes_it() -> TestResult {
    let api = Api::new()?;
    let response = api.delete(&resource_url("articles", "2"))?;

    assert!(
        matches!(response.status(), 200 | 204),
        "got {}",
        response.status()
    );
    assert_eq!(validate_document(response.doc()), None);

    let refetch = api.get(&resource_url("articles", "2"))?;
    assert_eq!(refetch.status(), 404);
    Ok(())
}
