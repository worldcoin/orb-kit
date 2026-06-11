//! DDL for `OrbKit`'s PCP vault. Idempotent.

use walletkit_db::{blobs, Connection, DbResult};

/// Create the PCP tables if absent. Used as the `ensure_schema` callback
/// passed to `walletkit_db::Vault::open`.
///
/// # Errors
///
/// Returns the underlying `DbError` if any statement fails.
pub fn ensure_schema(conn: &Connection) -> DbResult<()> {
    blobs::ensure_schema(conn)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS pcp_records (
            signup_id                  TEXT    NOT NULL,
            tier                       INTEGER NOT NULL DEFAULT 0,
            version                    TEXT    NOT NULL,
            signup_reason              TEXT,
            status                     TEXT    NOT NULL
                                       CHECK (status IN (
                                           'Downloaded',
                                           'EnrollmentRequestInitiated',
                                           'EnrollmentRequestSuccess',
                                           'Enrolled',
                                           'EnrollmentAbandoned',
                                           'EnrollmentFailed',
                                           'Unverified'
                                       )),
            is_download_acknowledged   INTEGER NOT NULL DEFAULT 0
                                       CHECK (is_download_acknowledged IN (0, 1)),
            creation_source            TEXT    NOT NULL
                                       CHECK (creation_source IN (
                                           'UserCentricEnrollment',
                                           'UserCentricReEnrollment',
                                           'ReAuthentication',
                                           'Sync',
                                           'CredentialRecovery'
                                       )),
            package_blob_cid           BLOB    NOT NULL
                                       CHECK (length(package_blob_cid) = 32),
            orb_created_at             INTEGER NOT NULL,
            created_at                 INTEGER NOT NULL,
            updated_at                 INTEGER NOT NULL,
            PRIMARY KEY (signup_id, tier),
            FOREIGN KEY (package_blob_cid)
                REFERENCES blob_objects(content_id)
                ON DELETE RESTRICT
        );

        CREATE INDEX IF NOT EXISTS idx_pcp_by_status_orb_created
            ON pcp_records (status, orb_created_at DESC);
        ",
    )
}
