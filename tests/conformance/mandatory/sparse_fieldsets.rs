//! Sparse Fieldsets — spec §"Sparse Fieldsets".

use crate::test_support::*;
use crate::validations::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn field_names(resource: &Value, group: &str) -> BTreeSet<String> {
    resource
        .get(group)
        .and_then(Value::as_object)
        .map(|members| members.keys().cloned().collect())
        .unwrap_or_default()
}

// "If a client requests a restricted set of fields for a given resource type, an
// endpoint MUST NOT include additional fields in resource objects of that type
// in its response."
#[test]
fn restricting_a_type_excludes_its_other_fields() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?fields[authors]=name",
        collection_url("authors")
    ))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);

    let data = response
        .at("/data")
        .and_then(Value::as_array)
        .ok_or("collection data must be an array")?;
    for resource in data {
        // `name` is the only requested field: no other attribute, no relationship.
        assert!(
            field_names(resource, "attributes").is_subset(&BTreeSet::from(["name".to_owned()]))
        );
        assert!(field_names(resource, "relationships").is_empty());
    }
    Ok(())
}

// "A resource object's attributes and its relationships are collectively called
// its 'fields'." Fields thus comprise both, and since "an endpoint MUST NOT
// include additional fields" beyond those requested, a relationship may be
// selected as a field.
#[test]
fn a_relationship_may_be_selected_as_a_field() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?fields[authors]=articles",
        resource_url("authors", "1")
    ))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);

    let resource = response.at("/data").ok_or("missing primary data")?;
    assert!(field_names(resource, "attributes").is_empty());
    assert!(
        field_names(resource, "relationships").is_subset(&BTreeSet::from(["articles".to_owned()]))
    );
    Ok(())
}

// "An empty value indicates that no fields should be returned."
#[test]
fn an_empty_fieldset_yields_no_fields() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?fields[authors]=",
        resource_url("authors", "1")
    ))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);

    let resource = response.at("/data").ok_or("missing primary data")?;
    assert!(field_names(resource, "attributes").is_empty());
    assert!(field_names(resource, "relationships").is_empty());
    Ok(())
}

// "This section applies to any endpoint that responds with resources as primary
// or included data" — so the restriction reaches included resources too.
#[test]
fn sparse_fieldsets_apply_to_included_resources() -> TestResult {
    let api = Api::new()?;
    let response = api.get(&format!(
        "{}?include=author&fields[authors]=name",
        resource_url("articles", "1")
    ))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);

    let included = response
        .at("/included")
        .and_then(Value::as_array)
        .ok_or("compound document must carry included")?;
    for resource in included {
        if resource.get("type") == Some(&json!("authors")) {
            assert!(
                field_names(resource, "attributes").is_subset(&BTreeSet::from(["name".to_owned()]))
            );
            assert!(field_names(resource, "relationships").is_empty());
        }
    }
    Ok(())
}

// "If a client requests a restricted set of fields for a given resource type, an
// endpoint MUST NOT include additional fields in resource objects of that type"
// — each type is restricted by its own `fields[TYPE]`, independently.
#[test]
fn distinct_types_are_restricted_independently() -> TestResult {
    let api = Api::new()?;
    // Keep the `author` relationship on the article so the linkage stays; restrict
    // the included author to `name`.
    let response = api.get(&format!(
        "{}?include=author&fields[articles]=author&fields[authors]=name",
        resource_url("articles", "1")
    ))?;

    assert_eq!(response.status(), 200);
    assert_eq!(validate_document(response.doc()), None);

    let article = response.at("/data").ok_or("missing primary data")?;
    assert!(field_names(article, "attributes").is_empty());
    assert!(
        field_names(article, "relationships").is_subset(&BTreeSet::from(["author".to_owned()]))
    );

    let included = response
        .at("/included")
        .and_then(Value::as_array)
        .ok_or("compound document must carry included")?;
    for resource in included {
        if resource.get("type") == Some(&json!("authors")) {
            assert!(
                field_names(resource, "attributes").is_subset(&BTreeSet::from(["name".to_owned()]))
            );
            assert!(field_names(resource, "relationships").is_empty());
        }
    }
    Ok(())
}
