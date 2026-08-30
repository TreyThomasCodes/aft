use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::config::GhReadConfig;
use crate::db::github_read_cache::{
    evict_hard_expired_github_read_cache_entries, invalidate_github_read_cache_resource,
    lookup_github_read_cache_entry, upsert_github_read_cache_entry, GithubReadCacheEntry,
    GithubReadCacheKey, GithubReadResourceKind,
};

use super::attachments::{
    download_github_image_attachments, GithubImageAttachment, GithubImageDownloader,
};
use super::fetch::{GithubFetchRequest, GithubFetcher, GithubReadError};
use super::render::render_document;
use super::resource::{parse_resource, GithubResource, GithubResourceKind, InvalidGithubResource};

/// Fresh GitHub renders are served without a new network request for this long.
/// A short window reduces redundant `gh` calls during a tool turn while still
/// making active issue threads responsive.
pub const GITHUB_READ_CACHE_SOFT_TTL_MS: i64 = 60_000;
/// Cache rows reaching this age are evicted before a caller refetches. The
/// larger hard window lets stale-while-revalidate survive a temporary GitHub
/// failure without serving content indefinitely.
pub const GITHUB_READ_CACHE_HARD_TTL_MS: i64 = 15 * 60_000;

/// Clock seam used to make freshness boundaries deterministic in tests.
pub trait GithubReadClock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Production wall clock for cache timestamps.
#[derive(Default)]
pub struct SystemGithubReadClock;

impl GithubReadClock for SystemGithubReadClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX)
    }
}

/// A cache persistence seam. The production implementation stores canonical
/// text in AFT's existing `aft.db`; tests can use an in-memory fixture store.
pub trait GithubReadCacheStore: Send + Sync {
    fn lookup(&self, key: &GithubReadCacheKey) -> Result<Option<GithubReadCacheEntry>, String>;
    fn upsert(
        &self,
        key: &GithubReadCacheKey,
        canonical_text: &str,
        fetched_at_ms: i64,
    ) -> Result<(), String>;
    fn evict_hard_expired_at(&self, cutoff_ms: i64) -> Result<usize, String>;
    fn invalidate(
        &self,
        kind: GithubResourceKind,
        repository: &str,
        number: u64,
        authentication_identity: Option<&str>,
    ) -> Result<usize, String>;
}

/// `aft.db` implementation of the GitHub read cache seam.
#[derive(Clone, Debug)]
pub struct SqliteGithubReadCacheStore {
    database_path: PathBuf,
}

impl SqliteGithubReadCacheStore {
    pub fn new(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
        }
    }

    fn connection(&self) -> Result<rusqlite::Connection, String> {
        crate::db::open(&self.database_path)
            .map_err(|error| format!("failed to open GitHub read cache: {error}"))
    }
}

impl GithubReadCacheStore for SqliteGithubReadCacheStore {
    fn lookup(&self, key: &GithubReadCacheKey) -> Result<Option<GithubReadCacheEntry>, String> {
        let connection = self.connection()?;
        lookup_github_read_cache_entry(&connection, key)
            .map_err(|error| format!("failed to look up GitHub read cache: {error}"))
    }

    fn upsert(
        &self,
        key: &GithubReadCacheKey,
        canonical_text: &str,
        fetched_at_ms: i64,
    ) -> Result<(), String> {
        let connection = self.connection()?;
        upsert_github_read_cache_entry(&connection, key, canonical_text, fetched_at_ms)
            .map_err(|error| format!("failed to update GitHub read cache: {error}"))
    }

    fn evict_hard_expired_at(&self, cutoff_ms: i64) -> Result<usize, String> {
        let connection = self.connection()?;
        evict_hard_expired_github_read_cache_entries(&connection, cutoff_ms)
            .map_err(|error| format!("failed to evict GitHub read cache: {error}"))
    }

    fn invalidate(
        &self,
        kind: GithubResourceKind,
        repository: &str,
        number: u64,
        authentication_identity: Option<&str>,
    ) -> Result<usize, String> {
        let number = i64::try_from(number)
            .map_err(|_| "GitHub resource number exceeds cache storage range".to_string())?;
        let connection = self.connection()?;
        invalidate_github_read_cache_resource(
            &connection,
            database_kind(kind),
            repository,
            number,
            authentication_identity,
        )
        .map_err(|error| format!("failed to invalidate GitHub read cache: {error}"))
    }
}

/// Request context that the read integration supplies before entering a
/// deferred fetch. An absent capability is intentionally different from an
/// inferred one: only `Some(true)` permits image downloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GithubReadRequest {
    pub resource: GithubResource,
    pub working_directory: PathBuf,
    pub effective_authentication_identity: String,
    pub vision_capability: Option<bool>,
}

impl GithubReadRequest {
    pub fn parse(
        resource: &str,
        working_directory: impl Into<PathBuf>,
        effective_authentication_identity: impl Into<String>,
        vision_capability: Option<bool>,
    ) -> Result<Self, InvalidGithubResource> {
        Ok(Self {
            resource: parse_resource(resource)?,
            working_directory: working_directory.into(),
            effective_authentication_identity: effective_authentication_identity.into(),
            vision_capability,
        })
    }
}

/// Selector applied only after the complete canonical document is rendered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GithubReadSelector {
    WholeDocument,
    LineRange {
        start_line: usize,
        end_line: Option<usize>,
        limit: usize,
    },
    ByteOffset {
        offset: usize,
        limit: Option<usize>,
    },
}

impl Default for GithubReadSelector {
    fn default() -> Self {
        Self::WholeDocument
    }
}

/// Freshness of the text that satisfied a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GithubReadFreshness {
    Fetched,
    FreshCache,
    /// The cache row is older than the soft TTL. A single background worker has
    /// been scheduled (or was already scheduled) for the same exact cache key.
    StaleCacheRefreshing,
}

impl GithubReadFreshness {
    /// Agent-visible freshness context for a stale-while-revalidate response.
    /// Kept outside canonical text so cache state cannot alter rendered bytes.
    pub const fn note(self) -> Option<&'static str> {
        match self {
            Self::StaleCacheRefreshing => {
                Some("Cached GitHub data is stale; a background refresh is in progress.")
            }
            Self::Fetched | Self::FreshCache => None,
        }
    }
}

/// A completed GitHub read ready for the transport-specific response adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GithubReadCompletion {
    pub content: String,
    pub total_lines: usize,
    pub freshness: GithubReadFreshness,
    pub attachments: Vec<GithubImageAttachment>,
}

/// Handle for a fetch or attachment task that is running away from the request
/// loop. Poll it from `PendingResponse`; never wait on it in standalone input
/// handling.
pub struct GithubReadDeferred {
    receiver: mpsc::Receiver<Result<GithubReadCompletion, GithubReadError>>,
}

impl GithubReadDeferred {
    pub fn try_complete(&self) -> Option<Result<GithubReadCompletion, GithubReadError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(GithubReadError::FetchFailed(
                "GitHub read worker stopped before completing".to_string(),
            ))),
        }
    }
}

/// The initial outcome for a GitHub read. A cache-only, text-only response is
/// immediate; every path that can call `gh` or download an image is deferred.
pub enum GithubReadStart {
    Immediate(GithubReadCompletion),
    Deferred(GithubReadDeferred),
}

#[derive(Default)]
struct GithubReadEngineState {
    aliases: BTreeMap<ShortResourceAlias, String>,
    refreshes: BTreeSet<CacheSlot>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ShortResourceAlias {
    kind: GithubResourceKind,
    number: u64,
    working_directory: PathBuf,
    authentication_identity_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CacheSlot {
    kind: GithubResourceKind,
    repository: String,
    number: u64,
    authentication_identity_hash: [u8; 32],
}

/// Coordinates durable cache lookup, stale refresh single-flight, structured
/// fetches, rendering, and deferred attachments for both issue and PR reads.
pub struct GithubReadEngine {
    cache: Arc<dyn GithubReadCacheStore>,
    fetcher: Arc<dyn GithubFetcher>,
    image_downloader: Arc<dyn GithubImageDownloader>,
    clock: Arc<dyn GithubReadClock>,
    state: Arc<Mutex<GithubReadEngineState>>,
}

impl GithubReadEngine {
    pub fn new(
        cache: Arc<dyn GithubReadCacheStore>,
        fetcher: Arc<dyn GithubFetcher>,
        image_downloader: Arc<dyn GithubImageDownloader>,
        clock: Arc<dyn GithubReadClock>,
    ) -> Self {
        Self {
            cache,
            fetcher,
            image_downloader,
            clock,
            state: Arc::new(Mutex::new(GithubReadEngineState::default())),
        }
    }

    /// Begin a resource-string read with strict parser validation.
    pub fn start_resource(
        &self,
        gh_read: &GhReadConfig,
        resource: &str,
        working_directory: impl Into<PathBuf>,
        effective_authentication_identity: impl Into<String>,
        vision_capability: Option<bool>,
        selector: GithubReadSelector,
    ) -> Result<GithubReadStart, GithubReadError> {
        self.require_enabled(gh_read)?;
        let request = GithubReadRequest::parse(
            resource,
            working_directory,
            effective_authentication_identity,
            vision_capability,
        )
        .map_err(|error| GithubReadError::invalid_resource(error.to_string()))?;
        self.start(gh_read, request, selector)
    }

    /// Start a read without blocking on GitHub or an image host.
    pub fn start(
        &self,
        gh_read: &GhReadConfig,
        request: GithubReadRequest,
        selector: GithubReadSelector,
    ) -> Result<GithubReadStart, GithubReadError> {
        self.require_enabled(gh_read)?;
        let now_ms = self.clock.now_ms();
        let Some((slot, key)) = self.cache_key_for_request(&request)? else {
            return Ok(self.defer_fetch(request, selector));
        };
        let entry = self.cache.lookup(&key).map_err(cache_failure)?;
        let Some(entry) = entry else {
            return Ok(self.defer_fetch(request, selector));
        };
        let age_ms = now_ms.saturating_sub(entry.fetched_at_ms);
        if age_ms < GITHUB_READ_CACHE_SOFT_TTL_MS {
            return Ok(self.complete_or_defer_attachments(
                request,
                selector,
                entry.canonical_text,
                GithubReadFreshness::FreshCache,
            ));
        }
        if age_ms < GITHUB_READ_CACHE_HARD_TTL_MS {
            self.schedule_background_refresh(request.clone(), slot);
            return Ok(self.complete_or_defer_attachments(
                request,
                selector,
                entry.canonical_text,
                GithubReadFreshness::StaleCacheRefreshing,
            ));
        }

        // Hard-expired rows are evicted before the deferred refetch. Failed
        // background refreshes never take this path, so they retain the prior
        // stale row until an actual hard-TTL request arrives.
        self.cache
            .evict_hard_expired_at(now_ms.saturating_sub(GITHUB_READ_CACHE_HARD_TTL_MS))
            .map_err(cache_failure)?;
        Ok(self.defer_fetch(request, selector))
    }

    /// Invalidate the exact resource after a successful structured `gh` mutation.
    /// Passing no identity conservatively removes every principal's cache row.
    pub fn invalidate(
        &self,
        kind: GithubResourceKind,
        resolved_repository: &str,
        number: u64,
        effective_authentication_identity: Option<&str>,
    ) -> Result<usize, GithubReadError> {
        self.cache
            .invalidate(
                kind,
                resolved_repository,
                number,
                effective_authentication_identity,
            )
            .map_err(cache_failure)
    }

    fn require_enabled(&self, gh_read: &GhReadConfig) -> Result<(), GithubReadError> {
        if gh_read.enabled {
            Ok(())
        } else {
            Err(GithubReadError::GithubReadDisabled)
        }
    }

    /// Deterministic seam for asserting stale refresh single-flight completion.
    pub fn refresh_in_flight_for_test(&self) -> usize {
        self.state.lock().refreshes.len()
    }

    fn cache_key_for_request(
        &self,
        request: &GithubReadRequest,
    ) -> Result<Option<(CacheSlot, GithubReadCacheKey)>, GithubReadError> {
        let repository = match &request.resource.repository {
            Some(repository) => Some(repository.clone()),
            None => self
                .state
                .lock()
                .aliases
                .get(&short_alias(request))
                .cloned(),
        };
        let Some(repository) = repository else {
            return Ok(None);
        };
        let slot = cache_slot(
            &request.resource,
            &repository,
            &request.effective_authentication_identity,
        );
        let key = github_cache_key(&slot, &request.effective_authentication_identity)?;
        Ok(Some((slot, key)))
    }

    fn defer_fetch(
        &self,
        request: GithubReadRequest,
        selector: GithubReadSelector,
    ) -> GithubReadStart {
        let cache = Arc::clone(&self.cache);
        let fetcher = Arc::clone(&self.fetcher);
        let downloader = Arc::clone(&self.image_downloader);
        let clock = Arc::clone(&self.clock);
        let state = Arc::clone(&self.state);
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = fetch_render_store(
                &request,
                cache.as_ref(),
                fetcher.as_ref(),
                clock.as_ref(),
                &state,
            )
            .and_then(|canonical_text| {
                complete_with_optional_attachments(
                    &request,
                    selector,
                    canonical_text,
                    GithubReadFreshness::Fetched,
                    downloader.as_ref(),
                )
            });
            let _ = sender.send(result);
        });
        GithubReadStart::Deferred(GithubReadDeferred { receiver })
    }

    fn complete_or_defer_attachments(
        &self,
        request: GithubReadRequest,
        selector: GithubReadSelector,
        canonical_text: String,
        freshness: GithubReadFreshness,
    ) -> GithubReadStart {
        if request.vision_capability != Some(true) {
            return GithubReadStart::Immediate(complete(
                canonical_text,
                selector,
                freshness,
                Vec::new(),
            ));
        }
        let downloader = Arc::clone(&self.image_downloader);
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = complete_with_optional_attachments(
                &request,
                selector,
                canonical_text,
                freshness,
                downloader.as_ref(),
            );
            let _ = sender.send(result);
        });
        GithubReadStart::Deferred(GithubReadDeferred { receiver })
    }

    fn schedule_background_refresh(&self, request: GithubReadRequest, slot: CacheSlot) {
        {
            let mut state = self.state.lock();
            if !state.refreshes.insert(slot.clone()) {
                return;
            }
        }
        let cache = Arc::clone(&self.cache);
        let fetcher = Arc::clone(&self.fetcher);
        let clock = Arc::clone(&self.clock);
        let state = Arc::clone(&self.state);
        std::thread::spawn(move || {
            // A failed refresh deliberately does not touch its previous row.
            let _ = fetch_render_store(
                &request,
                cache.as_ref(),
                fetcher.as_ref(),
                clock.as_ref(),
                &state,
            );
            state.lock().refreshes.remove(&slot);
        });
    }
}

fn fetch_render_store(
    request: &GithubReadRequest,
    cache: &dyn GithubReadCacheStore,
    fetcher: &dyn GithubFetcher,
    clock: &dyn GithubReadClock,
    state: &Mutex<GithubReadEngineState>,
) -> Result<String, GithubReadError> {
    let document = fetcher.fetch(&GithubFetchRequest {
        resource: request.resource.clone(),
        working_directory: request.working_directory.clone(),
    })?;
    let repository = document.repository.clone();
    let canonical_text = render_document(&document);
    let slot = cache_slot(
        &request.resource,
        &repository,
        &request.effective_authentication_identity,
    );
    let key = github_cache_key(&slot, &request.effective_authentication_identity)?;
    // A cache-write outage should not convert a fetched document into a failed
    // read. The response is still complete; a later read simply refetches.
    if let Err(error) = cache.upsert(&key, &canonical_text, clock.now_ms()) {
        log::warn!("GitHub read cache write failed: {error}");
    }
    if request.resource.repository.is_none() {
        state
            .lock()
            .aliases
            .insert(short_alias(request), repository);
    }
    Ok(canonical_text)
}

fn complete_with_optional_attachments(
    request: &GithubReadRequest,
    selector: GithubReadSelector,
    canonical_text: String,
    freshness: GithubReadFreshness,
    downloader: &dyn GithubImageDownloader,
) -> Result<GithubReadCompletion, GithubReadError> {
    let attachments = if request.vision_capability == Some(true) {
        download_github_image_attachments(&canonical_text, downloader)
    } else {
        Vec::new()
    };
    Ok(complete(canonical_text, selector, freshness, attachments))
}

fn complete(
    canonical_text: String,
    selector: GithubReadSelector,
    freshness: GithubReadFreshness,
    attachments: Vec<GithubImageAttachment>,
) -> GithubReadCompletion {
    let total_lines = canonical_text.lines().count();
    GithubReadCompletion {
        content: apply_selector(&canonical_text, selector),
        total_lines,
        freshness,
        attachments,
    }
}

/// Apply selection to the completed canonical render, never to raw GitHub data.
pub fn apply_selector(canonical_text: &str, selector: GithubReadSelector) -> String {
    match selector {
        GithubReadSelector::WholeDocument => canonical_text.to_string(),
        GithubReadSelector::LineRange {
            start_line,
            end_line,
            limit,
        } => {
            let lines: Vec<_> = canonical_text.lines().collect();
            let start_index = start_line.saturating_sub(1).min(lines.len());
            let requested_end =
                end_line.unwrap_or_else(|| start_line.saturating_add(limit).saturating_sub(1));
            let end_index = requested_end.min(lines.len()).max(start_index);
            let selected = lines[start_index..end_index].join("\n");
            (!selected.is_empty())
                .then(|| format!("{selected}\n"))
                .unwrap_or_default()
        }
        GithubReadSelector::ByteOffset { offset, limit } => {
            let start = canonical_text.floor_char_boundary(offset.min(canonical_text.len()));
            let requested_end = limit
                .map(|limit| start.saturating_add(limit).min(canonical_text.len()))
                .unwrap_or(canonical_text.len());
            let end = canonical_text.floor_char_boundary(requested_end);
            canonical_text[start..end].to_string()
        }
    }
}

fn short_alias(request: &GithubReadRequest) -> ShortResourceAlias {
    ShortResourceAlias {
        kind: request.resource.kind,
        number: request.resource.number,
        working_directory: request.working_directory.clone(),
        authentication_identity_hash: authentication_identity_hash(
            &request.effective_authentication_identity,
        ),
    }
}

fn cache_slot(
    resource: &GithubResource,
    repository: &str,
    authentication_identity: &str,
) -> CacheSlot {
    CacheSlot {
        kind: resource.kind,
        repository: repository.trim().to_ascii_lowercase(),
        number: resource.number,
        authentication_identity_hash: authentication_identity_hash(authentication_identity),
    }
}

fn github_cache_key(
    slot: &CacheSlot,
    authentication_identity: &str,
) -> Result<GithubReadCacheKey, GithubReadError> {
    let number = i64::try_from(slot.number).map_err(|_| {
        GithubReadError::FetchFailed(
            "GitHub resource number exceeds cache storage range".to_string(),
        )
    })?;
    Ok(GithubReadCacheKey::new(
        database_kind(slot.kind),
        &slot.repository,
        number,
        authentication_identity,
    ))
}

fn database_kind(kind: GithubResourceKind) -> GithubReadResourceKind {
    match kind {
        GithubResourceKind::Issue => GithubReadResourceKind::Issue,
        GithubResourceKind::PullRequest => GithubReadResourceKind::PullRequest,
    }
}

fn authentication_identity_hash(identity: &str) -> [u8; 32] {
    *blake3::hash(identity.as_bytes()).as_bytes()
}

fn cache_failure(error: String) -> GithubReadError {
    GithubReadError::FetchFailed(format!("GitHub read cache is unavailable: {error}"))
}

/// Convenience constructor for the real cache location. The read integration
/// supplies its existing `aft.db` path; this module never creates a second DB.
pub fn sqlite_cache_store(database_path: impl AsRef<Path>) -> Arc<dyn GithubReadCacheStore> {
    Arc::new(SqliteGithubReadCacheStore::new(database_path.as_ref()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use super::*;
    use crate::github_read::attachments::{DownloadedGithubImage, GithubImageDownloader};
    use crate::github_read::model::{GithubDocument, GithubDocumentKind};

    #[derive(Default)]
    struct MemoryCache(Mutex<Option<GithubReadCacheEntry>>);

    impl GithubReadCacheStore for MemoryCache {
        fn lookup(
            &self,
            _key: &GithubReadCacheKey,
        ) -> Result<Option<GithubReadCacheEntry>, String> {
            Ok(self.0.lock().clone())
        }

        fn upsert(
            &self,
            _key: &GithubReadCacheKey,
            canonical_text: &str,
            fetched_at_ms: i64,
        ) -> Result<(), String> {
            *self.0.lock() = Some(GithubReadCacheEntry {
                canonical_text: canonical_text.to_string(),
                fetched_at_ms,
                updated_at_ms: fetched_at_ms,
            });
            Ok(())
        }

        fn evict_hard_expired_at(&self, _cutoff_ms: i64) -> Result<usize, String> {
            Ok(0)
        }

        fn invalidate(
            &self,
            _kind: GithubResourceKind,
            _repository: &str,
            _number: u64,
            _authentication_identity: Option<&str>,
        ) -> Result<usize, String> {
            Ok(0)
        }
    }

    struct FixtureClock(AtomicI64);

    impl FixtureClock {
        fn new(now: i64) -> Self {
            Self(AtomicI64::new(now))
        }

        fn set(&self, now: i64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl GithubReadClock for FixtureClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct FixtureFetcher(AtomicUsize);

    impl GithubFetcher for FixtureFetcher {
        fn fetch(&self, request: &GithubFetchRequest) -> Result<GithubDocument, GithubReadError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(GithubDocument {
                repository: request
                    .resource
                    .repository
                    .clone()
                    .unwrap_or_else(|| "owner/repo".to_string()),
                kind: GithubDocumentKind::Issue,
                number: request.resource.number,
                title: "fixture".to_string(),
                state: "OPEN".to_string(),
                body: "body https://user-images.githubusercontent.com/fixture.png".to_string(),
                ..GithubDocument::default()
            })
        }
    }

    struct GatedRefreshFetcher {
        calls: AtomicUsize,
        refresh_started: std::sync::mpsc::SyncSender<()>,
        refresh_release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    impl GithubFetcher for GatedRefreshFetcher {
        fn fetch(&self, request: &GithubFetchRequest) -> Result<GithubDocument, GithubReadError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 2 {
                let _ = self.refresh_started.send(());
                if let Some(release) = self.refresh_release.lock().take() {
                    let _ = release.recv();
                }
            }
            Ok(GithubDocument {
                repository: "owner/repo".to_string(),
                kind: GithubDocumentKind::Issue,
                number: request.resource.number,
                title: format!("fixture {call}"),
                state: "OPEN".to_string(),
                body: "body".to_string(),
                ..GithubDocument::default()
            })
        }
    }

    #[derive(Default)]
    struct CountingDownloader(AtomicUsize);

    impl GithubImageDownloader for CountingDownloader {
        fn download(
            &self,
            _url: &url::Url,
            _maximum_bytes: usize,
        ) -> Result<Option<DownloadedGithubImage>, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
    }

    fn enabled_gh_read() -> GhReadConfig {
        GhReadConfig { enabled: true }
    }

    fn request(vision_capability: Option<bool>) -> GithubReadRequest {
        GithubReadRequest::parse("issue://1", "/fixture", "identity", vision_capability).unwrap()
    }

    fn wait_for(deferred: GithubReadDeferred) -> Result<GithubReadCompletion, GithubReadError> {
        for _ in 0..1000 {
            if let Some(result) = deferred.try_complete() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("deferred read did not complete")
    }

    #[test]
    fn disabled_read_refuses_before_the_fetch_seam_runs() {
        let fetcher = Arc::new(FixtureFetcher::default());
        let engine = GithubReadEngine::new(
            Arc::new(MemoryCache::default()),
            fetcher.clone(),
            Arc::new(CountingDownloader::default()),
            Arc::new(FixtureClock::new(1_000)),
        );

        let error = match engine.start_resource(
            &GhReadConfig::default(),
            "issue://1",
            "/fixture",
            "identity",
            None,
            GithubReadSelector::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("disabled GitHub reads must refuse"),
        };

        assert_eq!(error.code(), "gh_read_disabled");
        assert_eq!(
            error.to_string(),
            "GitHub reads are disabled; set gh_read.enabled: true in aft.jsonc"
        );
        assert_eq!(fetcher.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn enabled_read_reaches_the_fixture_fetcher() {
        let fetcher = Arc::new(FixtureFetcher::default());
        let engine = GithubReadEngine::new(
            Arc::new(MemoryCache::default()),
            fetcher.clone(),
            Arc::new(CountingDownloader::default()),
            Arc::new(FixtureClock::new(1_000)),
        );

        let GithubReadStart::Deferred(deferred) = engine
            .start_resource(
                &enabled_gh_read(),
                "issue://1",
                "/fixture",
                "identity",
                None,
                GithubReadSelector::default(),
            )
            .expect("enabled GitHub reads should proceed")
        else {
            panic!("a cache miss must defer its GitHub fetch");
        };

        assert!(wait_for(deferred).unwrap().content.contains("# Issue #1"));
        assert_eq!(fetcher.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn missing_capability_is_text_only_and_cache_reuse_avoids_a_second_fetch() {
        let cache = Arc::new(MemoryCache::default());
        let fetcher = Arc::new(FixtureFetcher::default());
        let downloader = Arc::new(CountingDownloader::default());
        let engine = GithubReadEngine::new(
            cache,
            fetcher.clone(),
            downloader.clone(),
            Arc::new(FixtureClock::new(1_000)),
        );

        let GithubReadStart::Deferred(first) = engine
            .start(
                &enabled_gh_read(),
                request(None),
                GithubReadSelector::default(),
            )
            .unwrap()
        else {
            panic!("cache miss must defer its GitHub fetch");
        };
        assert_eq!(wait_for(first).unwrap().attachments.len(), 0);
        let GithubReadStart::Immediate(second) = engine
            .start(
                &enabled_gh_read(),
                request(None),
                GithubReadSelector::default(),
            )
            .unwrap()
        else {
            panic!("fresh text-only cache hit must be immediate");
        };
        assert_eq!(second.freshness, GithubReadFreshness::FreshCache);
        assert_eq!(fetcher.0.load(Ordering::SeqCst), 1);
        assert_eq!(downloader.0.load(Ordering::SeqCst), 0);

        let GithubReadStart::Deferred(vision) = engine
            .start(
                &enabled_gh_read(),
                request(Some(true)),
                GithubReadSelector::default(),
            )
            .unwrap()
        else {
            panic!("vision attachments must run outside the request loop");
        };
        assert!(wait_for(vision).unwrap().attachments.is_empty());
        assert_eq!(fetcher.0.load(Ordering::SeqCst), 1);
        assert_eq!(downloader.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stale_reads_share_one_background_refresh_and_keep_stale_text() {
        let cache = Arc::new(MemoryCache::default());
        let (refresh_started_tx, refresh_started_rx) = std::sync::mpsc::sync_channel(1);
        let (refresh_release_tx, refresh_release_rx) = std::sync::mpsc::sync_channel(1);
        let fetcher = Arc::new(GatedRefreshFetcher {
            calls: AtomicUsize::new(0),
            refresh_started: refresh_started_tx,
            refresh_release: Mutex::new(Some(refresh_release_rx)),
        });
        let clock = Arc::new(FixtureClock::new(1_000));
        let engine = GithubReadEngine::new(
            cache,
            fetcher.clone(),
            Arc::new(CountingDownloader::default()),
            clock.clone(),
        );
        let GithubReadStart::Deferred(first) = engine
            .start(
                &enabled_gh_read(),
                request(None),
                GithubReadSelector::default(),
            )
            .unwrap()
        else {
            panic!("cache miss must defer");
        };
        assert!(wait_for(first).unwrap().content.contains("fixture 1"));

        clock.set(1_000 + GITHUB_READ_CACHE_SOFT_TTL_MS + 1);
        let GithubReadStart::Immediate(first_stale) = engine
            .start(
                &enabled_gh_read(),
                request(None),
                GithubReadSelector::default(),
            )
            .unwrap()
        else {
            panic!("stale text-only cache hit must remain immediate");
        };
        assert_eq!(
            first_stale.freshness,
            GithubReadFreshness::StaleCacheRefreshing
        );
        assert!(first_stale.content.contains("fixture 1"));
        refresh_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("background refresh started");
        assert_eq!(engine.refresh_in_flight_for_test(), 1);

        let GithubReadStart::Immediate(second_stale) = engine
            .start(
                &enabled_gh_read(),
                request(None),
                GithubReadSelector::default(),
            )
            .unwrap()
        else {
            panic!("concurrent stale cache hit must remain immediate");
        };
        assert!(second_stale.content.contains("fixture 1"));
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 2);
        refresh_release_tx.send(()).unwrap();
    }
}
