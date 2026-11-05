//! Persistence failures.

/// A failure in a repository or migration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The database is unavailable or the query failed.
    #[error("database error: {0}")]
    Database(String),
    /// A uniqueness or state-transition conflict.
    #[error("state conflict: {0}")]
    Conflict(String),
    /// The requested record does not exist.
    #[error("record not found")]
    NotFound,
}
