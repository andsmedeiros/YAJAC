//! JSON Pointers into a document, used to name the origin of an error.
//!
//! The standard requires a pointer to address a value that actually exists in the request document
//! (spec §"Error Objects"), so these are built only where the caller holds the document the pointer
//! addresses. Every path segment is escaped per RFC 6901, keeping a member name containing `/` or
//! `~` from silently addressing somewhere else.

use crate::json_api::error::Source;
use std::fmt::Display;

/// Escapes a single reference token per RFC 6901: `~` becomes `~0` and `/` becomes `~1`. Tilde is
/// escaped first, so the tilde introduced by an escaped solidus is left alone.
fn escape_token(token: impl Display) -> String {
    token.to_string().replace('~', "~0").replace('/', "~1")
}

/// Points at the document's primary data as a whole.
pub fn for_primary_data() -> Source {
    Source::Pointer("/data".to_string())
}

/// Points at a member of the primary resource object itself, such as `type` or `id`.
pub fn for_member(member: impl Display) -> Source {
    Source::Pointer(format!("/data/{}", escape_token(member)))
}

/// Points at a named attribute of the primary resource object.
pub fn for_attribute(attribute: impl Display) -> Source {
    Source::Pointer(format!("/data/attributes/{}", escape_token(attribute)))
}

/// Points at a named relationship of the primary resource object.
pub fn for_relationship(relationship: impl Display) -> Source {
    Source::Pointer(format!(
        "/data/relationships/{}",
        escape_token(relationship)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_data_points_at_the_document_root() {
        assert_eq!(for_primary_data(), Source::Pointer("/data".to_string()));
    }

    #[test]
    fn a_member_points_beside_the_primary_data() {
        assert_eq!(
            for_member("type"),
            Source::Pointer("/data/type".to_string())
        );
    }

    #[test]
    fn an_attribute_points_under_attributes() {
        assert_eq!(
            for_attribute("title"),
            Source::Pointer("/data/attributes/title".to_string())
        );
    }

    #[test]
    fn a_relationship_points_under_relationships() {
        assert_eq!(
            for_relationship("author"),
            Source::Pointer("/data/relationships/author".to_string())
        );
    }

    #[test]
    fn a_solidus_in_a_member_name_is_escaped() {
        assert_eq!(
            for_attribute("published/at"),
            Source::Pointer("/data/attributes/published~1at".to_string())
        );
    }

    #[test]
    fn a_tilde_in_a_member_name_is_escaped() {
        assert_eq!(
            for_attribute("published~at"),
            Source::Pointer("/data/attributes/published~0at".to_string())
        );
    }

    /// Tilde must be escaped before solidus, or the tilde of an escaped solidus is escaped in turn
    /// and the pointer addresses a different location.
    #[test]
    fn an_escaped_solidus_is_not_escaped_again() {
        assert_eq!(
            for_attribute("a~1b/c"),
            Source::Pointer("/data/attributes/a~01b~1c".to_string())
        );
    }
}
