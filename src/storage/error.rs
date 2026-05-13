//! Error type returned by the storage layer.

use thiserror::Error;
use walletkit_db::{DbError, StoreError};

/// Result alias for [`StorageError`].
pub type StorageResult<T> = Result<T, StorageError>;

/// Errors raised by [`crate::storage::OrbPcpStore`] and related operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Underlying error from `walletkit-db` (vault open, blob IO, envelope,
    /// lock, integrity check).
    #[error("walletkit-db: {0}")]
    WalletKitDb(#[from] StoreError),
    /// Low-level `SQLite` error surfaced from a direct query (statement
    /// prepare, step, bind).
    #[error("sqlite: {0}")]
    Db(#[from] DbError),
    /// Attempted a value that violates the row state machine
    /// (illegal status transition, malformed enum string, negative
    /// timestamp).
    #[error("invalid state: {0}")]
    InvalidState(String),
}
