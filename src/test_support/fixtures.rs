//! Named rows, grouped by the table they belong to.
//!
//! Each writes through the framework's insert path and yields the persisted row. Ids are explicit
//! and stable, so one fixture references another by a known value, and a fixture referencing another
//! requires it to be inserted first.

use super::Result;
use crate::database::adapters::SqliteAdapter;
use crate::database::adapters::sqlite::Connection;
use crate::database::attributes::{Attribute, Row};
use crate::database::connection_manager::ConnectionManager;
use crate::database::query_parameters::QueryParameters;
use crate::database::table::Table;
use Attribute::*;

type Manager<'sch> = ConnectionManager<'sch, SqliteAdapter>;

/// Two authors. `ann` is active, older and higher rated; `bob` is inactive, younger and lower
/// rated, so an ordering by any of those columns differs from insertion order.
pub(crate) mod authors {
    use super::*;

    pub(crate) fn ann<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("authors", connection)?;
        let row = Row::from_iter([
            ("id", Integer(1)),
            ("name", Text("Ann Sorensen".to_string())),
            ("handle", Text("ann".to_string())),
            ("age", Integer(41)),
            ("rating", Float(4.5)),
            ("active", Boolean(true)),
            ("joined_at", DateTime("2018-03-14T09:00:00Z".parse()?)),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }

    pub(crate) fn bob<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("authors", connection)?;
        let row = Row::from_iter([
            ("id", Integer(2)),
            ("name", Text("Bob Ferreira".to_string())),
            ("handle", Text("bob".to_string())),
            ("age", Integer(29)),
            ("rating", Float(3.0)),
            ("active", Boolean(false)),
            ("joined_at", DateTime("2021-11-02T17:30:00Z".parse()?)),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }
}

pub(crate) mod publishers {
    use super::*;

    pub(crate) fn acme<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("publishers", connection)?;
        let row = Row::from_iter([
            ("id", Text("acme-press".to_string())),
            ("name", Text("Acme Press".to_string())),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }
}

/// Three articles. `first` carries an author, an editor and a publisher; `second` shares `first`'s
/// author and publisher and has no editor; `unattributed` carries none of the three.
pub(crate) mod articles {
    use super::*;

    pub(crate) fn first<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("articles", connection)?;
        let row = Row::from_iter([
            ("id", Integer(1)),
            ("author_id", Integer(1)),
            ("editor_id", Integer(2)),
            ("publisher_id", Text("acme-press".to_string())),
            ("title", Text("On Borrowed Lifetimes".to_string())),
            (
                "body",
                Text("A study of provenance in layered systems.".to_string()),
            ),
            ("published", Boolean(true)),
            ("views", Integer(1_204)),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }

    pub(crate) fn second<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("articles", connection)?;
        let row = Row::from_iter([
            ("id", Integer(2)),
            ("author_id", Integer(1)),
            ("editor_id", Null),
            ("publisher_id", Text("acme-press".to_string())),
            ("title", Text("The Cost of a Clone".to_string())),
            (
                "body",
                Text("Where copying quietly becomes the bottleneck.".to_string()),
            ),
            ("published", Boolean(false)),
            ("views", Integer(87)),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }

    pub(crate) fn unattributed<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("articles", connection)?;
        let row = Row::from_iter([
            ("id", Integer(3)),
            ("author_id", Null),
            ("editor_id", Null),
            ("publisher_id", Null),
            ("title", Text("Notes Found in a Drawer".to_string())),
            ("body", Text("Provenance unknown.".to_string())),
            ("published", Boolean(false)),
            ("views", Integer(3)),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }
}

/// A two-deep thread on `articles::first`: `praise` is top-level, and `reply` answers it.
pub(crate) mod comments {
    use super::*;

    pub(crate) fn praise<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("comments", connection)?;
        let row = Row::from_iter([
            ("id", Integer(1)),
            ("article_id", Integer(1)),
            ("author_id", Integer(2)),
            ("parent_id", Null),
            (
                "content",
                Text("This clarified something I had been circling for months.".to_string()),
            ),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }

    pub(crate) fn reply<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("comments", connection)?;
        let row = Row::from_iter([
            ("id", Integer(2)),
            ("article_id", Integer(1)),
            ("author_id", Integer(1)),
            ("parent_id", Integer(1)),
            (
                "content",
                Text("Glad it landed -- the second half was the hard part to write.".to_string()),
            ),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }
}

/// Ann's profile, joined to her by `handle` rather than by primary key.
pub(crate) mod profiles {
    use super::*;

    pub(crate) fn anns<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("profiles", connection)?;
        let row = Row::from_iter([
            ("id", Integer(1)),
            ("author_handle", Text("ann".to_string())),
            (
                "bio",
                Text("Writes about systems, mostly the parts that leak.".to_string()),
            ),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }
}

pub(crate) mod summaries {
    use super::*;

    pub(crate) fn firsts<'sch>(
        manager: &'sch Manager<'sch>,
        connection: &Connection,
    ) -> Result<Row<'sch>> {
        let table = manager.table("summaries", connection)?;
        let row = Row::from_iter([
            ("id", Integer(1)),
            ("article_id", Integer(1)),
            (
                "synopsis",
                Text("Lifetimes as provenance, not storage.".to_string()),
            ),
        ]);

        table
            .insert(row, &QueryParameters::new(table.schema()))
            .map_err(Into::into)
    }
}
