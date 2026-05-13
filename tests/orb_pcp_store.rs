//! Integration tests for `OrbPcpStore` against a real walletkit-db vault.
//!
//! Uses in-memory stubs of `Keystore` and `AtomicBlobStore` so the tests
//! don't need a host platform. Each test opens its own temp dir so they
//! can run in parallel.

#![allow(clippy::missing_panics_doc)]

use std::collections::HashMap;
use std::sync::Mutex;

use orb_kit::storage::{
    paths::{LOCK_FILENAME, VAULT_FILENAME},
    store::{PcpIngest, Vault},
    CreationSource, PackageStatus, StorageError,
};
use walletkit_db::{AtomicBlobStore, Keystore, Lock, StoreResult};

const TEST_TIME: u64 = 1_700_000_000;
const SAMPLE_PCP_BYTES: &[u8] = b"fake-encrypted-pcp-bytes";

// ---- Host stubs -----------------------------------------------------------

struct XorKeystore {
    pad: [u8; 32],
}

impl Keystore for XorKeystore {
    fn seal(&self, _ad: Vec<u8>, plaintext: Vec<u8>) -> StoreResult<Vec<u8>> {
        Ok(plaintext
            .into_iter()
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

fn open_vault(dir: &tempfile::TempDir) -> Vault {
    let keystore = XorKeystore { pad: [0x5A; 32] };
    let blob_store = InMemoryBlobs::default();
    let lock = Lock::open(&dir.path().join(LOCK_FILENAME)).expect("open lock");
    Vault::open(
        &dir.path().join(VAULT_FILENAME),
        TEST_TIME,
        lock,
        &keystore,
        &blob_store,
    )
    .expect("open vault")
}

fn ingest(signup_id: &str, tier: u8) -> PcpIngest<'_> {
    PcpIngest {
        signup_id,
        tier,
        version: "V3_0",
        signup_reason: None,
        creation_source: CreationSource::UserCentricEnrollment,
        is_download_acknowledged: false,
        encrypted_bytes: SAMPLE_PCP_BYTES,
        orb_created_at_seconds: TEST_TIME,
        now_seconds: TEST_TIME + 1,
    }
}

// ---- Tests ----------------------------------------------------------------

#[test]
fn init_meta_is_idempotent() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta first");
    store.init_meta(TEST_TIME).expect("init_meta second");
}

#[test]
fn put_package_writes_row_and_blob() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta");

    let cid = store.put_package(&ingest("abc-12345", 0)).expect("put");
    assert_eq!(cid.len(), 32);

    let rows = store
        .tiers_for_signup("abc-12345")
        .expect("tiers_for_signup");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].signup_id, "abc-12345");
    assert_eq!(rows[0].tier, 0);
    assert_eq!(rows[0].status, PackageStatus::Downloaded);
    assert_eq!(
        rows[0].creation_source,
        CreationSource::UserCentricEnrollment
    );
    assert!(!rows[0].is_download_acknowledged);
}

#[test]
fn status_transitions_apply_to_all_tiers() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta");

    for tier in 0..3 {
        store
            .put_package(&ingest("def-67890", tier))
            .expect("put tier");
    }
    store
        .update_status(
            "def-67890",
            PackageStatus::EnrollmentRequestInitiated,
            None,
            TEST_TIME + 10,
        )
        .expect("transition");

    let rows = store.tiers_for_signup("def-67890").expect("tiers");
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_eq!(row.status, PackageStatus::EnrollmentRequestInitiated);
    }
}

#[test]
fn illegal_transition_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta");
    store.put_package(&ingest("xyz", 0)).expect("put");

    let err = store
        .update_status("xyz", PackageStatus::Enrolled, None, TEST_TIME + 1)
        .expect_err("Downloaded -> Enrolled is illegal");
    assert!(matches!(err, StorageError::InvalidState(_)));
}

#[test]
fn mark_ack_scopes_to_one_tier() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta");

    for tier in 0..2 {
        store.put_package(&ingest("sig-1", tier)).expect("put tier");
    }
    store
        .mark_ack("sig-1", 0, TEST_TIME + 5)
        .expect("ack tier 0");

    let rows = store.tiers_for_signup("sig-1").expect("tiers");
    assert!(rows[0].is_download_acknowledged);
    assert!(!rows[1].is_download_acknowledged);
    assert_eq!(
        store.unacked_tiers().expect("unacked"),
        vec![("sig-1".to_string(), 1)],
    );
}

#[test]
fn latest_enrolled_returns_most_recent_enrolled_signup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta");

    // Two signups, both eventually Enrolled. Newer one has a later orb_created_at.
    for (signup, orb_ts) in [("older", TEST_TIME), ("newer", TEST_TIME + 1_000_000)] {
        let mut ing = ingest(signup, 0);
        ing.orb_created_at_seconds = orb_ts;
        store.put_package(&ing).expect("put");
        for status in [
            PackageStatus::EnrollmentRequestInitiated,
            PackageStatus::EnrollmentRequestSuccess,
            PackageStatus::Enrolled,
        ] {
            store
                .update_status(signup, status, None, orb_ts + 1)
                .expect("advance");
        }
    }

    let latest = store
        .latest_enrolled()
        .expect("latest_enrolled")
        .expect("at least one enrolled");
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].signup_id, "newer");
}

#[test]
fn latest_enrolled_is_none_when_no_enrolled_signup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let vault = open_vault(&dir);
    let store = vault.store();
    store.init_meta(TEST_TIME).expect("init_meta");
    store
        .put_package(&ingest("downloaded-only", 0))
        .expect("put");

    assert!(store.latest_enrolled().expect("latest_enrolled").is_none());
}

#[test]
fn state_persists_across_reopens() {
    let dir = tempfile::tempdir().expect("temp dir");
    let keystore = XorKeystore { pad: [0xC3; 32] };
    let blob_store = InMemoryBlobs::default();

    {
        let lock = Lock::open(&dir.path().join(LOCK_FILENAME)).expect("open lock");
        let vault = Vault::open(
            &dir.path().join(VAULT_FILENAME),
            TEST_TIME,
            lock,
            &keystore,
            &blob_store,
        )
        .expect("open vault");
        let store = vault.store();
        store.init_meta(TEST_TIME).expect("init_meta");
        store.put_package(&ingest("persist-1", 0)).expect("put");
    }

    let lock = Lock::open(&dir.path().join(LOCK_FILENAME)).expect("reopen lock");
    let vault = Vault::open(
        &dir.path().join(VAULT_FILENAME),
        TEST_TIME,
        lock,
        &keystore,
        &blob_store,
    )
    .expect("reopen vault");
    let store = vault.store();
    let rows = store.tiers_for_signup("persist-1").expect("tiers");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, PackageStatus::Downloaded);
}
