use super::{
    connection::Connection as ConnectionInterface,
    migrator::Migrator as MigratorInterface,
    query_builder::QueryBuilder as QueryBuilderInterface,
    table::Table as TableInterface,
};

#[cfg(feature = "sqlite")]
pub mod sqlite;

pub trait Adapter {
    type Connection : ConnectionInterface;
    type QueryBuilder : QueryBuilderInterface;
    type Migrator : MigratorInterface;
    type Table : TableInterface<Self::Connection, Self::QueryBuilder>;
}

#[cfg(feature = "sqlite")]
pub struct SqliteAdapter<'a>;

#[cfg(feature = "sqlite")]
impl<'a> Adapter for SqliteAdapter<'a> {
    type Table = sqlite::Table<'a>;
    type QueryBuilder = sqlite::QueryBuilder<'a>;
    type Migrator = sqlite::Migrator<'a>;
    type Connection = rusqlite::Connection;
}