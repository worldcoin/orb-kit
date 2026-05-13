//! `OrbPcpStore`, the typed CRUD surface over `OrbKit`'s PCP vault.
//!
//! Layered on `walletkit_db::Vault`: reads bypass the lock, mutations
//! acquire it implicitly via `Vault::mutate`. Multi-statement writes run
//! inside a single `mutate` closure so they are atomic across processes.

use std::path::Path;

use walletkit_db::{
    blobs, params, AtomicBlobStore, Connection, Keystore, Lock, StepResult,
    Vault as WkVault,
};

use crate::storage::blob_kinds::KIND_PCP_PACKAGE;
use crate::storage::error::{StorageError, StorageResult};
use crate::storage::paths::{ENVELOPE_AD, ENVELOPE_FILENAME};
use crate::storage::schema::{ensure_schema, SCHEMA_VERSION};
use crate::storage::types::{CreationSource, PackageStatus, PcpRecord, SignupId, Tier};

/// `OrbKit`'s encrypted PCP vault.
///
/// Wraps `walletkit_db::Vault` with PCP-shaped read / mutate helpers.
/// Construct via [`Vault::open`], then call the operations on it.
pub struct Vault {
    inner: WkVault,
}

impl Vault {
    /// Open the `OrbKit` vault, sealing the bulk key with `keystore` and
    /// persisting the envelope via `blob_store`.
    ///
    /// On first use this generates `K_intermediate` and writes the
    /// envelope. On subsequent calls the envelope is unsealed and the same
    /// bulk key is recovered.
    ///
    /// # Errors
    ///
    /// Propagates errors from envelope IO, keystore seal / open, vault
    /// open, schema setup, or the integrity check.
    pub fn open(
        vault_path: &Path,
        envelope_now_seconds: u64,
        lock: Lock,
        keystore: &dyn Keystore,
        blob_store: &dyn AtomicBlobStore,
    ) -> StorageResult<Self> {
        let key = walletkit_db::init_or_open_envelope_key(
            keystore,
            blob_store,
            &lock,
            ENVELOPE_FILENAME,
            ENVELOPE_AD,
            envelope_now_seconds,
        )?;
        let inner = WkVault::open(vault_path, &key, lock, ensure_schema)?;
        Ok(Self { inner })
    }

    /// Borrow the typed PCP store.
    #[must_use]
    pub const fn store(&self) -> OrbPcpStore<'_> {
        OrbPcpStore { vault: &self.inner }
    }
}

/// Typed CRUD surface over `pcp_records` and `vault_meta`.
///
/// Borrowed from [`Vault::store`]; cheap to recreate, holds no state of
/// its own.
pub struct OrbPcpStore<'a> {
    vault: &'a WkVault,
}

impl OrbPcpStore<'_> {
    /// Initialise `vault_meta` with the current schema version. Idempotent:
    /// repeated calls are no-ops once the singleton row exists.
    ///
    /// # Errors
    ///
    /// Database errors propagate.
    pub fn init_meta(&self, now_seconds: u64) -> StorageResult<()> {
        self.vault.mutate(|conn| -> StorageResult<()> {
            let now_i64 = to_i64(now_seconds, "now")?;
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM vault_meta)",
                &[],
                |row| Ok(row.column_i64(0) != 0),
            )?;
            if !exists {
                conn.execute(
                    "INSERT INTO vault_meta (
                        schema_version, sub, current_signup_id, created_at, updated_at
                     ) VALUES (?1, NULL, NULL, ?2, ?2)",
                    params![SCHEMA_VERSION, now_i64],
                )?;
            }
            Ok(())
        })
    }

    /// Insert (or replace) one tier of a signup's PCP. Writes the blob
    /// bytes via `walletkit_db::blobs::put` and a `pcp_records` row in
    /// one transaction.
    ///
    /// Re-inserting with the same `(signup_id, tier)` overwrites the
    /// existing row (used when the backend re-issues a tier).
    ///
    /// # Errors
    ///
    /// Database errors propagate. Timestamps overflowing `i64` return
    /// [`StorageError::InvalidState`].
    pub fn put_package(&self, ingest: &PcpIngest<'_>) -> StorageResult<[u8; 32]> {
        self.vault.mutate(|conn| -> StorageResult<[u8; 32]> {
            let now_i64 = to_i64(ingest.now_seconds, "now")?;
            let orb_i64 = to_i64(ingest.orb_created_at_seconds, "orb_created_at")?;
            let cid = blobs::put(
                conn,
                KIND_PCP_PACKAGE,
                ingest.encrypted_bytes,
                ingest.now_seconds,
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO pcp_records (
                    signup_id, tier, version, signup_reason,
                    status, is_download_acknowledged, creation_source,
                    package_blob_cid, orb_created_at, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    ingest.signup_id,
                    i64::from(ingest.tier),
                    ingest.version,
                    ingest.signup_reason.unwrap_or_default(),
                    PackageStatus::Downloaded.as_str(),
                    i64::from(u8::from(ingest.is_download_acknowledged)),
                    ingest.creation_source.as_str(),
                    cid.as_slice(),
                    orb_i64,
                    now_i64,
                ],
            )?;
            let mut out = [0u8; 32];
            out.copy_from_slice(&cid);
            Ok(out)
        })
    }

    /// Update the status of every tier of a signup atomically. Rejects
    /// illegal transitions via [`PackageStatus::can_transition_to`].
    /// Idempotent: setting a row to its current status is allowed.
    ///
    /// `new_source`, if supplied, overwrites `creation_source` (used when
    /// the backend re-classifies the signup).
    ///
    /// # Errors
    ///
    /// `InvalidState` if no rows match `signup_id` or any existing row
    /// cannot legally transition to `new_status`.
    pub fn update_status(
        &self,
        signup_id: &str,
        new_status: PackageStatus,
        new_source: Option<CreationSource>,
        now_seconds: u64,
    ) -> StorageResult<()> {
        self.vault.mutate(|conn| -> StorageResult<()> {
            let now_i64 = to_i64(now_seconds, "now")?;
            let current = read_signup_statuses(conn, signup_id)?;
            if current.is_empty() {
                return Err(StorageError::InvalidState(format!(
                    "no rows for signup_id={signup_id}"
                )));
            }
            for cur in &current {
                if *cur != new_status && !cur.can_transition_to(new_status) {
                    return Err(StorageError::InvalidState(format!(
                        "illegal transition {} -> {} for signup_id={signup_id}",
                        cur.as_str(),
                        new_status.as_str()
                    )));
                }
            }
            if let Some(src) = new_source {
                conn.execute(
                    "UPDATE pcp_records
                        SET status = ?1, creation_source = ?2, updated_at = ?3
                      WHERE signup_id = ?4",
                    params![new_status.as_str(), src.as_str(), now_i64, signup_id],
                )?;
            } else {
                conn.execute(
                    "UPDATE pcp_records
                        SET status = ?1, updated_at = ?2
                      WHERE signup_id = ?3",
                    params![new_status.as_str(), now_i64, signup_id],
                )?;
            }
            Ok(())
        })
    }

    /// Mark one tier as acked. Per-tier, since each tier is downloaded
    /// and acked independently to the backend.
    ///
    /// # Errors
    ///
    /// Database errors propagate.
    pub fn mark_ack(
        &self,
        signup_id: &str,
        tier: Tier,
        now_seconds: u64,
    ) -> StorageResult<()> {
        self.vault.mutate(|conn| -> StorageResult<()> {
            let now_i64 = to_i64(now_seconds, "now")?;
            conn.execute(
                "UPDATE pcp_records
                    SET is_download_acknowledged = 1, updated_at = ?1
                  WHERE signup_id = ?2 AND tier = ?3",
                params![now_i64, signup_id, i64::from(tier)],
            )?;
            Ok(())
        })
    }

    /// All rows for a signup, ordered by tier ascending.
    ///
    /// # Errors
    ///
    /// Database errors propagate.
    pub fn tiers_for_signup(&self, signup_id: &str) -> StorageResult<Vec<PcpRecord>> {
        let conn = self.vault.read();
        read_signup_rows(conn, signup_id)
    }

    /// The latest signup that has at least one tier in `Enrolled` status,
    /// returned as all of its tier rows ordered by tier ascending. `None`
    /// if no signup is currently `Enrolled`.
    ///
    /// Latest is defined as `MAX(orb_created_at)` across `Enrolled` rows.
    /// This replaces oxide's `find_enrolled_pcp`.
    ///
    /// # Errors
    ///
    /// Database errors propagate.
    pub fn latest_enrolled(&self) -> StorageResult<Option<Vec<PcpRecord>>> {
        let conn = self.vault.read();
        let signup: Option<String> = conn.query_row_optional(
            "SELECT signup_id FROM pcp_records
              WHERE status = 'Enrolled'
              GROUP BY signup_id
              ORDER BY MAX(orb_created_at) DESC
              LIMIT 1",
            &[],
            |row| Ok(row.column_text(0)),
        )?;
        match signup {
            Some(signup_id) => Ok(Some(read_signup_rows(conn, &signup_id)?)),
            None => Ok(None),
        }
    }

    /// All `(signup_id, tier)` pairs whose ack is still pending. Used by
    /// the cold-start retry path that re-sends ack to the backend.
    ///
    /// # Errors
    ///
    /// Database errors propagate.
    pub fn unacked_tiers(&self) -> StorageResult<Vec<(SignupId, Tier)>> {
        let conn = self.vault.read();
        let mut stmt = conn.prepare(
            "SELECT signup_id, tier FROM pcp_records WHERE is_download_acknowledged = 0",
        )?;
        let mut out = Vec::new();
        while let StepResult::Row(row) = stmt.step()? {
            let signup_id: String = row.column_text(0);
            let tier_i64 = row.column_i64(1);
            let tier = u8::try_from(tier_i64).map_err(|_| {
                StorageError::InvalidState(format!("tier out of range: {tier_i64}"))
            })?;
            out.push((signup_id, tier));
        }
        Ok(out)
    }
}

/// Arguments for [`OrbPcpStore::put_package`].
#[allow(missing_docs)]
pub struct PcpIngest<'a> {
    pub signup_id: &'a str,
    pub tier: Tier,
    pub version: &'a str,
    pub signup_reason: Option<&'a str>,
    pub creation_source: CreationSource,
    pub is_download_acknowledged: bool,
    pub encrypted_bytes: &'a [u8],
    pub orb_created_at_seconds: u64,
    pub now_seconds: u64,
}

// ---- private SQL helpers ------------------------------------------------

fn to_i64(value: u64, label: &str) -> StorageResult<i64> {
    i64::try_from(value).map_err(|_| {
        StorageError::InvalidState(format!("{label} overflows i64: {value}"))
    })
}

fn read_signup_statuses(
    conn: &Connection,
    signup_id: &str,
) -> StorageResult<Vec<PackageStatus>> {
    let mut stmt = conn
        .prepare("SELECT status FROM pcp_records WHERE signup_id = ?1 ORDER BY tier")?;
    stmt.bind_values(params![signup_id])?;
    let mut out = Vec::new();
    while let StepResult::Row(row) = stmt.step()? {
        out.push(PackageStatus::parse(&row.column_text(0))?);
    }
    Ok(out)
}

fn read_signup_rows(
    conn: &Connection,
    signup_id: &str,
) -> StorageResult<Vec<PcpRecord>> {
    let mut stmt = conn.prepare(
        "SELECT signup_id, tier, version, signup_reason, status,
                is_download_acknowledged, creation_source, package_blob_cid,
                orb_created_at, created_at, updated_at
           FROM pcp_records
          WHERE signup_id = ?1
          ORDER BY tier",
    )?;
    stmt.bind_values(params![signup_id])?;
    let mut out = Vec::new();
    while let StepResult::Row(row) = stmt.step()? {
        out.push(row_to_record(&row)?);
    }
    Ok(out)
}

fn row_to_record(row: &walletkit_db::Row<'_, '_>) -> StorageResult<PcpRecord> {
    let signup_id = row.column_text(0);
    let tier_i64 = row.column_i64(1);
    let tier = u8::try_from(tier_i64).map_err(|_| {
        StorageError::InvalidState(format!("tier out of range: {tier_i64}"))
    })?;
    let version = row.column_text(2);
    let signup_reason_text = row.column_text(3);
    let signup_reason = if signup_reason_text.is_empty() {
        None
    } else {
        Some(signup_reason_text)
    };
    let status = PackageStatus::parse(&row.column_text(4))?;
    let ack = row.column_i64(5) != 0;
    let creation_source = CreationSource::parse(&row.column_text(6))?;
    let cid_bytes = row.column_blob(7);
    if cid_bytes.len() != 32 {
        return Err(StorageError::InvalidState(format!(
            "package_blob_cid len {} != 32",
            cid_bytes.len()
        )));
    }
    let mut cid = [0u8; 32];
    cid.copy_from_slice(&cid_bytes);
    let orb_created_at = u64::try_from(row.column_i64(8))
        .map_err(|_| StorageError::InvalidState("orb_created_at negative".into()))?;
    let created_at = u64::try_from(row.column_i64(9))
        .map_err(|_| StorageError::InvalidState("created_at negative".into()))?;
    let updated_at = u64::try_from(row.column_i64(10))
        .map_err(|_| StorageError::InvalidState("updated_at negative".into()))?;
    Ok(PcpRecord {
        signup_id,
        tier,
        version,
        signup_reason,
        status,
        is_download_acknowledged: ack,
        creation_source,
        package_blob_cid: cid,
        orb_created_at,
        created_at,
        updated_at,
    })
}
