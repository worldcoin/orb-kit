//! `OrbPcpStore`, typed CRUD over `OrbKit`'s PCP vault.
//!
//! Reads borrow the connection; multi-statement writes wrap
//! `Connection::transaction`. WAL handles writer serialization, so no
//! cross-process lock is held outside the envelope-init bootstrap.

use std::path::Path;

use walletkit_db::{
    blobs, params, AtomicBlobStore, Connection, Keystore, Lock, Row, StepResult, Value,
    Vault,
};

use crate::storage::error::{StorageError, StorageResult};
use crate::storage::paths::{ENVELOPE_AD, ENVELOPE_FILENAME};
use crate::storage::schema::ensure_schema;
use crate::storage::types::{CreationSource, PackageStatus, PcpRecord, SignupId, Tier};

/// Blob kind tag distinguishing PCP package bytes from any other blob
/// kinds we add later. Must stay stable: it's hashed into `content_id`.
const KIND_PCP_PACKAGE: u8 = 1;

/// Typed CRUD surface over `pcp_records`.
pub struct OrbPcpStore {
    vault: Vault,
}

impl OrbPcpStore {
    /// Open (or create) the encrypted PCP vault.
    ///
    /// # Errors
    ///
    /// Propagates envelope, vault, schema, and integrity-check failures.
    pub fn open(
        vault_path: &Path,
        envelope_now_seconds: u64,
        lock: &Lock,
        keystore: &dyn Keystore,
        blob_store: &dyn AtomicBlobStore,
    ) -> StorageResult<Self> {
        let key = walletkit_db::init_or_open_envelope_key(
            keystore,
            blob_store,
            lock,
            ENVELOPE_FILENAME,
            ENVELOPE_AD,
            envelope_now_seconds,
        )?;
        let vault = Vault::open(vault_path, &key, ensure_schema)?;
        Ok(Self { vault })
    }

    /// Insert (or replace) one tier of a signup.
    ///
    /// # Errors
    ///
    /// Database errors; [`StorageError::InvalidState`] if a timestamp
    /// overflows `i64`.
    pub fn put_package(&self, ingest: &PcpIngest<'_>) -> StorageResult<[u8; 32]> {
        let now_i64 = to_i64(ingest.now_seconds, "now")?;
        let orb_i64 = to_i64(ingest.orb_created_at_seconds, "orb_created_at")?;
        let conn = self.vault.connection();
        let tx = conn.transaction()?;
        let cid = blobs::put(
            conn,
            KIND_PCP_PACKAGE,
            ingest.encrypted_bytes,
            ingest.now_seconds,
        )?;
        tx.execute(
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
        tx.commit()?;
        Ok(cid)
    }

    /// Move every tier of a signup to `new_status`. Idempotent.
    ///
    /// # Errors
    ///
    /// [`StorageError::SignupNotFound`] if no rows match;
    /// [`StorageError::IllegalTransition`] if any existing row cannot
    /// reach `new_status` per [`PackageStatus::can_transition_to`].
    pub fn update_status(
        &self,
        signup_id: &str,
        new_status: PackageStatus,
        new_source: Option<CreationSource>,
        now_seconds: u64,
    ) -> StorageResult<()> {
        let now_i64 = to_i64(now_seconds, "now")?;
        let conn = self.vault.connection();
        let tx = conn.transaction()?;
        let current = read_signup_statuses(conn, signup_id)?;
        if current.is_empty() {
            return Err(StorageError::SignupNotFound);
        }
        for cur in &current {
            if *cur != new_status && !cur.can_transition_to(new_status) {
                return Err(StorageError::IllegalTransition {
                    from: *cur,
                    to: new_status,
                });
            }
        }
        let source_value = new_source
            .map(CreationSource::as_str)
            .map_or(Value::Null, |s| Value::Text(s.to_string()));
        tx.execute(
            "UPDATE pcp_records
                SET status = ?1,
                    creation_source = COALESCE(?2, creation_source),
                    updated_at = ?3
              WHERE signup_id = ?4",
            params![new_status.as_str(), source_value, now_i64, signup_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Mark one tier acknowledged. Per-tier: each tier acks independently.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn mark_ack(
        &self,
        signup_id: &str,
        tier: Tier,
        now_seconds: u64,
    ) -> StorageResult<()> {
        let now_i64 = to_i64(now_seconds, "now")?;
        self.vault.connection().execute(
            "UPDATE pcp_records
                SET is_download_acknowledged = 1, updated_at = ?1
              WHERE signup_id = ?2 AND tier = ?3",
            params![now_i64, signup_id, i64::from(tier)],
        )?;
        Ok(())
    }

    /// All rows for `signup_id`, ordered by tier.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn tiers_for_signup(&self, signup_id: &str) -> StorageResult<Vec<PcpRecord>> {
        read_signup_rows(self.vault.connection(), signup_id)
    }

    /// All tiers of the latest signup with at least one `Enrolled` row,
    /// or `None` if none. Latest = `MAX(orb_created_at)` across enrolled.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn latest_enrolled(&self) -> StorageResult<Option<Vec<PcpRecord>>> {
        let conn = self.vault.connection();
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

    /// All `(signup_id, tier)` pairs with pending ack. For cold-start
    /// ack retry.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn unacked_tiers(&self) -> StorageResult<Vec<(SignupId, Tier)>> {
        let conn = self.vault.connection();
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

fn to_i64(value: u64, label: &str) -> StorageResult<i64> {
    i64::try_from(value).map_err(|_| {
        StorageError::InvalidState(format!("{label} overflows i64: {value}"))
    })
}

fn nonneg_u64(row: &Row<'_, '_>, idx: usize, label: &str) -> StorageResult<u64> {
    u64::try_from(row.column_i64(idx))
        .map_err(|_| StorageError::InvalidState(format!("{label} negative")))
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

fn row_to_record(row: &Row<'_, '_>) -> StorageResult<PcpRecord> {
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
    Ok(PcpRecord {
        signup_id,
        tier,
        version,
        signup_reason,
        status,
        is_download_acknowledged: ack,
        creation_source,
        package_blob_cid: cid,
        orb_created_at: nonneg_u64(row, 8, "orb_created_at")?,
        created_at: nonneg_u64(row, 9, "created_at")?,
        updated_at: nonneg_u64(row, 10, "updated_at")?,
    })
}
