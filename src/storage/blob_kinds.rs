//! Blob `kind` tags `OrbKit` uses with `walletkit_db::blobs`.
//!
//! The `kind` byte is hashed into each blob's `content_id`, so it must be
//! unique within `OrbKit`'s namespace. Adding a new tag here does not collide
//! with credential blobs in `walletkit-core` because the vault files are
//! separate (different `SQLite` databases, different `blob_objects` tables).

/// Encrypted PCP package bytes, one entry per tier of a signup.
pub const KIND_PCP_PACKAGE: u8 = 1;
