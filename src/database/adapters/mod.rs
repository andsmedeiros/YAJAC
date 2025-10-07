#[cfg(feature = "sqlite")]
pub mod sqlite;

pub trait Adapter {
    type Table;
    type QueryBuilder;
    type Migrator;
}

#[cfg(feature = "sqlite")]
pub struct SqliteAdapter<'a>;

#[cfg(feature = "sqlite")]
impl<'a> Adapter for SqliteAdapter<'a> {
    type Table = sqlite::Table<'a>;
    type QueryBuilder = sqlite::QueryBuilder<'a>;
    type Migrator = sqlite::Migrator<'a>;
}