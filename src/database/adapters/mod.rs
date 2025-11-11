use super::{
    connection::Connection as ConnectionInterface,
    migrator::Migrator as MigratorInterface,
    query_builder::QueryBuilder as QueryBuilderInterface,
    table::Table as TableInterface,
};

#[cfg(feature = "sqlite")]
pub mod sqlite;

pub trait Adapter {
    type Connection: ConnectionInterface;
    type QueryBuilder<'a>: QueryBuilderInterface<'a>;
    // type Migrator: MigratorInterface;
    type Table<'a>: TableInterface<'a, Self::Connection, Self::QueryBuilder<'a>>;
}

#[cfg(feature = "sqlite")]
pub struct SqliteAdapter;

#[cfg(feature = "sqlite")]
impl Adapter for SqliteAdapter {
    // type Migrator<'a> = sqlite::Migrator<'a>;
    type Connection = rusqlite::Connection;
    type QueryBuilder<'a> = sqlite::QueryBuilder<'a>;
    type Table<'a> = sqlite::Table<'a>;
}