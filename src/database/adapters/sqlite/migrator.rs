use crate::database::migrator::{Error, Migration, Migrator as MigratorInterface};
use chrono::Utc;
use log::{debug, info};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::error::Error as StdError;

pub struct Migrator<'a> {
    pub connection: &'a Connection,
    pub migrations: Vec<Migration>,
}

impl MigratorInterface for Migrator<'_> {
    fn get_migrations(&self) -> &[Migration] {
        self.migrations.as_slice()
    }

    fn current_migration_version(&self) -> Result<usize, Box<dyn StdError>> {
        let version = self
            .connection
            .query_row(
                "SELECT version FROM migrations ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map(|r| r.unwrap_or(0) as usize)?;

        Ok(version)
    }

    fn has_pending_migrations(&self) -> Result<bool, Box<dyn StdError>> {
        Ok(!self.pending_migrations()?.is_empty())
    }

    fn migrate_one(&self) -> Result<(), Box<dyn StdError>> {
        self.ensure_migrations_table_exists()?;
        info!("Running single pending migration.");

        let current_version = self.current_migration_version()?;
        info!("Current migration version: {}.", current_version);

        if let Some(migration) = self.pending_migrations()?.iter().next() {
            info!(
                "Running migration #{}: {}.",
                migration.version, migration.name
            );
            self.in_transaction(|transaction| Self::run_migration(&transaction, migration))
        } else {
            info!("No migration is pending");
            Ok(())
        }
    }

    fn rollback_one(&self) -> Result<(), Box<dyn StdError>> {
        self.ensure_migrations_table_exists()?;
        info!("Rolling back single migration.");

        let current_version = self.current_migration_version()?;
        info!("Current migration version: {}.", current_version);

        if let Some(migration) = self.executed_migrations()?.iter().last() {
            info!(
                "Rolling back migration #{}: {}.",
                migration.version, migration.name
            );
            self.in_transaction(|transaction| Self::rollback_migration(&transaction, migration))
        } else {
            info!("No migration to rollback.");
            Ok(())
        }
    }

    fn migrate_all(&self) -> Result<(), Box<dyn StdError>> {
        self.ensure_migrations_table_exists()?;
        info!("Running all pending migrations.");

        let current_version = self.current_migration_version()?;
        info!("Current migration version: {}.", current_version);

        let migrations = self.pending_migrations()?;

        if !migrations.is_empty() {
            self.in_transaction(|transaction| {
                for migration in migrations {
                    info!(
                        "Running migration #{}: {}.",
                        migration.version, migration.name
                    );
                    Self::run_migration(&transaction, &migration)?;
                }
                Ok(())
            })?;

            let version = self.current_migration_version()?;
            info!("Database migrated to version {version}.");
        } else {
            info!("No migration is pending");
        }

        Ok(())
    }

    fn rollback_all(&self) -> Result<(), Box<dyn StdError>> {
        self.ensure_migrations_table_exists()?;
        info!("Rolling back all migrations.");

        let current_version = self.current_migration_version()?;
        info!("Current migration version: {}.", current_version);

        let migrations = self.executed_migrations()?;
        if !migrations.is_empty() {
            self.in_transaction(|transaction| {
                for migration in migrations.iter().rev() {
                    info!(
                        "Rolling back migration #{}: {}.",
                        migration.version, migration.name
                    );
                    Self::rollback_migration(&transaction, migration)?;
                }
                Ok(())
            })?;

            let version = self.current_migration_version()?;
            info!("Database rolled back to version {version}.");
        } else {
            info!("No migration to rollback.");
        }

        Ok(())
    }
}

impl<'a> Migrator<'a> {
    pub fn try_new(
        connection: &'a Connection,
        migrations: Vec<Migration>,
    ) -> Result<Self, Box<dyn StdError>> {
        for pair in migrations.windows(2) {
            if pair[0].version == pair[1].version {
                Err(Error::RepeatingMigrationRegistry)?;
            }

            if pair[0].version > pair[1].version {
                Err(Error::UnsortedMigrationRegistry)?;
            }
        }

        let migrator = Migrator {
            connection,
            migrations,
        };
        migrator.ensure_migrations_table_exists()?;

        Ok(migrator)
    }

    fn ensure_migrations_table_exists(&self) -> Result<(), Box<dyn StdError>> {
        debug!("Ensuring migrations table exists.");
        self.connection.execute(
            "\
                CREATE TABLE IF NOT EXISTS migrations (\
                    version INTEGER NOT NULL PRIMARY KEY, \
                    name TEXT NOT NULL, \
                    migrated_at INTEGER NOT NULL\
                )\
            ",
            [],
        )?;

        Ok(())
    }

    fn migration_index(&self, current_version: usize) -> Result<usize, Box<dyn StdError>> {
        let index = self
            .migrations
            .iter()
            .position(|m| m.version == current_version)
            .ok_or_else(|| {
                format!(
                    "Migration #{}, reported by database as latest, does not exist in registry.",
                    current_version
                )
            })?;

        Ok(index)
    }

    fn pending_migrations(&'a self) -> Result<&'a [Migration], Box<dyn StdError>> {
        let current_version = self.current_migration_version()?;
        if current_version == 0 {
            Ok(self.migrations.as_slice())
        } else {
            let index = self.migration_index(current_version)?;
            Ok(&self.migrations[index + 1..])
        }
    }

    fn executed_migrations(&'a self) -> Result<&'a [Migration], Box<dyn StdError>> {
        let current_version = self.current_migration_version()?;
        if current_version == 0 {
            Ok(&[])
        } else {
            let index = self.migration_index(current_version)?;
            Ok(&self.migrations[..index + 1])
        }
    }

    fn in_transaction<B>(&self, block: B) -> Result<(), Box<dyn StdError>>
    where
        B: FnOnce(&Transaction) -> Result<(), Box<dyn StdError>>,
    {
        let transaction = self.connection.unchecked_transaction()?;
        block(&transaction)?;
        transaction
            .commit()
            .map_err(|err| format!("Failed to commit transaction: {}", err).into())
    }

    fn run_migration(
        transaction: &Transaction,
        migration: &Migration,
    ) -> Result<(), Box<dyn StdError>> {
        transaction.execute_batch(&migration.up)?;
        transaction.execute(
            "\
                    INSERT INTO migrations (version, name, migrated_at) \
                    VALUES ($1, $2, $3)\
                ",
            params![migration.version, migration.name, Utc::now()],
        )?;

        Ok(())
    }

    fn rollback_migration(
        transaction: &Transaction,
        migration: &Migration,
    ) -> Result<(), Box<dyn StdError>> {
        transaction.execute_batch(&migration.down)?;
        transaction.execute(
            "\
                    DELETE FROM migrations \
                    WHERE version = $1\
                ",
            params![migration.version],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Error as SqliteError;

    // Test migrations
    fn test_migrations() -> Vec<Migration> {
        vec![
            Migration {
                version: 1,
                name: "create_users".into(),
                up: "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);".into(),
                down: "DROP TABLE users;".into(),
            },
            Migration {
                version: 2,
                name: "add_email_to_users".into(),
                up: "ALTER TABLE users ADD COLUMN email TEXT;".into(),
                down: "ALTER TABLE users DROP COLUMN email".into(),
            },
            Migration {
                version: 3,
                name: "create_posts".into(),
                up: "CREATE TABLE posts (
                        id INTEGER PRIMARY KEY,
                        user_id INTEGER NOT NULL REFERENCES users(id),
                        title TEXT NOT NULL,
                        content TEXT NOT NULL
                    );".into(),
                down: "DROP TABLE posts;".into(),
            },
            Migration {
                version: 4,
                name: "add_timestamps".into(),
                up: "ALTER TABLE users ADD COLUMN created_at INTEGER NOT NULL DEFAULT CURRENT_TIMESTAMP;
                     ALTER TABLE posts ADD COLUMN created_at INTEGER NOT NULL DEFAULT CURRENT_TIMESTAMP;".into(),
                down: "ALTER TABLE posts DROP COLUMN created_at;
                       ALTER TABLE users DROP COLUMN created_at;".into(),
            }
        ]
    }

    // Failing migrations for testing error scenarios
    fn failing_migrations() -> Vec<Migration> {
        vec![
            Migration {
                version: 1,
                name: "invalid_sql".into(),
                up: "THIS IS NOT VALID SQL;".into(),
                down: "DROP TABLE IF EXISTS test;".into(),
            },
            Migration {
                version: 2,
                name: "dependent_table".into(),
                up: "CREATE TABLE dependent (
                         id INTEGER PRIMARY KEY,
                         test_id INTEGER NOT NULL REFERENCES test(id)
                     );"
                .into(),
                down: "DROP TABLE dependent; DROP TABLE test;".into(),
            },
            Migration {
                version: 3,
                name: "violates_foreign_key".into(),
                up: "INSERT INTO dependent (id, test_id) VALUES (1, 999);".into(),
                down: "DELETE FROM dependent WHERE id = 1;".into(),
            },
        ]
    }

    fn setup_test_db() -> Result<Connection, Box<dyn StdError>> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    #[test]
    fn test_empty_database_reports_version_zero() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let migrator = Migrator::try_new(&conn, test_migrations())?;

        assert_eq!(migrator.current_migration_version()?, 0);
        Ok(())
    }

    #[test]
    fn test_migrations_table_creation() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let _ = Migrator::try_new(&conn, test_migrations())?;

        let table_info: Vec<(String, String)> = conn
            .prepare("PRAGMA table_info(migrations)")?
            .query_map([], |row| Ok((row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, SqliteError>>()?;

        assert_eq!(table_info.len(), 3);
        assert!(table_info.contains(&("version".to_string(), "INTEGER".to_string())));
        assert!(table_info.contains(&("name".to_string(), "TEXT".to_string())));
        assert!(table_info.contains(&("migrated_at".to_string(), "INTEGER".to_string())));

        Ok(())
    }

    #[test]
    fn test_successful_migration_sequence() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let migrator = Migrator::try_new(&conn, test_migrations())?;

        assert!(migrator.has_pending_migrations()?);

        // First migration - creates users table
        migrator.migrate_one()?;
        assert_eq!(migrator.current_migration_version()?, 1);
        assert!(
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='users'",
                [],
                |row| row.get::<_, i32>(0)
            )
            .is_ok()
        );

        // Second migration - adds email column
        migrator.migrate_one()?;
        assert_eq!(migrator.current_migration_version()?, 2);
        assert!(
            conn.query_row(
                "SELECT 1 FROM pragma_table_info('users') WHERE name='email'",
                [],
                |_| Ok(())
            )
            .is_ok()
        );

        // Complete remaining migrations
        migrator.migrate_all()?;
        assert_eq!(migrator.current_migration_version()?, 4);
        assert!(!migrator.has_pending_migrations()?);

        Ok(())
    }

    #[test]
    fn test_successful_rollback_sequence() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let migrator = Migrator::try_new(&conn, test_migrations())?;

        // Apply all migrations
        migrator.migrate_all()?;
        assert_eq!(migrator.current_migration_version()?, 4);
        assert!(!migrator.has_pending_migrations()?);

        // Rollback timestamps migration
        migrator.rollback_one()?;
        assert_eq!(migrator.current_migration_version()?, 3);
        assert!(
            conn.query_row("SELECT created_at FROM users LIMIT 0", [], |_| Ok(()))
                .is_err()
        );
        assert!(migrator.has_pending_migrations()?);

        // Rollback remaining migrations
        migrator.rollback_all()?;
        assert_eq!(migrator.current_migration_version()?, 0);
        assert!(
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='users'",
                [],
                |_| Ok(())
            )
            .is_err()
        );
        assert!(migrator.has_pending_migrations()?);

        Ok(())
    }

    #[test]
    fn test_invalid_sql_migration() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let migrator = Migrator::try_new(&conn, failing_migrations())?;

        let result = migrator.migrate_one();
        assert!(result.is_err());
        assert_eq!(migrator.current_migration_version()?, 0);

        Ok(())
    }

    #[test]
    fn test_foreign_key_violation() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let migrator = Migrator::try_new(&conn, failing_migrations())?;

        // First migration should fail
        assert!(migrator.migrate_one().is_err());

        // Replace with valid first migration
        let mut migrations = failing_migrations();
        migrations[0] = Migration {
            version: 1,
            name: "create_test".into(),
            up: "CREATE TABLE test (id INTEGER PRIMARY KEY);".into(),
            down: "DROP TABLE test;".into(),
        };
        let migrator = Migrator::try_new(&conn, migrations)?;

        // Now first migration should succeed
        assert!(migrator.migrate_one().is_ok());
        assert_eq!(migrator.current_migration_version()?, 1);

        // Second migration (dependent table) should succeed
        assert!(migrator.migrate_one().is_ok());
        assert_eq!(migrator.current_migration_version()?, 2);

        // Third migration should fail due to foreign key violation
        assert!(migrator.migrate_one().is_err());
        assert_eq!(migrator.current_migration_version()?, 2);

        Ok(())
    }

    #[test]
    fn test_transaction_rollback_on_failure() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let mut migrations = test_migrations();

        // Add a failing statement at the end of a migration
        migrations[0].up.push_str("THIS IS INVALID SQL;");
        let migrator = Migrator::try_new(&conn, migrations)?;

        // Migration should fail
        assert!(migrator.migrate_one().is_err());

        // Database should be in initial state
        assert_eq!(migrator.current_migration_version()?, 0);
        assert!(
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='users'",
                [],
                |_| Ok(())
            )
            .is_err()
        );

        Ok(())
    }

    #[test]
    fn test_multiple_statements_in_migration() -> Result<(), Box<dyn StdError>> {
        let conn = setup_test_db()?;
        let migrations = vec![Migration {
            version: 1,
            name: "multiple_statements".into(),
            up: "CREATE TABLE test1 (id INTEGER PRIMARY KEY);
                 CREATE TABLE test2 (id INTEGER PRIMARY KEY);
                 CREATE TABLE test3 (id INTEGER PRIMARY KEY);"
                .into(),
            down: "DROP TABLE test3;
                  DROP TABLE test2;
                  DROP TABLE test1;"
                .into(),
        }];

        let migrator = Migrator::try_new(&conn, migrations)?;
        migrator.migrate_one()?;

        // Verify all tables were created
        for i in 1..=3 {
            assert!(
                conn.query_row(
                    &format!(
                        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='test{}'",
                        i
                    ),
                    [],
                    |_| Ok(())
                )
                .is_ok()
            );
        }

        migrator.rollback_one()?;

        // Verify all tables were dropped
        for i in 1..=3 {
            assert!(
                conn.query_row(
                    &format!(
                        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='test{}'",
                        i
                    ),
                    [],
                    |_| Ok(())
                )
                .is_err()
            );
        }

        Ok(())
    }

    #[test]
    fn test_migration_version_uniqueness() -> Result<(), Box<dyn StdError>> {
        if cfg!(debug_assertions) {
            let conn = setup_test_db()?;
            let migrations = [
                vec![Migration {
                    version: 1, // Duplicate version
                    name: "duplicate_version".into(),
                    up: "CREATE TABLE test (id INTEGER PRIMARY KEY);".into(),
                    down: "DROP TABLE test;".into(),
                }],
                test_migrations(),
            ]
            .concat();

            let result = Migrator::try_new(&conn, migrations);
            assert!(result.is_err());
        }

        Ok(())
    }

    #[test]
    fn test_migration_version_order() -> Result<(), Box<dyn StdError>> {
        if cfg!(debug_assertions) {
            let conn = setup_test_db()?;
            let migrations = [
                vec![Migration {
                    version: 5, // Unsorted migration
                    name: "unsorted_migration".into(),
                    up: "CREATE TABLE other (id INTEGER PRIMARY KEY);".into(),
                    down: "DROP TABLE other;".into(),
                }],
                test_migrations(),
            ]
            .concat();

            let result = Migrator::try_new(&conn, migrations);
            assert!(result.is_err());
        }

        Ok(())
    }
}
