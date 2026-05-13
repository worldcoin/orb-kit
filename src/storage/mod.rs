//! `OrbKit`'s PCP storage layer.
//!
//! Wraps `walletkit-db`'s vault primitives to persist Personal Custody
//! Packages on device.
//!
//! # Quick tour
//!
//! - [`Vault`] opens an encrypted `SQLite` vault at a given path, with a
//!   separate envelope file holding the sealed bulk key.
//! - [`OrbPcpStore`] is the typed API consumers use to read and mutate
//!   `pcp_records` and `vault_meta`.
//! - All public enums round-trip through their oxide-compatible string
//!   names (`Downloaded`, `Enrolled`, ...) so the schema is byte-trivial to
//!   migrate from oxide's JSON lookup table.

pub mod blob_kinds;
pub mod error;
pub mod paths;
pub mod schema;
pub mod store;
pub mod types;

pub use error::{StorageError, StorageResult};
pub use store::{OrbPcpStore, Vault};
pub use types::{CreationSource, PackageStatus, PcpRecord, SignupId, Tier};
