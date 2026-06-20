use super::{connection::Connection as ConnectionInterface, error::Error};
use std::ops::Deref;

/// Source of database connections for a request.
///
/// `acquire` yields a handle held for the duration of a request (`'req`) and dereferenced to a
/// `Connection`. The handle may borrow the pool (a single shared connection) or own its
/// connection (a real pool), so it is generic over `'req`.
pub trait Pool {
    type Connection: ConnectionInterface;
    type Handle<'req>: Deref<Target = Self::Connection>
    where
        Self: 'req;

    fn acquire(&self) -> Result<Self::Handle<'_>, Error>;
}
