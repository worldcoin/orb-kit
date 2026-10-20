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
            signup_id                  TEXT    NOT NULL PRIMARY KEY,
            version                   TEXT    NOT NULL,
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
            FOREIGN KEY (package_blob_cid)
                REFERENCES blob_objects(content_id)
                ON DELETE RESTRICT
        );
        ",
    )
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;
    use std::path::Path;

    use walletkit_db::{compute_content_id, Connection};

    use super::ensure_schema;

    /// Frozen hash of `pcp_records` as stored in `sqlite_master`. Changing the
    /// DDL breaks this on purpose: existing vaults keep their original table,
    /// so any schema edit needs a migration, not a silent rewrite.
    const FROZEN_PCP_RECORDS_SQL_HASH: &str =
        "7ba9e8d1bf51c14e38b791af5fa5a71f2ab05f14994f4c976094f25ba44fbb86";

    #[test]
    fn pcp_records_schema_is_frozen() {
        let conn = Connection::open(Path::new(":memory:"), false).expect("open");
        ensure_schema(&conn).expect("schema");
        let sql = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'pcp_records'",
                &[],
                |row| Ok(row.column_text(0)),
            )
            .expect("pcp_records exists");
        let hash = compute_content_id(0, sql.as_bytes()).iter().fold(
            String::new(),
            |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            },
        );
        assert_eq!(hash, FROZEN_PCP_RECORDS_SQL_HASH);
    }
}
