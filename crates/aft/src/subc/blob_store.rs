//! Bind-scoped access to immutable family blob stores.
//!
//! `BlobStore` owns SQLite persistence only. This facade is the daemon entry
//! point: it applies the authenticated bind capability before every operation.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::blob_store::{BlobStore, BlobStoreError, FullKey, PutReport};

use super::BindTrust;

pub const FAMILY_QUOTA_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const FAMILY_QUOTA_ROWS: u64 = 2_000_000;
pub const PIN_TTL: Duration = Duration::from_secs(30 * 60);
pub const BLOB_AGE_FLOOR: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobQuota {
    pub payload_bytes: u64,
    pub rows: u64,
}

impl Default for BlobQuota {
    fn default() -> Self {
        Self {
            payload_bytes: FAMILY_QUOTA_BYTES,
            rows: FAMILY_QUOTA_ROWS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundPutOutcome {
    Stored(PutReport),
    Denied,
    QuotaExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedPath {
    pub path: Vec<u8>,
    pub reason: &'static str,
}

/// A request for the sweep owner to reclaim unreferenced family blobs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SweepRequest {
    pub family: String,
    pub reason: &'static str,
}

/// The only bind-aware blob-store API used by daemon code.
#[derive(Debug)]
pub struct BoundBlobStore {
    store: BlobStore,
    trust: BindTrust,
    family: String,
    view: String,
    quota: BlobQuota,
    failed_paths: BTreeMap<Vec<u8>, FailedPath>,
    sweep_requests: Vec<SweepRequest>,
}

impl BoundBlobStore {
    pub fn new(store: BlobStore, trust: BindTrust, view: impl Into<String>) -> Self {
        Self::with_quota(store, trust, view, BlobQuota::default())
    }

    /// Test-only limits cover quota paths without requiring multi-gigabyte
    /// fixtures. Production callers must use [`Self::new`].
    pub fn with_quota(
        store: BlobStore,
        trust: BindTrust,
        view: impl Into<String>,
        quota: BlobQuota,
    ) -> Self {
        let family = store.artifact_key().to_owned();
        Self {
            store,
            trust,
            family,
            view: view.into(),
            quota,
            failed_paths: BTreeMap::new(),
            sweep_requests: Vec::new(),
        }
    }

    pub fn put(
        &mut self,
        family: &str,
        path: &[u8],
        full_key: &FullKey,
        payload: &[u8],
    ) -> Result<BoundPutOutcome, BlobStoreError> {
        if !self.is_first_party() || family != self.family {
            return Ok(BoundPutOutcome::Denied);
        }
        // Existing immutable rows are reused and consume no additional quota.
        // Only a new payload can cross the family limits.
        let is_new = self.store.get(full_key)?.is_none();
        let usage = self.store.usage()?;
        if is_new
            && (usage.rows.saturating_add(1) > self.quota.rows
                || usage.payload_bytes.saturating_add(payload.len() as u64)
                    > self.quota.payload_bytes)
        {
            self.failed_paths.insert(
                path.to_vec(),
                FailedPath {
                    path: path.to_vec(),
                    reason: "quota",
                },
            );
            self.sweep_requests.push(SweepRequest {
                family: self.family.clone(),
                reason: "quota",
            });
            return Ok(BoundPutOutcome::QuotaExceeded);
        }
        self.store
            .put(full_key, payload)
            .map(BoundPutOutcome::Stored)
    }

    pub fn get(&self, family: &str, full_key: &FullKey) -> Result<Option<Vec<u8>>, BlobStoreError> {
        if family != self.family {
            return Ok(None);
        }
        self.store.get(full_key)
    }

    /// Records that a bind may write a manifest only for its own view. Manifest
    /// persistence is supplied by the publication slice; this gate is kept at
    /// the blob-store boundary so its authorization cannot be bypassed there.
    pub fn allow_manifest_write(&self, family: &str, view: &str) -> bool {
        family == self.family && view == self.view
    }

    pub fn failed_paths(&self) -> impl Iterator<Item = &FailedPath> {
        self.failed_paths.values()
    }

    pub fn sweep_requests(&self) -> &[SweepRequest] {
        &self.sweep_requests
    }

    fn is_first_party(&self) -> bool {
        matches!(self.trust, BindTrust::FirstParty)
    }
}
