//! The tables behind [`super::schemas`], and a connection manager bound to them.
//!
//! Their foreign keys divide as:
//!
//! - **nullable** — `articles.author_id`, `articles.editor_id`, `comments.author_id` and
//!   `profiles.author_handle`, which may therefore be detached
//! - **`NOT NULL`** — `comments.article_id` and `summaries.article_id`
//! - **`UNIQUE`** — `profiles.author_handle` and `summaries.article_id`, the has-one joins
//!
//! `authors.handle` carries a `UNIQUE` index, which is what `profiles.author_handle` references.

use super::{Result, schemas};
use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Pool;
use crate::database::attributes::Row;
use crate::database::connection_manager;
use crate::database::query_parameters::QueryParameters;
use crate::database::table::Table;

/// The crate's connection manager, bound to the adapter every suite runs against, so a signature
/// naming one carries only its lifetime.
pub(crate) type ConnectionManager<'sch> =
    connection_manager::ConnectionManager<'sch, SqliteAdapter>;

/// The tables the schema set describes, plus the full-text shadow and triggers that back the text
/// index `articles` declares.
const DDL: &str = "
    CREATE TABLE authors (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        handle TEXT NOT NULL UNIQUE,
        age INTEGER,
        rating REAL,
        active BOOLEAN,
        joined_at TEXT
    );

    CREATE TABLE publishers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL
    );

    CREATE TABLE articles (
        id INTEGER PRIMARY KEY,
        author_id INTEGER REFERENCES authors(id),
        editor_id INTEGER REFERENCES authors(id),
        publisher_id TEXT REFERENCES publishers(id),
        title TEXT NOT NULL,
        body TEXT,
        published BOOLEAN,
        views INTEGER
    );

    CREATE TABLE comments (
        id INTEGER PRIMARY KEY,
        article_id INTEGER NOT NULL REFERENCES articles(id),
        author_id INTEGER REFERENCES authors(id),
        parent_id INTEGER REFERENCES comments(id),
        content TEXT NOT NULL
    );

    CREATE TABLE profiles (
        id INTEGER PRIMARY KEY,
        author_handle TEXT UNIQUE REFERENCES authors(handle),
        bio TEXT
    );

    CREATE TABLE summaries (
        id INTEGER PRIMARY KEY,
        article_id INTEGER NOT NULL UNIQUE REFERENCES articles(id),
        synopsis TEXT NOT NULL
    );

    CREATE VIRTUAL TABLE articles_fts USING fts5(title, body, tokenize='trigram');

    CREATE TRIGGER articles_fts_insert AFTER INSERT ON articles BEGIN
        INSERT INTO articles_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
    END;

    CREATE TRIGGER articles_fts_update AFTER UPDATE ON articles BEGIN
        UPDATE articles_fts SET title = new.title, body = new.body WHERE rowid = new.id;
    END;

    CREATE TRIGGER articles_fts_delete AFTER DELETE ON articles BEGIN
        DELETE FROM articles_fts WHERE rowid = old.id;
    END;
";

/// A registry bound to a fresh in-memory database with the tables created and `seed` written into
/// them, each row paired with the table it belongs to and inserted in the order given.
///
/// Seeding is part of construction because the pool holds a single connection: the manager is handed
/// over only once every row is in place and that connection is back in the pool.
pub(crate) fn build_database<'a>(
    seed: impl IntoIterator<Item = (&'a str, Row<'static>)>,
) -> Result<ConnectionManager<'static>> {
    let connection_manager = ConnectionManager::new(schemas::build_registry()?, Pool::memory()?);
    let connection = connection_manager.acquire()?;
    connection.execute_batch(DDL)?;

    for (table, row) in seed {
        connection_manager
            .table(table, &connection)
            .and_then(|table| table.insert(row, &QueryParameters::new(table.schema())))?;
    }

    Ok(connection_manager)
}
