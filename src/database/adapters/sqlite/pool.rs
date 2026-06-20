use super::Connection;
use crate::database::{error::Error, pool::Pool as PoolInterface};

/// Placeholder connection source holding a single connection and lending it for the request.
/// Single-threaded (`!Sync`); a real pool replaces it behind the [`PoolInterface`] seam.
pub struct Pool {
    connection: Connection,
}

impl Pool {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }
}

impl PoolInterface for Pool {
    type Connection = Connection;
    type Handle<'req> = &'req Connection;

    fn acquire(&self) -> Result<Self::Handle<'_>, Error> {
        Ok(&self.connection)
    }
}
