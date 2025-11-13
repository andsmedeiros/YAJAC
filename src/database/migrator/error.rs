use std::error::Error as StdError;
use std::fmt::Display;

#[derive(Debug, Clone)]
pub enum Error {
    UnsortedMigrationRegistry,
    RepeatingMigrationRegistry,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl StdError for Error {}
