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
use crate::storage::types::{CreationSource, PackageStatus, PcpRecord, SignupId};

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

    /// Insert a signup's PCP with status `Downloaded`. A repeat download
    /// only merges the ack flag, so it can't rewind a row that has moved
    /// past `Downloaded`. Returns the stored package's content id.
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
        let stored_cid = conn.query_row(
            "INSERT INTO pcp_records (
                signup_id, version, signup_reason,
                status, is_download_acknowledged, creation_source,
                package_blob_cid, orb_created_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT (signup_id) DO UPDATE SET is_download_acknowledged =
                 is_download_acknowledged OR excluded.is_download_acknowledged
             RETURNING package_blob_cid",
            params![
                ingest.signup_id,
                ingest.version,
                ingest.signup_reason.map_or(Value::Null, Value::from),
                PackageStatus::Downloaded.as_str(),
                i64::from(u8::from(ingest.is_download_acknowledged)),
                ingest.creation_source.as_str(),
                cid.as_slice(),
                orb_i64,
                now_i64,
            ],
            |row| Ok(row.column_blob(0)),
        )?;
        tx.commit()?;
        to_cid(stored_cid)
    }

    /// Move a signup to `new_status`. Idempotent.
    ///
    /// # Errors
    ///
    /// [`StorageError::SignupNotFound`] if the signup doesn't exist;
    /// [`StorageError::IllegalTransition`] if the row cannot reach
    /// `new_status` per [`PackageStatus::can_transition_to`].
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
        let cur = read_record(conn, signup_id)?
            .ok_or(StorageError::SignupNotFound)?
            .status;
        if cur != new_status && !cur.can_transition_to(new_status) {
            return Err(StorageError::IllegalTransition {
                from: cur,
                to: new_status,
            });
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

    /// The record for `signup_id`, if any.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn get(&self, signup_id: &str) -> StorageResult<Option<PcpRecord>> {
        read_record(self.vault.connection(), signup_id)
    }

    /// Mark a signup's download acknowledged.
    ///
    /// # Errors
    ///
    /// [`StorageError::SignupNotFound`] if the signup doesn't exist.
    /// Database errors.
    pub fn mark_ack(&self, signup_id: &str, now_seconds: u64) -> StorageResult<()> {
        let now_i64 = to_i64(now_seconds, "now")?;
        let rows = self.vault.connection().execute(
            "UPDATE pcp_records
                SET is_download_acknowledged = 1, updated_at = ?1
              WHERE signup_id = ?2",
            params![now_i64, signup_id],
        )?;
        if rows == 0 {
            return Err(StorageError::SignupNotFound);
        }
        Ok(())
    }

    /// Signups with a pending download ack. For cold-start ack retry.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn unacked_signups(&self) -> StorageResult<Vec<SignupId>> {
        let conn = self.vault.connection();
        let mut stmt = conn.prepare(
            "SELECT signup_id FROM pcp_records WHERE is_download_acknowledged = 0",
        )?;
        let mut out = Vec::new();
        while let StepResult::Row(row) = stmt.step()? {
            out.push(row.column_text(0));
        }
        Ok(out)
    }
}

/// Arguments for [`OrbPcpStore::put_package`].
#[expect(missing_docs, reason = "fields mirror the pcp_records columns")]
pub struct PcpIngest<'a> {
    pub signup_id: &'a str,
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

fn read_record(conn: &Connection, signup_id: &str) -> StorageResult<Option<PcpRecord>> {
    let mut stmt = conn.prepare(
        "SELECT signup_id, version, signup_reason, status,
                is_download_acknowledged, creation_source, package_blob_cid,
                orb_created_at, created_at, updated_at
           FROM pcp_records
          WHERE signup_id = ?1",
    )?;
    stmt.bind_values(params![signup_id])?;
    match stmt.step()? {
        StepResult::Row(row) => Ok(Some(row_to_record(&row)?)),
        StepResult::Done => Ok(None),
    }
}

fn row_to_record(row: &Row<'_, '_>) -> StorageResult<PcpRecord> {
    Ok(PcpRecord {
        signup_id: row.column_text(0),
        version: row.column_text(1),
        signup_reason: (!row.is_column_null(2)).then(|| row.column_text(2)),
        status: PackageStatus::parse(&row.column_text(3))?,
        is_download_acknowledged: row.column_i64(4) != 0,
        creation_source: CreationSource::parse(&row.column_text(5))?,
        package_blob_cid: to_cid(row.column_blob(6))?,
        orb_created_at: nonneg_u64(row, 7, "orb_created_at")?,
        created_at: nonneg_u64(row, 8, "created_at")?,
        updated_at: nonneg_u64(row, 9, "updated_at")?,
    })
}

fn to_cid(bytes: Vec<u8>) -> StorageResult<[u8; 32]> {
    let len = bytes.len();
    <[u8; 32]>::try_from(bytes).map_err(|_| {
        StorageError::InvalidState(format!("package_blob_cid len {len} != 32"))
    })
}
