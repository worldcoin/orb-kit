//! Integration tests for `OrbPcpStore` against a real walletkit-db vault.
//!
//! Uses in-memory stubs of `Keystore` and `AtomicBlobStore` so the tests
//! don't need a host platform. Each test opens its own temp dir so they
//! can run in parallel.

use std::collections::HashMap;
use std::sync::Mutex;

use orb_kit::storage::{
    paths::{LOCK_FILENAME, VAULT_FILENAME},
    CreationSource, OrbPcpStore, PackageStatus, PcpIngest, PcpRecord, StorageError,
};
use walletkit_db::{AtomicBlobStore, Keystore, Lock, StoreResult};

const TEST_TIME: u64 = 1_700_000_000;
const SAMPLE_PCP_BYTES: &[u8] = b"fake-encrypted-pcp-bytes";

struct XorKeystore {
    pad: [u8; 32],
}

impl Keystore for XorKeystore {
    fn seal(&self, _ad: &[u8], plaintext: &[u8]) -> StoreResult<Vec<u8>> {
        Ok(plaintext
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ self.pad[i % 32])
            .collect())
    }
    fn open_sealed(&self, _ad: Vec<u8>, ciphertext: Vec<u8>) -> StoreResult<Vec<u8>> {
        Ok(ciphertext
            .into_iter()
            .enumerate()
            .map(|(i, b)| b ^ self.pad[i % 32])
            .collect())
    }
}

#[derive(Default)]
struct InMemoryBlobs {
    inner: Mutex<HashMap<String, Vec<u8>>>,
}

impl AtomicBlobStore for InMemoryBlobs {
    fn read(&self, path: String) -> StoreResult<Option<Vec<u8>>> {
        Ok(self.inner.lock().unwrap().get(&path).cloned())
    }
    fn write_atomic(&self, path: String, bytes: Vec<u8>) -> StoreResult<()> {
        self.inner.lock().unwrap().insert(path, bytes);
        Ok(())
    }
    fn delete(&self, path: String) -> StoreResult<()> {
        self.inner.lock().unwrap().remove(&path);
        Ok(())
    }
}

fn open_store(dir: &tempfile::TempDir) -> OrbPcpStore {
    let keystore = XorKeystore { pad: [0x5A; 32] };
    let blob_store = InMemoryBlobs::default();
    let lock = Lock::open(&dir.path().join(LOCK_FILENAME)).expect("open lock");
    OrbPcpStore::open(
        &dir.path().join(VAULT_FILENAME),
        TEST_TIME,
        &lock,
        &keystore,
        &blob_store,
    )
    .expect("open store")
}

fn ingest(signup_id: &str) -> PcpIngest<'_> {
    PcpIngest {
        signup_id,
        version: "V3_0",
        signup_reason: None,
        creation_source: CreationSource::UserCentricEnrollment,
        is_download_acknowledged: false,
        encrypted_bytes: SAMPLE_PCP_BYTES,
        orb_created_at_seconds: TEST_TIME,
        now_seconds: TEST_TIME + 1,
    }
}

fn get(store: &OrbPcpStore, signup_id: &str) -> PcpRecord {
    store.get(signup_id).expect("get").expect("row exists")
}

#[test]
fn put_package_writes_row_and_blob() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);

    let cid = store.put_package(&ingest("abc-12345")).expect("put");

    let row = get(&store, "abc-12345");
    assert_eq!(row.package_blob_cid, cid);
    assert_eq!(row.status, PackageStatus::Downloaded);
    assert_eq!(row.creation_source, CreationSource::UserCentricEnrollment);
    assert!(!row.is_download_acknowledged);
}

#[test]
fn status_transition_applies() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);
    store.put_package(&ingest("def-67890")).expect("put");

    store
        .update_status(
            "def-67890",
            PackageStatus::EnrollmentRequestInitiated,
            None,
            TEST_TIME + 10,
        )
        .expect("transition");

    assert_eq!(
        get(&store, "def-67890").status,
        PackageStatus::EnrollmentRequestInitiated
    );
}

#[test]
fn illegal_transition_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);
    store.put_package(&ingest("xyz")).expect("put");

    let err = store
        .update_status("xyz", PackageStatus::Enrolled, None, TEST_TIME + 1)
        .expect_err("Downloaded -> Enrolled is illegal");
    assert!(matches!(
        err,
        StorageError::IllegalTransition {
            from: PackageStatus::Downloaded,
            to: PackageStatus::Enrolled,
        }
    ));
}

#[test]
fn update_status_on_unknown_signup_returns_not_found() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);

    let err = store
        .update_status(
            "no-such-signup",
            PackageStatus::EnrollmentRequestInitiated,
            None,
            TEST_TIME + 1,
        )
        .expect_err("no rows should be rejected");
    assert!(matches!(err, StorageError::SignupNotFound));
}

#[test]
fn put_package_errors_when_row_already_exists() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);

    store.put_package(&ingest("dup")).expect("first put");
    store
        .update_status(
            "dup",
            PackageStatus::EnrollmentRequestInitiated,
            None,
            TEST_TIME + 2,
        )
        .expect("advance");
    let err = store
        .put_package(&ingest("dup"))
        .expect_err("second put on same signup should error");
    assert!(matches!(err, StorageError::AlreadyExists));

    assert_eq!(
        get(&store, "dup").status,
        PackageStatus::EnrollmentRequestInitiated
    );
}

#[test]
fn mark_ack_clears_pending_ack() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);
    store.put_package(&ingest("sig-1")).expect("put");
    store.put_package(&ingest("sig-2")).expect("put");

    store.mark_ack("sig-1", TEST_TIME + 5).expect("ack");

    assert!(get(&store, "sig-1").is_download_acknowledged);
    assert_eq!(
        store.unacked_signups().expect("unacked"),
        vec!["sig-2".to_string()],
    );
}

#[test]
fn mark_ack_errors_when_row_not_found() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(&dir);

    let err = store
        .mark_ack("no-such-signup", TEST_TIME + 5)
        .expect_err("ack on missing row should error");
    assert!(matches!(err, StorageError::SignupNotFound));
}

#[test]
fn state_persists_across_reopens() {
    let dir = tempfile::tempdir().expect("temp dir");
    let keystore = XorKeystore { pad: [0xC3; 32] };
    let blob_store = InMemoryBlobs::default();

    {
        let lock = Lock::open(&dir.path().join(LOCK_FILENAME)).expect("open lock");
        let store = OrbPcpStore::open(
            &dir.path().join(VAULT_FILENAME),
            TEST_TIME,
            &lock,
            &keystore,
            &blob_store,
        )
        .expect("open store");
        store.put_package(&ingest("persist-1")).expect("put");
    }

    let lock = Lock::open(&dir.path().join(LOCK_FILENAME)).expect("reopen lock");
    let store = OrbPcpStore::open(
        &dir.path().join(VAULT_FILENAME),
        TEST_TIME,
        &lock,
        &keystore,
        &blob_store,
    )
    .expect("reopen store");
    assert_eq!(get(&store, "persist-1").status, PackageStatus::Downloaded);
}
