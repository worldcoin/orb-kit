//! Filename constants for `OrbKit`'s storage files.
//!
//! These names are part of `OrbKit`'s contract with the host and with the
//! sealed envelope's associated-data binding. **Do not rename without a
//! migration plan**: changing `ENVELOPE_FILENAME` orphans existing vaults,
//! and changing `ENVELOPE_AD` makes the existing envelope refuse to open.

/// Encrypted `SQLite` vault file holding `pcp_records`, `vault_meta`, and
/// `blob_objects`.
pub const VAULT_FILENAME: &str = "orb_pcp.sqlite";

/// Lock file used to serialize mutations across processes.
pub const LOCK_FILENAME: &str = "orb_pcp.lock";

/// CBOR envelope persisted via the host's `AtomicBlobStore`. Holds the
/// `K_device`-sealed `K_intermediate` for the PCP vault.
pub const ENVELOPE_FILENAME: &str = "orb_pcp_keys.bin";

/// Associated-data string bound into the AEAD wrap of `K_intermediate`.
/// Must differ from `walletkit-core`'s credential envelope AD.
pub const ENVELOPE_AD: &[u8] = b"worldid:orb-pcp-envelope";
