use super::{connection::Connection as ConnectionInterface, error::Error};

/// A pool of reusable database connections, lent out one at a time and reclaimed for reuse once a
/// caller is done with them.
pub trait Pool {
    type Connection: ConnectionInterface;

    /// Borrows a connection from the pool for exclusive use, returning it wrapped for the request
    /// path. The connection rejoins the pool when the wrapper is dropped; fails when none can be
    /// obtained (for example, an exhausted pool or a broken connection).
    fn acquire(&self) -> Result<Self::Connection, Error>;
}
