use super::Migration;
use include_dir::{Dir, include_dir};
use itertools::Itertools;
use std::sync::LazyLock;

const MIGRATION_FILES: Dir<'_> = include_dir!("$CARGO_WORKSPACE_DIR/migrations");

static MIGRATIONS: LazyLock<Vec<Migration>> = LazyLock::new(|| {
    MIGRATION_FILES
        .dirs()
        .map(Migration::from)
        .sorted()
        .collect()
});

pub fn builtin_migrations() -> Vec<Migration> {
    MIGRATIONS.clone()
}
