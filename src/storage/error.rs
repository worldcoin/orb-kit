//! Error type returned by the storage layer.

use thiserror::Error;
use walletkit_db::{DbError, StoreError};

use crate::storage::types::PackageStatus;

/// Result alias for [`StorageError`].
pub type StorageResult<T> = Result<T, StorageError>;

/// Errors raised by [`crate::storage::OrbPcpStore`].
#[derive(Debug, Error)]
#[allow(missing_docs)]
pub enum StorageError {
    #[error("walletkit-db: {0}")]
    WalletKitDb(#[from] StoreError),
    #[error("sqlite: {0}")]
    Db(#[from] DbError),
    #[error("no rows for the requested signup")]
    SignupNotFound,
    #[error("illegal status transition: {from} -> {to}")]
    IllegalTransition {
        from: PackageStatus,
        to: PackageStatus,
    },
    #[error("invalid state: {0}")]
    InvalidState(String),
}
