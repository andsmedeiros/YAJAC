//! Content Negotiation — spec §"Content Negotiation".

use crate::test_support::*;
use crate::validations::*;
use serde_json::Value;

const UNKNOWN_EXT: &str = "application/vnd.api+json; ext=\"https://example.test/ext/unknown\"";
const UNKNOWN_PROFILE: &str =
    "application/vnd.api+json; profile=\"https://example.test/profile/unknown\"";
const PARAMETERISED: &str = "application/vnd.api+json; charset=utf-8";
const MIXED_ACCEPT: &str = "application/vnd.api+json; charset=utf-8, application/vnd.api+json";

fn get_with(header: (&str, &str)) -> Result<Res, BoxError> {
    Api::new()?.request_with("GET", &collection_url("authors"), Value::Null, &[header])
}

// "Clients and servers MUST send all JSON:API payloads using the JSON:API media
// type in the Content-Type header."
#[test]
fn successful_responses_use_the_json_api_media_type() -> TestResult {
    let response = Api::new()?.get(&collection_url("authors"))?;

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Type").as_deref(), Some(JSONAPI));
    Ok(())
}

// "servers MUST respond with a 415 Unsupported Media Type status code if that
// media type contains any media type parameters other than ext or profile." — an
// unparameterised media type is therefore accepted.
#[test]
fn a_valid_content_type_is_accepted() -> TestResult {
    let response = get_with(("Content-Type", JSONAPI))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "servers MUST respond with a 415 Unsupported Media Type status code if that
// media type contains any media type parameters other than ext or profile."
#[test]
fn a_parameterised_content_type_is_unsupported() -> TestResult {
    let response = get_with(("Content-Type", PARAMETERISED))?;

    assert_eq!(response.status(), 415);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If a request specifies the Content-Type header with an instance of the
// JSON:API media type modified by the ext media type parameter and that
// parameter contains an unsupported extension URI, the server MUST respond with
// a 415 Unsupported Media Type status code."
#[test]
fn an_unsupported_extension_in_content_type_is_unsupported() -> TestResult {
    let response = get_with(("Content-Type", UNKNOWN_EXT))?;

    assert_eq!(response.status(), 415);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If a request's Accept header contains an instance of the JSON:API media type,
// servers MUST ignore instances of that media type which are modified by a media
// type parameter other than ext or profile."
#[test]
fn a_valid_accept_is_acceptable() -> TestResult {
    let response = get_with(("Accept", JSONAPI))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "servers MUST ignore instances of that media type which are modified by a
// media type parameter other than ext or profile." — the clean instance is served.
#[test]
fn an_acceptable_instance_among_others_is_served() -> TestResult {
    let response = get_with(("Accept", MIXED_ACCEPT))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If all instances of that media type are modified with a media type parameter
// other than ext or profile, servers MUST respond with a 406 Not Acceptable
// status code."
#[test]
fn a_fully_parameterised_accept_is_not_acceptable() -> TestResult {
    let response = get_with(("Accept", PARAMETERISED))?;

    assert_eq!(response.status(), 406);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "If every instance of that media type is modified by the ext parameter and
// each contains at least one unsupported extension URI, the server MUST also
// respond with a 406 Not Acceptable."
#[test]
fn an_unsupported_extension_in_accept_is_not_acceptable() -> TestResult {
    let response = get_with(("Accept", UNKNOWN_EXT))?;

    assert_eq!(response.status(), 406);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}

// "A server MUST ignore any profiles that it does not recognize."
#[test]
fn an_unrecognised_profile_is_ignored() -> TestResult {
    let response = get_with(("Accept", UNKNOWN_PROFILE))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);
    Ok(())
}
