use super::{
    connection::Connection as ConnectionInterface, pool::Pool as PoolInterface,
    query_builder::QueryBuilder as QueryBuilderInterface, table::Table as TableInterface,
};

#[cfg(feature = "sqlite")]
pub mod sqlite;

pub trait Adapter {
    type Connection: ConnectionInterface;
    type Pool: PoolInterface<Connection = Self::Connection>;
    type QueryBuilder<'sch>: QueryBuilderInterface<'sch>;
    // type Migrator: MigratorInterface;
    type Table<'sch, 'req>: TableInterface<'sch, 'req, Self::Connection, Self::QueryBuilder<'sch>>
    where
        Self::Connection: 'req;
}

#[cfg(feature = "sqlite")]
pub struct SqliteAdapter;

#[cfg(feature = "sqlite")]
impl Adapter for SqliteAdapter {
    // type Migrator<'a> = sqlite::Migrator<'a>;
    type Connection = rusqlite::Connection;
    type Pool = sqlite::Pool;
    type QueryBuilder<'sch> = sqlite::QueryBuilder<'sch>;
    type Table<'sch, 'req> = sqlite::Table<'sch, 'req>;
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::SqliteAdapter;
    use crate::database::registry::Registry;

    fn assert_send_sync<T: Send + Sync>() {}

    /// A registry over a real pool must be shareable across threads, so the request-handling path
    /// that borrows it can run on whichever worker thread owns it.
    #[test]
    fn sqlite_registry_is_send_and_sync() {
        assert_send_sync::<Registry<'static, SqliteAdapter>>();
    }
}
