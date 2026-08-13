//! The harness's own contract: the schema set is internally consistent, the tables behind it agree
//! with it column for column, and the recurring cast lands in those tables intact.

use super::{Result, database, fixtures, schemas};
use crate::database::attributes::Attribute::*;
use crate::database::attributes::Identifier;
use crate::database::query_parameters::QueryParameters;
use crate::database::table::Table;
use test_log::test;

/// Building the database validates the whole set — `Registry::try_new` refuses an inconsistent one —
/// and then queries every table through the framework. The generated `SELECT` names each schema
/// column, so a column the DDL lacks, misspells or types differently fails the query outright.
#[test]
fn every_schema_matches_its_table() -> Result {
    let manager = database::build_database([])?;
    let connection = manager.acquire()?;

    for name in schemas::TABLES {
        let table = manager.table(name, &connection)?;
        let rows = table.query(&QueryParameters::new(table.schema()))?;

        assert!(rows.is_empty(), "{name} starts empty");
    }

    Ok(())
}

/// Seeding the cast proves its foreign keys resolve in the order given, and reading the rows back
/// proves each column stores and materialises as the type its schema declares — a text primary key
/// as text, an unset foreign key as null, and the whole attribute range on `ann`.
#[test]
fn the_recurring_cast_lands_intact() -> Result {
    let manager = database::build_database([
        ("authors", fixtures::authors::ann()?),
        ("authors", fixtures::authors::bob()?),
        ("publishers", fixtures::publishers::acme()?),
        ("articles", fixtures::articles::first()?),
        ("articles", fixtures::articles::second()?),
        ("articles", fixtures::articles::unattributed()?),
        ("comments", fixtures::comments::praise()?),
        ("comments", fixtures::comments::reply()?),
        ("profiles", fixtures::profiles::anns()?),
        ("summaries", fixtures::summaries::firsts()?),
    ])?;
    let connection = manager.acquire()?;

    let authors = manager.table("authors", &connection)?;
    let articles = manager.table("articles", &connection)?;
    let publishers = manager.table("publishers", &connection)?;
    let comments = manager.table("comments", &connection)?;
    let profiles = manager.table("profiles", &connection)?;

    let ann = authors.find(
        Identifier::Integer(1),
        &QueryParameters::new(authors.schema()),
    )?;
    let acme = publishers.find(
        Identifier::Text("acme-press".to_string()),
        &QueryParameters::new(publishers.schema()),
    )?;
    let first = articles.find(
        Identifier::Integer(1),
        &QueryParameters::new(articles.schema()),
    )?;
    let second = articles.find(
        Identifier::Integer(2),
        &QueryParameters::new(articles.schema()),
    )?;
    let unattributed = articles.find(
        Identifier::Integer(3),
        &QueryParameters::new(articles.schema()),
    )?;
    let reply = comments.find(
        Identifier::Integer(2),
        &QueryParameters::new(comments.schema()),
    )?;
    let anns_profile = profiles.find(
        Identifier::Integer(1),
        &QueryParameters::new(profiles.schema()),
    )?;

    // Every attribute type survives the round trip.
    assert_eq!(ann["name"], Text("Ann Sorensen".to_string()));
    assert_eq!(ann["age"], Integer(41));
    assert_eq!(ann["rating"], Float(4.5));
    assert_eq!(ann["active"], Boolean(true));
    assert_eq!(ann["joined_at"], DateTime("2018-03-14T09:00:00Z".parse()?));

    // A text primary key stays text, and so does the foreign key referencing it.
    assert_eq!(acme["id"], Text("acme-press".to_string()));
    assert_eq!(first["publisher_id"], Text("acme-press".to_string()));

    // What is attached is attached, and what is not reads as null rather than absent.
    assert_eq!(first["editor_id"], Integer(2));
    assert_eq!(second["editor_id"], Null);
    assert_eq!(unattributed["author_id"], Null);
    assert_eq!(unattributed["editor_id"], Null);
    assert_eq!(unattributed["publisher_id"], Null);

    // The self-reference, and the join that does not run through a primary key.
    assert_eq!(reply["parent_id"], Integer(1));
    assert_eq!(anns_profile["author_handle"], ann["handle"]);

    Ok(())
}
