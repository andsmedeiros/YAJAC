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
