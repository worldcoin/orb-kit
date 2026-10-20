//! On-disk format constants. Once shipped, renaming any of these breaks
//! every existing user's vault.

/// Encrypted vault file.
pub const VAULT_FILENAME: &str = "orb_pcp.sqlite";

/// Cross-process lock file.
pub const LOCK_FILENAME: &str = "orb_pcp.lock";

/// CBOR envelope holding the `K_device`-sealed `K_intermediate`.
pub const ENVELOPE_FILENAME: &str = "orb_pcp_keys.bin";

/// AEAD associated data for the envelope wrap. Must differ from
/// `walletkit-core`'s credential AD.
pub const ENVELOPE_AD: &[u8] = b"worldid:orb-pcp-envelope";
