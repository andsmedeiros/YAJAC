//! Query Parameters — spec §"Query Parameters" (Implementation-Specific).
//!
//! Implementation-specific parameters must come from a family whose base name
//! carries a non-`a-z` character; an all-lowercase base is neither reserved nor
//! legal, so the server must reject it.

use crate::test_support::*;
use crate::validations::*;

// "If a server encounters a query parameter that does not follow the naming
// conventions above, or the server does not know how to process it as a query
// parameter from this specification, it MUST return 400 Bad Request."
#[test]
fn an_invalidly_named_query_parameter_is_rejected() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?foobar=1", collection_url("authors")))?;

    assert_eq!(response.status(), 400);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If a server encounters a query parameter that does not follow the naming
// conventions above [...] it MUST return 400 Bad Request."
#[test]
fn an_invalidly_named_parameter_family_is_rejected() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!("{}?foo[bar]=1", collection_url("authors")))?;

    assert_eq!(response.status(), 400);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
