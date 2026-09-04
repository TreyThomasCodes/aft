use std::path::Path;

use aft::blob_store::{BlobPlane, BlobStore, SemanticKey};
use aft::subc::blob_store::{BlobQuota, BoundBlobStore, BoundPutOutcome};
use aft::subc::BindTrust;

fn key() -> aft::blob_store::FullKey {
    SemanticKey::for_current(b"source", b"src/lib.rs", "model-a").full_key()
}

fn bound(storage: &Path, family: &str, trust: BindTrust, view: &str) -> BoundBlobStore {
    BoundBlobStore::new(
        BlobStore::open(storage, family, BlobPlane::Semantic).expect("open blob store"),
        trust,
        view,
    )
}

fn assert_untrusted_bind_class(trust: BindTrust) {
    let storage = tempfile::tempdir().expect("tempdir");
    let full_key = key();
    let mut owner = bound(storage.path(), "family-a", BindTrust::FirstParty, "view-a");
    assert!(matches!(
        owner
            .put("family-a", b"src/lib.rs", &full_key, b"payload")
            .expect("owner put"),
        BoundPutOutcome::Stored(_)
    ));

    let mut untrusted = bound(storage.path(), "family-a", trust, "view-a");
    assert_eq!(
        untrusted
            .put("family-a", b"src/lib.rs", &full_key, b"payload")
            .expect("untrusted put"),
        BoundPutOutcome::Denied
    );
    assert_eq!(
        untrusted.get("family-a", &full_key).expect("own read"),
        Some(b"payload".to_vec())
    );
    assert_eq!(
        untrusted.get("family-b", &full_key).expect("other read"),
        None
    );
    assert!(untrusted.allow_manifest_write("family-a", "view-a"));
    assert!(!untrusted.allow_manifest_write("family-a", "view-b"));
}

#[test]
fn mcp_bind_trust_gates_put_read_and_manifest_write() {
    // `mcp:*` route binds resolve to the untrusted capability class.
    assert_untrusted_bind_class(BindTrust::Untrusted);
}

#[test]
fn unverified_bind_trust_gates_put_read_and_manifest_write() {
    assert_untrusted_bind_class(BindTrust::Untrusted);
}

#[test]
fn federation_bind_trust_gates_put_read_and_manifest_write() {
    // `fed:*` route binds resolve to the untrusted capability class.
    assert_untrusted_bind_class(BindTrust::Untrusted);
}

#[test]
fn injected_value_coverage_refuses_byte_quota_and_requests_sweep() {
    let storage = tempfile::tempdir().expect("tempdir");
    let full_key = key();
    let mut store = BoundBlobStore::with_quota(
        BlobStore::open(storage.path(), "family-a", BlobPlane::Semantic).expect("open blob store"),
        BindTrust::FirstParty,
        "view-a",
        BlobQuota {
            payload_bytes: 6,
            rows: 10,
        },
    );

    assert_eq!(
        store
            .put("family-a", b"src/lib.rs", &full_key, b"payload")
            .expect("quota put"),
        BoundPutOutcome::QuotaExceeded
    );
    assert_eq!(store.failed_paths().collect::<Vec<_>>()[0].reason, "quota");
    assert_eq!(store.sweep_requests()[0].reason, "quota");
}

#[test]
fn injected_value_coverage_refuses_row_quota_and_requests_sweep() {
    let storage = tempfile::tempdir().expect("tempdir");
    let full_key = key();
    let mut store = BoundBlobStore::with_quota(
        BlobStore::open(storage.path(), "family-a", BlobPlane::Semantic).expect("open blob store"),
        BindTrust::FirstParty,
        "view-a",
        BlobQuota {
            payload_bytes: 100,
            rows: 0,
        },
    );

    assert_eq!(
        store
            .put("family-a", b"src/lib.rs", &full_key, b"payload")
            .expect("quota put"),
        BoundPutOutcome::QuotaExceeded
    );
    assert_eq!(store.failed_paths().collect::<Vec<_>>()[0].reason, "quota");
    assert_eq!(store.sweep_requests()[0].reason, "quota");
}
