//! Generic validation for the conformance suite.
//!
//! Reusable validators over a response document, each returning the first
//! violation (or `None` when clean):
//!
//! - [`validate_document`] — JSON:API v1.1 *grammar*: every required member
//!   present and well-typed, no member beyond the allowed set, recursively over
//!   document → resource → relationship → identifier → errors → links, plus the
//!   compound-document `type`+`id` uniqueness rule.
//! - [`validate_full_linkage`] — every `included` resource is reachable from
//!   primary data through relationship linkage.
//! - [`validate_urls`] — the application's *URL set*: any link a resource or its
//!   relationships carries must equal the URL its `type`+`id` imply.
//!
//! Each requires *only* what the contract mandates and validates optional
//! members solely when present. Everything request-specific (a resource's seeded
//! values, the status code, the top-level `self`/`Location` for a given request)
//! is asserted at the test site, not here.

// A shared toolbox: not every test module uses every validator or URL helper.
#![allow(dead_code)]

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub pointer: String,
    pub reason: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let at = if self.pointer.is_empty() {
            "/"
        } else {
            &self.pointer
        };
        write!(f, "at `{at}`: {}", self.reason)
    }
}

impl std::error::Error for ValidationError {}

type Check = Result<(), ValidationError>;

// --- JSON:API grammar -----------------------------------------------------

const TOP_LEVEL_MEMBERS: &[&str] = &["data", "errors", "meta", "jsonapi", "links", "included"];
const RESOURCE_MEMBERS: &[&str] = &[
    "type",
    "id",
    "lid",
    "attributes",
    "relationships",
    "links",
    "meta",
];
const RELATIONSHIP_MEMBERS: &[&str] = &["links", "data", "meta"];
const IDENTIFIER_MEMBERS: &[&str] = &["type", "id", "lid", "meta"];
const ERROR_MEMBERS: &[&str] = &[
    "id", "links", "status", "code", "title", "detail", "source", "meta",
];
const SOURCE_MEMBERS: &[&str] = &["pointer", "parameter", "header"];
const LINK_OBJECT_MEMBERS: &[&str] = &[
    "href",
    "rel",
    "describedby",
    "title",
    "type",
    "hreflang",
    "meta",
];
const JSONAPI_MEMBERS: &[&str] = &["version", "ext", "profile", "meta"];

/// Validates a whole response document against the JSON:API grammar.
/// `None` ⇒ structurally conformant. A `null` document (a `204` no-content
/// body) has nothing to validate and is conformant.
pub fn validate_document(document: &Value) -> Option<ValidationError> {
    validate_top_level(document).err()
}

fn fail(pointer: &str, reason: impl Into<String>) -> Check {
    Err(ValidationError {
        pointer: pointer.to_owned(),
        reason: reason.into(),
    })
}

fn object<'a>(value: &'a Value, pointer: &str) -> Result<&'a Map<String, Value>, ValidationError> {
    value.as_object().ok_or_else(|| ValidationError {
        pointer: pointer.to_owned(),
        reason: "must be a JSON object".to_owned(),
    })
}

fn array<'a>(value: &'a Value, pointer: &str) -> Result<&'a Vec<Value>, ValidationError> {
    value.as_array().ok_or_else(|| ValidationError {
        pointer: pointer.to_owned(),
        reason: "must be an array".to_owned(),
    })
}

/// Rejects any member outside `allowed`. `@`-prefixed members are ignored when
/// interpreting the spec and so are always tolerated.
fn forbid_foreign(map: &Map<String, Value>, pointer: &str, allowed: &[&str]) -> Check {
    for key in map.keys() {
        if key.starts_with('@') {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            return fail(pointer, format!("member `{key}` is not allowed here"));
        }
    }
    Ok(())
}

/// Requires a member to be present and a string.
fn require_string(map: &Map<String, Value>, pointer: &str, member: &str) -> Check {
    match map.get(member) {
        None => fail(pointer, format!("missing required member `{member}`")),
        Some(Value::String(_)) => Ok(()),
        Some(_) => fail(
            &format!("{pointer}/{member}"),
            format!("`{member}` must be a string"),
        ),
    }
}

/// Validates a member to be a string, only when it is present.
fn string_if_present(map: &Map<String, Value>, pointer: &str, member: &str) -> Check {
    if map.contains_key(member) {
        require_string(map, pointer, member)?;
    }
    Ok(())
}

/// Validates a member to be an object, only when it is present.
fn object_if_present(map: &Map<String, Value>, pointer: &str, member: &str) -> Check {
    if let Some(value) = map.get(member) {
        object(value, &format!("{pointer}/{member}"))?;
    }
    Ok(())
}

fn validate_top_level(document: &Value) -> Check {
    if document.is_null() {
        return Ok(());
    }
    let map = object(document, "")?;
    forbid_foreign(map, "", TOP_LEVEL_MEMBERS)?;

    let has = |member: &str| map.contains_key(member);
    if !has("data") && !has("errors") && !has("meta") {
        return fail(
            "",
            "document must contain at least one of `data`, `errors`, `meta`",
        );
    }
    if has("data") && has("errors") {
        return fail("", "`data` and `errors` must not coexist");
    }
    if has("included") && !has("data") {
        return fail("", "`included` must not be present without `data`");
    }

    if let Some(data) = map.get("data") {
        validate_primary_data(data, "/data")?;
    }
    if let Some(errors) = map.get("errors") {
        validate_errors(errors, "/errors")?;
    }
    if let Some(included) = map.get("included") {
        for (index, item) in array(included, "/included")?.iter().enumerate() {
            validate_resource(item, &format!("/included/{index}"))?;
        }
    }
    if let Some(links) = map.get("links") {
        validate_links(links, "/links")?;
    }
    if let Some(jsonapi) = map.get("jsonapi") {
        validate_jsonapi(jsonapi, "/jsonapi")?;
    }
    validate_unique_resources(document)?;
    object_if_present(map, "", "meta")
}

fn validate_primary_data(data: &Value, pointer: &str) -> Check {
    match data {
        Value::Null => Ok(()),
        Value::Object(_) => validate_resource(data, pointer),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_resource(item, &format!("{pointer}/{index}"))?;
            }
            Ok(())
        }
        _ => fail(
            pointer,
            "primary data must be a resource object, an array of them, or null",
        ),
    }
}

fn validate_resource(value: &Value, pointer: &str) -> Check {
    let map = object(value, pointer)?;
    forbid_foreign(map, pointer, RESOURCE_MEMBERS)?;
    require_string(map, pointer, "type")?;
    require_string(map, pointer, "id")?;

    let attributes = match map.get("attributes") {
        Some(value) => Some(object(value, &format!("{pointer}/attributes"))?),
        None => None,
    };
    let relationships = match map.get("relationships") {
        Some(value) => Some(object(value, &format!("{pointer}/relationships"))?),
        None => None,
    };

    // Fields share a namespace with each other and with `type`/`id`.
    for fields in [attributes, relationships].into_iter().flatten() {
        for name in fields.keys() {
            if name == "type" || name == "id" {
                return fail(pointer, format!("field `{name}` collides with `type`/`id`"));
            }
        }
    }
    if let (Some(attributes), Some(relationships)) = (attributes, relationships) {
        for name in relationships.keys() {
            if attributes.contains_key(name) {
                return fail(
                    pointer,
                    format!("`{name}` is both an attribute and a relationship"),
                );
            }
        }
    }
    if let Some(relationships) = relationships {
        for (name, relationship) in relationships {
            validate_relationship(relationship, &format!("{pointer}/relationships/{name}"))?;
        }
    }
    if let Some(links) = map.get("links") {
        validate_links(links, &format!("{pointer}/links"))?;
    }
    object_if_present(map, pointer, "meta")
}

fn validate_relationship(value: &Value, pointer: &str) -> Check {
    let map = object(value, pointer)?;
    forbid_foreign(map, pointer, RELATIONSHIP_MEMBERS)?;
    if !["links", "data", "meta"]
        .iter()
        .any(|m| map.contains_key(*m))
    {
        return fail(
            pointer,
            "relationship object must contain at least one of `links`, `data`, `meta`",
        );
    }
    if let Some(data) = map.get("data") {
        validate_linkage(data, &format!("{pointer}/data"))?;
    }
    if let Some(links) = map.get("links") {
        validate_links(links, &format!("{pointer}/links"))?;
    }
    object_if_present(map, pointer, "meta")
}

fn validate_linkage(data: &Value, pointer: &str) -> Check {
    match data {
        Value::Null => Ok(()),
        Value::Object(_) => validate_identifier(data, pointer),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_identifier(item, &format!("{pointer}/{index}"))?;
            }
            Ok(())
        }
        _ => fail(
            pointer,
            "resource linkage must be null, an identifier object, or an array of them",
        ),
    }
}

fn validate_identifier(value: &Value, pointer: &str) -> Check {
    let map = object(value, pointer)?;
    forbid_foreign(map, pointer, IDENTIFIER_MEMBERS)?;
    require_string(map, pointer, "type")?;
    if !map.contains_key("id") && !map.contains_key("lid") {
        return fail(pointer, "identifier object must contain `id` (or `lid`)");
    }
    string_if_present(map, pointer, "id")?;
    string_if_present(map, pointer, "lid")?;
    object_if_present(map, pointer, "meta")
}

fn validate_links(value: &Value, pointer: &str) -> Check {
    let map = object(value, pointer)?;
    for (name, link) in map {
        let link_pointer = format!("{pointer}/{name}");
        match link {
            Value::Null | Value::String(_) => {}
            Value::Object(_) => {
                let link = object(link, &link_pointer)?;
                forbid_foreign(link, &link_pointer, LINK_OBJECT_MEMBERS)?;
                require_string(link, &link_pointer, "href")?;
            }
            _ => {
                return fail(
                    &link_pointer,
                    "a link must be a string, a link object, or null",
                );
            }
        }
    }
    Ok(())
}

fn validate_errors(value: &Value, pointer: &str) -> Check {
    for (index, error) in array(value, pointer)?.iter().enumerate() {
        let error_pointer = format!("{pointer}/{index}");
        let map = object(error, &error_pointer)?;
        forbid_foreign(map, &error_pointer, ERROR_MEMBERS)?;
        if map.is_empty() {
            return fail(
                &error_pointer,
                "error object must contain at least one member",
            );
        }
        for member in ["status", "code", "title", "detail"] {
            string_if_present(map, &error_pointer, member)?;
        }
        if let Some(source) = map.get("source") {
            let source_pointer = format!("{error_pointer}/source");
            forbid_foreign(
                object(source, &source_pointer)?,
                &source_pointer,
                SOURCE_MEMBERS,
            )?;
        }
        if let Some(links) = map.get("links") {
            validate_links(links, &format!("{error_pointer}/links"))?;
        }
        object_if_present(map, &error_pointer, "meta")?;
    }
    Ok(())
}

fn validate_jsonapi(value: &Value, pointer: &str) -> Check {
    let map = object(value, pointer)?;
    forbid_foreign(map, pointer, JSONAPI_MEMBERS)?;
    string_if_present(map, pointer, "version")?;
    object_if_present(map, pointer, "meta")
}

/// A compound document must not carry more than one resource object per
/// `type`+`id`, across primary data and `included`.
fn validate_unique_resources(document: &Value) -> Check {
    let mut seen = HashSet::new();
    for (resource, pointer) in resources(document) {
        if let (Some(kind), Some(id)) = (
            resource.get("type").and_then(Value::as_str),
            resource.get("id").and_then(Value::as_str),
        ) && !seen.insert((kind, id))
        {
            return fail(
                &pointer,
                format!(
                    "duplicate resource `{kind}:{id}`; a document must not carry more than one object per type+id"
                ),
            );
        }
    }
    Ok(())
}

/// Every `included` resource must be reachable from primary data through a chain
/// of relationship linkage ("full linkage"). `None` ⇒ fully linked (or no
/// `included`). Callers must avoid documents where a sparse fieldset hides the
/// linking relationship — the spec's stated exception.
pub fn validate_full_linkage(document: &Value) -> Option<ValidationError> {
    let included = match document.get("included").and_then(Value::as_array) {
        Some(included) if !included.is_empty() => included,
        _ => return None,
    };

    let mut by_identity = HashMap::new();
    for resource in included {
        if let Some(identity) = identifier_of(resource) {
            by_identity.insert(identity, resource);
        }
    }

    let mut reachable = HashSet::new();
    let mut frontier: Vec<(String, String)> = primary_data_resources(document)
        .iter()
        .flat_map(|resource| linkage_targets(resource))
        .collect();
    while let Some(identity) = frontier.pop() {
        if let Some(resource) = by_identity.get(&identity)
            && reachable.insert(identity.clone())
        {
            frontier.extend(linkage_targets(resource));
        }
    }

    for resource in included {
        if let Some(identity) = identifier_of(resource)
            && !reachable.contains(&identity)
        {
            let (kind, id) = identity;
            return Some(ValidationError {
                pointer: "/included".to_owned(),
                reason: format!(
                    "included resource `{kind}:{id}` is not reachable from primary data"
                ),
            });
        }
    }
    None
}

fn primary_data_resources(document: &Value) -> Vec<&Value> {
    match document.get("data") {
        Some(data @ Value::Object(_)) => vec![data],
        Some(Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

fn identifier_of(value: &Value) -> Option<(String, String)> {
    let kind = value.get("type").and_then(Value::as_str)?;
    let id = value.get("id").and_then(Value::as_str)?;
    Some((kind.to_owned(), id.to_owned()))
}

fn linkage_targets(resource: &Value) -> Vec<(String, String)> {
    let mut targets = Vec::new();
    if let Some(relationships) = resource.get("relationships").and_then(Value::as_object) {
        for relationship in relationships.values() {
            match relationship.get("data") {
                Some(one @ Value::Object(_)) => targets.extend(identifier_of(one)),
                Some(Value::Array(many)) => {
                    for identifier in many {
                        targets.extend(identifier_of(identifier));
                    }
                }
                _ => {}
            }
        }
    }
    targets
}

// --- Application URL set --------------------------------------------------
//
// The arbitrary URL set this test application exposes — as much our choice as
// the schema set. Tests aim requests at these URLs; `validate_urls` checks that
// any link a response carries matches the URL its `type`+`id` imply.

pub const BASE_URL: &str = "https://api.example.test";

pub fn collection_url(kind: &str) -> String {
    format!("{BASE_URL}/{kind}")
}

pub fn resource_url(kind: &str, id: &str) -> String {
    format!("{BASE_URL}/{kind}/{id}")
}

pub fn relationship_url(kind: &str, id: &str, name: &str) -> String {
    format!("{BASE_URL}/{kind}/{id}/relationships/{name}")
}

pub fn related_url(kind: &str, id: &str, name: &str) -> String {
    format!("{BASE_URL}/{kind}/{id}/{name}")
}

/// Validates that every link a resource (in `data` or `included`) or its
/// relationships carries matches the URL our URL set assigns from its
/// `type`+`id`. Presence is never required — only present links are checked.
/// Top-level `self` depends on the request target, so it is asserted at the
/// test site.
pub fn validate_urls(document: &Value) -> Option<ValidationError> {
    validate_resource_urls_in(document).err()
}

fn validate_resource_urls_in(document: &Value) -> Check {
    for (resource, pointer) in resources(document) {
        validate_resource_urls(resource, &pointer)?;
    }
    Ok(())
}

fn resources(document: &Value) -> Vec<(&Value, String)> {
    let mut collected = Vec::new();
    match document.get("data") {
        Some(data @ Value::Object(_)) => collected.push((data, "/data".to_owned())),
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                collected.push((item, format!("/data/{index}")));
            }
        }
        _ => {}
    }
    if let Some(Value::Array(items)) = document.get("included") {
        for (index, item) in items.iter().enumerate() {
            collected.push((item, format!("/included/{index}")));
        }
    }
    collected
}

fn validate_resource_urls(resource: &Value, pointer: &str) -> Check {
    let (Some(kind), Some(id)) = (
        resource.get("type").and_then(Value::as_str),
        resource.get("id").and_then(Value::as_str),
    ) else {
        return Ok(());
    };

    if let Some(url) = link(resource, "self") {
        expect_url(
            &format!("{pointer}/links/self"),
            &resource_url(kind, id),
            url,
        )?;
    }
    if let Some(relationships) = resource.get("relationships").and_then(Value::as_object) {
        for (name, relationship) in relationships {
            let base = format!("{pointer}/relationships/{name}");
            if let Some(url) = link(relationship, "self") {
                expect_url(
                    &format!("{base}/links/self"),
                    &relationship_url(kind, id, name),
                    url,
                )?;
            }
            if let Some(url) = link(relationship, "related") {
                expect_url(
                    &format!("{base}/links/related"),
                    &related_url(kind, id, name),
                    url,
                )?;
            }
        }
    }
    Ok(())
}

fn link<'a>(container: &'a Value, name: &str) -> Option<&'a str> {
    container.get("links")?.get(name)?.as_str()
}

fn expect_url(pointer: &str, expected: &str, found: &str) -> Check {
    if found == expected {
        Ok(())
    } else {
        fail(
            pointer,
            format!("expected URL `{expected}`, found `{found}`"),
        )
    }
}
