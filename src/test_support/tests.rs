//! The harness's own contract: the schema set is internally consistent, the tables behind it agree
//! with it column for column, and the recurring cast lands in those tables intact. All three are
//! invariants every other suite silently relies on, so a breach must fail here rather than as a
//! hundred unrelated failures elsewhere.

use super::{Result, database, fixtures, schemas};
use crate::database::attributes::Attribute::*;
use crate::database::query_parameters::QueryParameters;
use crate::database::table::Table;
use test_log::test;

/// Building the manager validates the whole set — `Registry::try_new` refuses an inconsistent one —
/// and then queries every table through the framework. The generated `SELECT` names each schema
/// column, so a column the DDL lacks, misspells or types differently fails the query outright.
#[test]
fn every_schema_matches_its_table() -> Result {
    let manager = database::build_connection_manager()?;
    let connection = manager.acquire()?;

    for name in schemas::TABLES {
        let schema = manager.registry().schema(name)?;
        let rows = manager
            .table(name, &connection)?
            .query(&QueryParameters::new(schema))?;

        assert!(rows.is_empty(), "{name} starts empty");
    }

    Ok(())
}

/// Inserting the cast in foreign-key order proves the references hold, and reading the rows back
/// proves each column stores and materialises as the type its schema declares — a text primary key
/// as text, a nullable foreign key as null when unset, and the whole attribute range on `ann`.
#[test]
fn the_recurring_cast_lands_intact() -> Result {
    let manager = database::build_connection_manager()?;
    let connection = manager.acquire()?;

    let ann = fixtures::authors::ann(&manager, &connection)?;
    fixtures::authors::bob(&manager, &connection)?;
    let acme = fixtures::publishers::acme(&manager, &connection)?;
    let first = fixtures::articles::first(&manager, &connection)?;
    let second = fixtures::articles::second(&manager, &connection)?;
    let unattributed = fixtures::articles::unattributed(&manager, &connection)?;
    fixtures::comments::praise(&manager, &connection)?;
    let reply = fixtures::comments::reply(&manager, &connection)?;
    let profile = fixtures::profiles::anns(&manager, &connection)?;
    let summary = fixtures::summaries::firsts(&manager, &connection)?;

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

    // The self-reference, and the two joins that do not run through a primary key.
    assert_eq!(reply["parent_id"], Integer(1));
    assert_eq!(profile["author_handle"], ann["handle"]);
    assert_eq!(summary["article_id"], first["id"]);

    Ok(())
}
