//! The harness's own contract: the schema set is internally consistent, and the tables behind it
//! agree with it column for column. Both are invariants every other suite silently relies on, so a
//! breach must fail here rather than as a hundred unrelated failures elsewhere.

use super::{Result, database, schemas};
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

    for builder in schemas::all() {
        let name = builder.into_parts().name;
        let schema = manager.registry().schema(name)?;
        let rows = manager
            .table(name, &connection)?
            .query(&QueryParameters::new(schema))?;

        assert!(rows.is_empty(), "{name} starts empty");
    }

    Ok(())
}
