//! Named rows, grouped by the table they belong to.
//!
//! Each is the data alone, as declared. Ids are explicit and stable, so one fixture references
//! another by a known value, and a fixture referencing another requires it in the database first.

use super::Result;
use crate::database::attributes::{Attribute, Row};
use Attribute::*;

/// Two authors. `ann` is active, older and higher rated; `bob` is inactive, younger and lower
/// rated, so an ordering by any of those columns differs from insertion order.
pub(crate) mod authors {
    use super::*;

    pub(crate) fn ann() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(1)),
            ("name", Text("Ann Sorensen".to_string())),
            ("handle", Text("ann".to_string())),
            ("age", Integer(41)),
            ("rating", Float(4.5)),
            ("active", Boolean(true)),
            ("joined_at", DateTime("2018-03-14T09:00:00Z".parse()?)),
        ]))
    }

    pub(crate) fn bob() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(2)),
            ("name", Text("Bob Ferreira".to_string())),
            ("handle", Text("bob".to_string())),
            ("age", Integer(29)),
            ("rating", Float(3.0)),
            ("active", Boolean(false)),
            ("joined_at", DateTime("2021-11-02T17:30:00Z".parse()?)),
        ]))
    }
}

pub(crate) mod publishers {
    use super::*;

    pub(crate) fn acme() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Text("acme-press".to_string())),
            ("name", Text("Acme Press".to_string())),
        ]))
    }
}

/// Three articles. `first` carries an author, an editor and a publisher; `second` shares `first`'s
/// author and publisher and has no editor; `unattributed` carries none of the three.
pub(crate) mod articles {
    use super::*;

    pub(crate) fn first() -> Result<Row<'static>> {
        Ok(Row::from_iter([
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
        ]))
    }

    pub(crate) fn second() -> Result<Row<'static>> {
        Ok(Row::from_iter([
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
        ]))
    }

    pub(crate) fn unattributed() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(3)),
            ("author_id", Null),
            ("editor_id", Null),
            ("publisher_id", Null),
            ("title", Text("Notes Found in a Drawer".to_string())),
            ("body", Text("Provenance unknown.".to_string())),
            ("published", Boolean(false)),
            ("views", Integer(3)),
        ]))
    }
}

/// A two-deep thread on `articles::first`: `praise` is top-level, and `reply` answers it.
pub(crate) mod comments {
    use super::*;

    pub(crate) fn praise() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(1)),
            ("article_id", Integer(1)),
            ("author_id", Integer(2)),
            ("parent_id", Null),
            (
                "content",
                Text("This clarified something I had been circling for months.".to_string()),
            ),
        ]))
    }

    pub(crate) fn reply() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(2)),
            ("article_id", Integer(1)),
            ("author_id", Integer(1)),
            ("parent_id", Integer(1)),
            (
                "content",
                Text("Glad it landed -- the second half was the hard part to write.".to_string()),
            ),
        ]))
    }
}

/// Ann's profile, joined to her by `handle` rather than by primary key.
pub(crate) mod profiles {
    use super::*;

    pub(crate) fn anns() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(1)),
            ("author_handle", Text("ann".to_string())),
            (
                "bio",
                Text("Writes about systems, mostly the parts that leak.".to_string()),
            ),
        ]))
    }
}

pub(crate) mod summaries {
    use super::*;

    pub(crate) fn firsts() -> Result<Row<'static>> {
        Ok(Row::from_iter([
            ("id", Integer(1)),
            ("article_id", Integer(1)),
            (
                "synopsis",
                Text("Lifetimes as provenance, not storage.".to_string()),
            ),
        ]))
    }
}
