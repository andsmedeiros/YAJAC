use super::Connection;
use crate::database::{error::Error, pool::Pool as PoolInterface};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

/// Statements run on every new connection before it enters the pool: enables write-ahead logging,
/// foreign-key enforcement and a busy timeout.
pub fn default_preamble(connection: &mut Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL; \
         PRAGMA foreign_keys = ON; \
         PRAGMA busy_timeout = 5000;",
    )
}

/// An r2d2-backed connection pool over SQLite.
pub struct Pool {
    inner: r2d2::Pool<SqliteConnectionManager>,
}

impl Pool {
    /// Opens a file-backed pool, running [`default_preamble`] on each connection.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::from_manager(
            SqliteConnectionManager::file(path).with_init(default_preamble),
            None,
        )
    }

    /// Opens a file-backed pool, running `preamble` on each connection.
    pub fn open_with(
        path: impl AsRef<Path>,
        preamble: impl Fn(&mut Connection) -> rusqlite::Result<()> + Send + Sync + 'static,
    ) -> Result<Self, Error> {
        Self::from_manager(
            SqliteConnectionManager::file(path).with_init(preamble),
            None,
        )
    }

    /// Opens a single-connection in-memory pool, running [`default_preamble`]. Suited to tests and
    /// single-process embedding, where one connection keeps the in-memory database alive.
    pub fn memory() -> Result<Self, Error> {
        Self::from_manager(
            SqliteConnectionManager::memory().with_init(default_preamble),
            Some(1),
        )
    }

    fn from_manager(
        manager: SqliteConnectionManager,
        max_size: Option<u32>,
    ) -> Result<Self, Error> {
        let mut builder = r2d2::Pool::builder();

        if let Some(size) = max_size {
            builder = builder.max_size(size).min_idle(Some(size));
        }

        let inner = builder
            .build(manager)
            .map_err(|error| Error::DatabaseFailure {
                message: error.to_string(),
            })?;

        Ok(Self { inner })
    }
}

impl PoolInterface for Pool {
    type Connection = Connection;
    type Handle<'req> = PooledConnection<SqliteConnectionManager>;

    fn acquire(&self) -> Result<Self::Handle<'_>, Error> {
        self.inner.get().map_err(|error| Error::DatabaseFailure {
            message: error.to_string(),
        })
    }
}
