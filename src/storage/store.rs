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

    /// Insert one tier of a signup with status `Downloaded`. Strict:
    /// errors if `(signup_id, tier)` already exists, so a re-download
    /// can't reset a row that has moved past `Downloaded`.
    ///
    /// # Errors
    ///
    /// [`StorageError::AlreadyExists`] if the row exists. Database
    /// errors; [`StorageError::InvalidState`] if a timestamp overflows
    /// `i64`.
    pub fn put_package(&self, ingest: &PcpIngest<'_>) -> StorageResult<[u8; 32]> {
        let now_i64 = to_i64(ingest.now_seconds, "now")?;
        let orb_i64 = to_i64(ingest.orb_created_at_seconds, "orb_created_at")?;
        let conn = self.vault.connection();
        let tx = conn.transaction()?;
        if read_existing_cid(conn, ingest.signup_id, ingest.tier)?.is_some() {
            return Err(StorageError::AlreadyExists);
        }
        let cid = blobs::put(
            conn,
            KIND_PCP_PACKAGE,
            ingest.encrypted_bytes,
            ingest.now_seconds,
        )?;
        tx.execute(
            "INSERT INTO pcp_records (
                signup_id, tier, version, signup_reason,
                status, is_download_acknowledged, creation_source,
                package_blob_cid, orb_created_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                ingest.signup_id,
                i64::from(ingest.tier),
                ingest.version,
                ingest.signup_reason.map_or(Value::Null, Value::from),
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
        let current = read_signup_rows(conn, signup_id)?;
        if current.is_empty() {
            return Err(StorageError::SignupNotFound);
        }
        for cur in current.iter().map(|r| r.status) {
            if cur != new_status && !cur.can_transition_to(new_status) {
                return Err(StorageError::IllegalTransition {
                    from: cur,
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

    /// All rows for `signup_id`, ordered by tier.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn tiers_for_signup(&self, signup_id: &str) -> StorageResult<Vec<PcpRecord>> {
        read_signup_rows(self.vault.connection(), signup_id)
    }

    /// Mark one tier acknowledged. Per-tier: each tier acks independently.
    ///
    /// # Errors
    ///
    /// [`StorageError::SignupNotFound`] if no row matches
    /// `(signup_id, tier)`. Database errors.
    pub fn mark_ack(
        &self,
        signup_id: &str,
        tier: Tier,
        now_seconds: u64,
    ) -> StorageResult<()> {
        let now_i64 = to_i64(now_seconds, "now")?;
        let rows = self.vault.connection().execute(
            "UPDATE pcp_records
                SET is_download_acknowledged = 1, updated_at = ?1
              WHERE signup_id = ?2 AND tier = ?3",
            params![now_i64, signup_id, i64::from(tier)],
        )?;
        if rows == 0 {
            return Err(StorageError::SignupNotFound);
        }
        Ok(())
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
            out.push((row.column_text(0), tier_at(&row, 1)?));
        }
        Ok(out)
    }
}

/// Arguments for [`OrbPcpStore::put_package`].
#[expect(missing_docs, reason = "fields mirror the pcp_records columns")]
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

fn read_existing_cid(
    conn: &Connection,
    signup_id: &str,
    tier: Tier,
) -> StorageResult<Option<[u8; 32]>> {
    let cid_bytes: Option<Vec<u8>> = conn.query_row_optional(
        "SELECT package_blob_cid FROM pcp_records
          WHERE signup_id = ?1 AND tier = ?2",
        params![signup_id, i64::from(tier)],
        |row| Ok(row.column_blob(0)),
    )?;
    cid_bytes.map(to_cid).transpose()
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
    Ok(PcpRecord {
        signup_id: row.column_text(0),
        tier: tier_at(row, 1)?,
        version: row.column_text(2),
        signup_reason: (!row.is_column_null(3)).then(|| row.column_text(3)),
        status: PackageStatus::parse(&row.column_text(4))?,
        is_download_acknowledged: row.column_i64(5) != 0,
        creation_source: CreationSource::parse(&row.column_text(6))?,
        package_blob_cid: to_cid(row.column_blob(7))?,
        orb_created_at: nonneg_u64(row, 8, "orb_created_at")?,
        created_at: nonneg_u64(row, 9, "created_at")?,
        updated_at: nonneg_u64(row, 10, "updated_at")?,
    })
}

fn tier_at(row: &Row<'_, '_>, idx: usize) -> StorageResult<Tier> {
    let tier = row.column_i64(idx);
    u8::try_from(tier)
        .map_err(|_| StorageError::InvalidState(format!("tier out of range: {tier}")))
}

fn to_cid(bytes: Vec<u8>) -> StorageResult<[u8; 32]> {
    let len = bytes.len();
    <[u8; 32]>::try_from(bytes).map_err(|_| {
        StorageError::InvalidState(format!("package_blob_cid len {len} != 32"))
    })
}
