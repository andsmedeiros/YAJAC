use super::{
    adapters::Adapter as AdapterInterface, error::Error, pool::Pool as PoolInterface,
    registry::Registry, table::Table as TableInterface,
};

/// Binds a validated `Registry` to a connection pool. Owns both — the registry is
/// moved in pre-built — and is the request path's single handle: it lends schemas
/// (through the registry) and hands out request-scoped connections and tables.
pub struct ConnectionManager<'sch, Adapter: AdapterInterface> {
    registry: Registry<'sch>,
    pool: Adapter::Pool,
}

impl<'sch, Adapter: AdapterInterface> ConnectionManager<'sch, Adapter> {
    /// Binds an already-validated registry to a pool.
    pub fn new(registry: Registry<'sch>, pool: Adapter::Pool) -> Self {
        Self { registry, pool }
    }

    /// The underlying schema collection, for consumers that need only schemas.
    pub fn registry(&self) -> &Registry<'sch> {
        &self.registry
    }

    /// Acquires a connection from the pool, held for the request.
    pub fn acquire(&self) -> Result<<Adapter::Pool as PoolInterface>::Handle<'_>, Error> {
        self.pool.acquire()
    }

    /// Builds a request-scoped table bound to `connection`. The schema reference
    /// is lent from the registry, so the table lives no longer than this borrow.
    pub fn table<'req>(
        &self,
        name: &str,
        connection: &'req Adapter::Connection,
    ) -> Result<Adapter::Table<'_, 'req>, Error> {
        Ok(Adapter::Table::new(self.registry.schema(name)?, connection))
    }
}
