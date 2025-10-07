mod error;
mod migration;

pub use error::Error;
pub use migration::Migration;

#[cfg(feature = "builtin_migrations")]
mod builtin_migrations;

#[cfg(feature = "builtin_migrations")]
pub use builtin_migrations::builtin_migrations;

use std::error::Error as StdError;

pub trait Migrator {
    fn get_migrations(&self) -> &[Migration];
    fn current_migration_version(&self) -> Result<usize, Box<dyn StdError>>;
    fn has_pending_migrations(&self) -> Result<bool, Box<dyn StdError>>;

    fn migrate_one(&self) -> Result<(), Box<dyn StdError>>;
    fn rollback_one(&self) -> Result<(), Box<dyn StdError>>;
    fn migrate_all(&self) -> Result<(), Box<dyn StdError>>;
    fn rollback_all(&self) -> Result<(), Box<dyn StdError>>;
}