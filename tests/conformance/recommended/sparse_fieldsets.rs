//! Sparse Fieldsets — recommended behaviour (spec §"Square Brackets in
//! Parameter Names").
//!
//! A compliant client percent-encodes the brackets in `fields[TYPE]`. Servers
//! SHOULD also accept the unencoded form and MUST treat it identically. This
//! pins the equivalence: encoded and unencoded requests yield the same fieldset.

use crate::test_support::*;
use crate::validations::*;
use serde_json::Value;
use std::collections::BTreeSet;

fn attribute_names(response: &Res) -> BTreeSet<String> {
    response
        .at("/data/attributes")
        .and_then(Value::as_object)
        .map(|members| members.keys().cloned().collect())
        .unwrap_or_default()
}

// "Servers SHOULD accept requests in which these square brackets are left
// unencoded in a query parameter's name. If a server does accept these requests,
// it MUST treat the request as equivalent to one in which the square brackets
// were percent-encoded."
#[test]
fn encoded_and_unencoded_bracket_parameters_are_equivalent() -> TestResult {
    let api = Api::new()?;
    let encoded = api.get(&format!(
        "{}?fields%5Bauthors%5D=name",
        resource_url("authors", "1")
    ))?;
    let unencoded = api.get(&format!(
        "{}?fields[authors]=name",
        resource_url("authors", "1")
    ))?;

    assert_eq!(encoded.status(), 200);
    assert_eq!(unencoded.status(), 200);
    // Both forms must restrict the author to exactly the `name` attribute.
    assert_eq!(
        attribute_names(&encoded),
        BTreeSet::from(["name".to_owned()])
    );
    assert_eq!(
        attribute_names(&unencoded),
        BTreeSet::from(["name".to_owned()])
    );
    Ok(())
}
