//! The shared schema set: a publishing domain of authors, publishers, articles, comments, profiles
//! and summaries. Authors write articles for publishers, readers comment on them, and an article may
//! carry a summary.
//!
//! Across its six resources it carries:
//!
//! - **two `belongs_to` onto one table** — `articles.author` and `articles.editor`
//! - **a self-reference** — `comments.parent` / `comments.replies`
//! - **a join on a non-primary-key text column, from both sides** — `authors.handle` ↔
//!   `profiles.author_handle`
//! - **a text primary key with no server-side default** — `publishers`
//! - **every attribute type** — `authors` carries text, integer, float, boolean and date-time
//! - **a text index** — on `articles`
//!
//! Column nullability, which decides what may be detached, lives with the tables in
//! [`super::database`].

use super::Result;
use crate::database::registry::Registry;
use crate::database::schema::{AttributeType, IdentifierType, Related, SchemaBuilder};

fn authors() -> SchemaBuilder<'static> {
    SchemaBuilder::table("authors")
        .attribute("name", AttributeType::Text)
        .attribute("handle", AttributeType::Text)
        .attribute("age", AttributeType::Integer)
        .attribute("rating", AttributeType::Float)
        .attribute("active", AttributeType::Boolean)
        .attribute("joined_at", AttributeType::DateTime)
        .has_many(
            "articles",
            Related::to("articles")
                .pointing_related("author_id")
                .to_own("id"),
        )
        .has_many(
            "edited",
            Related::to("articles")
                .pointing_related("editor_id")
                .to_own("id"),
        )
        .has_many(
            "comments",
            Related::to("comments")
                .pointing_related("author_id")
                .to_own("id"),
        )
        // Keyed on `handle`, not the primary key, and on a text column at that.
        .has_one(
            "profile",
            Related::to("profiles")
                .pointing_related("author_handle")
                .to_own("handle"),
        )
}

fn articles() -> SchemaBuilder<'static> {
    SchemaBuilder::table("articles")
        .attribute("title", AttributeType::Text)
        .attribute("body", AttributeType::Text)
        .attribute("published", AttributeType::Boolean)
        .attribute("views", AttributeType::Integer)
        .foreign_key("author_id", AttributeType::Integer)
        .foreign_key("editor_id", AttributeType::Integer)
        .foreign_key("publisher_id", AttributeType::Text)
        .belongs_to(
            "author",
            Related::to("authors")
                .pointing_own("author_id")
                .to_related("id"),
        )
        .belongs_to(
            "editor",
            Related::to("authors")
                .pointing_own("editor_id")
                .to_related("id"),
        )
        .belongs_to(
            "publisher",
            Related::to("publishers")
                .pointing_own("publisher_id")
                .to_related("id"),
        )
        .has_many(
            "comments",
            Related::to("comments")
                .pointing_related("article_id")
                .to_own("id"),
        )
        .has_one(
            "summary",
            Related::to("summaries")
                .pointing_related("article_id")
                .to_own("id"),
        )
        .text_index()
}

fn comments() -> SchemaBuilder<'static> {
    SchemaBuilder::table("comments")
        .attribute("content", AttributeType::Text)
        .foreign_key("article_id", AttributeType::Integer)
        .foreign_key("author_id", AttributeType::Integer)
        .foreign_key("parent_id", AttributeType::Integer)
        .belongs_to(
            "article",
            Related::to("articles")
                .pointing_own("article_id")
                .to_related("id"),
        )
        .belongs_to(
            "author",
            Related::to("authors")
                .pointing_own("author_id")
                .to_related("id"),
        )
        .belongs_to(
            "parent",
            Related::to("comments")
                .pointing_own("parent_id")
                .to_related("id"),
        )
        .has_many(
            "replies",
            Related::to("comments")
                .pointing_related("parent_id")
                .to_own("id"),
        )
}

fn profiles() -> SchemaBuilder<'static> {
    SchemaBuilder::table("profiles")
        .attribute("bio", AttributeType::Text)
        .foreign_key("author_handle", AttributeType::Text)
        .belongs_to(
            "author",
            Related::to("authors")
                .pointing_own("author_handle")
                .to_related("handle"),
        )
}

fn summaries() -> SchemaBuilder<'static> {
    SchemaBuilder::table("summaries")
        .attribute("synopsis", AttributeType::Text)
        .foreign_key("article_id", AttributeType::Integer)
        .belongs_to(
            "article",
            Related::to("articles")
                .pointing_own("article_id")
                .to_related("id"),
        )
}

fn publishers() -> SchemaBuilder<'static> {
    SchemaBuilder::table("publishers")
        .primary_key("id", IdentifierType::Text)
        .attribute("name", AttributeType::Text)
        .has_many(
            "articles",
            Related::to("articles")
                .pointing_related("publisher_id")
                .to_own("id"),
        )
}

/// Every resource the set declares, in foreign-key order: referencing a name from here is what a
/// test does instead of asking a builder what it was called.
pub(crate) const TABLES: [&str; 6] = [
    "authors",
    "publishers",
    "articles",
    "comments",
    "profiles",
    "summaries",
];

/// The whole set, in the order a reader meets it: the writers, what they write, and the satellites.
pub(crate) fn all() -> [SchemaBuilder<'static>; 6] {
    [
        authors(),
        publishers(),
        articles(),
        comments(),
        profiles(),
        summaries(),
    ]
}

/// The schema stage: a validated registry over the whole set, with no storage behind it. Suites
/// whose subject is a schema, a query string or generated SQL stop here and never open a database.
pub(crate) fn build_registry() -> Result<Registry<'static>> {
    let schemas = all();

    Registry::try_new(schemas).map_err(Into::into)
}
