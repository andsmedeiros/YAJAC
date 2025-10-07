use include_dir::{include_dir, Dir};
use itertools::Itertools;
use std::sync::LazyLock;
use super::Migration;

const MIGRATION_FILES: Dir<'_> = include_dir!("$CARGO_WORKSPACE_DIR/migrations");

static MIGRATIONS: LazyLock<Vec<Migration>> = LazyLock::new(||
    MIGRATION_FILES
        .dirs()
        .map(Migration::from)
        .sorted()
        .collect()
);

pub fn builtin_migrations() -> Vec<Migration> {
    MIGRATIONS.clone()
}

