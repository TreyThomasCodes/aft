#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

use aft::github_read::{
    download_github_image_attachments, render_document, sqlite_cache_store, DownloadedGithubImage,
    GithubDocument, GithubDocumentKind, GithubFetchRequest, GithubFetcher, GithubImageDownloader,
    GithubReadClock, GithubReadCompletion, GithubReadEngine, GithubReadError, GithubReadFreshness,
    GithubReadSelector, GithubReadStart, GithubResourceKind, GITHUB_READ_CACHE_HARD_TTL_MS,
    GITHUB_READ_CACHE_SOFT_TTL_MS, MAX_GITHUB_IMAGE_ATTACHMENTS, MAX_GITHUB_IMAGE_ATTACHMENT_BYTES,
};
use parking_lot::Mutex;
use url::Url;

const FIXTURE_DIRECTORY: &str = "tests/fixtures/github_read_cache_attachment";
const RESOURCE: &str = "issue://owner/repo/7";
const PNG_SIGNATURE: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10];

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIRECTORY)
        .join(name)
}

fn bug_report_fixture() -> GithubDocument {
    serde_json::from_str(
        &fs::read_to_string(fixture_path("bug_report.json"))
            .expect("read GitHub bug-report fixture"),
    )
    .expect("parse GitHub bug-report fixture")
}

struct FixtureClock(AtomicI64);

impl FixtureClock {
    fn new(now_ms: i64) -> Self {
        Self(AtomicI64::new(now_ms))
    }

    fn set(&self, now_ms: i64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }
}

impl GithubReadClock for FixtureClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct ProbeFetcher {
    calls: AtomicUsize,
    refresh_started: Option<mpsc::SyncSender<()>>,
    refresh_release: Mutex<Option<mpsc::Receiver<()>>>,
}

impl ProbeFetcher {
    fn ordinary() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            refresh_started: None,
            refresh_release: Mutex::new(None),
        }
    }

    fn gated(refresh_started: mpsc::SyncSender<()>, refresh_release: mpsc::Receiver<()>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            refresh_started: Some(refresh_started),
            refresh_release: Mutex::new(Some(refresh_release)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl GithubFetcher for ProbeFetcher {
    fn fetch(&self, request: &GithubFetchRequest) -> Result<GithubDocument, GithubReadError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 2 {
            if let Some(started) = &self.refresh_started {
                started.send(()).expect("report background refresh start");
                if let Some(release) = self.refresh_release.lock().take() {
                    release.recv().expect("release background refresh");
                }
            }
        }
        let repository = request.resource.repository.clone().unwrap_or_else(|| {
            let context = request
                .working_directory
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unresolved");
            format!("owner/{context}")
        });
        let kind = match request.resource.kind {
            GithubResourceKind::Issue => GithubDocumentKind::Issue,
            GithubResourceKind::PullRequest => GithubDocumentKind::PullRequest,
        };
        Ok(GithubDocument {
            repository,
            kind,
            number: request.resource.number,
            title: format!("network revision {call}"),
            state: "OPEN".to_string(),
            body: format!("network body revision {call}"),
            ..GithubDocument::default()
        })
    }
}

struct StaticFixtureFetcher {
    document: GithubDocument,
    calls: AtomicUsize,
}

impl StaticFixtureFetcher {
    fn new(document: GithubDocument) -> Self {
        Self {
            document,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl GithubFetcher for StaticFixtureFetcher {
    fn fetch(&self, _request: &GithubFetchRequest) -> Result<GithubDocument, GithubReadError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.document.clone())
    }
}

struct ScriptedDownloader {
    calls: Mutex<Vec<(String, usize)>>,
    outcomes: BTreeMap<String, Result<Option<DownloadedGithubImage>, String>>,
}

impl ScriptedDownloader {
    fn new(outcomes: BTreeMap<String, Result<Option<DownloadedGithubImage>, String>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            outcomes,
        }
    }

    fn calls(&self) -> Vec<(String, usize)> {
        self.calls.lock().clone()
    }
}

impl GithubImageDownloader for ScriptedDownloader {
    fn download(
        &self,
        url: &Url,
        maximum_bytes: usize,
    ) -> Result<Option<DownloadedGithubImage>, String> {
        let source = url.to_string();
        self.calls.lock().push((source.clone(), maximum_bytes));
        self.outcomes.get(&source).cloned().unwrap_or(Ok(None))
    }
}

fn successful_download(url: &str, mime: &str, bytes: Vec<u8>) -> DownloadedGithubImage {
    DownloadedGithubImage {
        final_url: Url::parse(url).expect("parse fixture image URL"),
        mime: mime.to_string(),
        bytes,
    }
}

fn png_bytes() -> Vec<u8> {
    PNG_SIGNATURE.to_vec()
}

fn oversized_png() -> Vec<u8> {
    let mut bytes = vec![0; MAX_GITHUB_IMAGE_ATTACHMENT_BYTES + 1];
    bytes[..PNG_SIGNATURE.len()].copy_from_slice(PNG_SIGNATURE);
    bytes
}

fn complete_read(start: GithubReadStart) -> GithubReadCompletion {
    match start {
        GithubReadStart::Immediate(completion) => completion,
        GithubReadStart::Deferred(deferred) => {
            for _ in 0..1_000 {
                if let Some(completion) = deferred.try_complete() {
                    return completion.expect("fixture GitHub read succeeds");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("fixture GitHub read did not complete");
        }
    }
}

fn immediate_read(start: GithubReadStart) -> GithubReadCompletion {
    match start {
        GithubReadStart::Immediate(completion) => completion,
        GithubReadStart::Deferred(_) => panic!("cache hit must not wait on a foreground worker"),
    }
}

fn wait_for_refresh_clear(engine: &GithubReadEngine) {
    for _ in 0..1_000 {
        if engine.refresh_in_flight_for_test() == 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("background cache refresh did not finish");
}

#[test]
fn cache_hits_are_network_free_and_stale_and_hard_ttl_paths_are_single_flight() {
    let storage = tempfile::tempdir().expect("create cache storage");
    let (refresh_started_tx, refresh_started_rx) = mpsc::sync_channel(1);
    let (refresh_release_tx, refresh_release_rx) = mpsc::sync_channel(1);
    let fetcher = Arc::new(ProbeFetcher::gated(refresh_started_tx, refresh_release_rx));
    let clock = Arc::new(FixtureClock::new(1_000));
    let engine = Arc::new(GithubReadEngine::new(
        sqlite_cache_store(storage.path().join("aft.db")),
        fetcher.clone(),
        Arc::new(ScriptedDownloader::new(BTreeMap::new())),
        clock.clone(),
    ));

    let GithubReadStart::Deferred(first_pending) = engine
        .start_resource(
            RESOURCE,
            "/fixture/cache",
            "principal:alice",
            None,
            GithubReadSelector::WholeDocument,
        )
        .expect("start cache-miss read")
    else {
        panic!("a cache miss must use pending-response execution");
    };
    let first = complete_read(GithubReadStart::Deferred(first_pending));
    assert_eq!(first.freshness, GithubReadFreshness::Fetched);
    assert!(first.content.contains("network revision 1"));
    assert_eq!(fetcher.calls(), 1);

    let fresh = immediate_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/cache",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("start fresh cached read"),
    );
    assert_eq!(fresh.content, first.content);
    assert_eq!(fresh.freshness, GithubReadFreshness::FreshCache);
    assert_eq!(fresh.freshness.note(), None);
    assert_eq!(fetcher.calls(), 1, "a soft-TTL hit must not call GitHub");

    clock.set(1_000 + GITHUB_READ_CACHE_SOFT_TTL_MS + 1);
    let barrier = Arc::new(Barrier::new(5));
    let mut readers = Vec::new();
    for _ in 0..4 {
        let reader_engine = Arc::clone(&engine);
        let reader_barrier = Arc::clone(&barrier);
        readers.push(std::thread::spawn(move || {
            reader_barrier.wait();
            immediate_read(
                reader_engine
                    .start_resource(
                        RESOURCE,
                        "/fixture/cache",
                        "principal:alice",
                        None,
                        GithubReadSelector::WholeDocument,
                    )
                    .expect("start concurrent stale cached read"),
            )
        }));
    }
    barrier.wait();
    for reader in readers {
        let stale = reader.join().expect("concurrent stale reader returns");
        assert_eq!(stale.content, first.content);
        assert_eq!(
            stale.freshness,
            GithubReadFreshness::StaleCacheRefreshing,
            "each stale reader must identify the background refresh"
        );
        assert_eq!(
            stale.freshness.note(),
            Some("Cached GitHub data is stale; a background refresh is in progress.")
        );
    }
    refresh_started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("one background refresh starts");
    assert_eq!(engine.refresh_in_flight_for_test(), 1);
    assert_eq!(
        fetcher.calls(),
        2,
        "concurrent stale reads must share one exact-key GitHub refresh"
    );

    refresh_release_tx
        .send(())
        .expect("release background refresh");
    wait_for_refresh_clear(engine.as_ref());
    clock.set(1_000 + GITHUB_READ_CACHE_SOFT_TTL_MS + 1 + GITHUB_READ_CACHE_HARD_TTL_MS);

    let GithubReadStart::Deferred(hard_pending) = engine
        .start_resource(
            RESOURCE,
            "/fixture/cache",
            "principal:alice",
            None,
            GithubReadSelector::WholeDocument,
        )
        .expect("start hard-expired read")
    else {
        panic!("a hard-TTL hit must evict and refetch through pending execution");
    };
    let hard_refetch = complete_read(GithubReadStart::Deferred(hard_pending));
    assert_eq!(hard_refetch.freshness, GithubReadFreshness::Fetched);
    assert!(hard_refetch.content.contains("network revision 3"));
    assert_eq!(fetcher.calls(), 3);
}

#[test]
fn cache_keys_keep_authentication_and_unresolved_short_contexts_isolated() {
    let storage = tempfile::tempdir().expect("create cache storage");
    let fetcher = Arc::new(ProbeFetcher::ordinary());
    let engine = GithubReadEngine::new(
        sqlite_cache_store(storage.path().join("aft.db")),
        fetcher.clone(),
        Arc::new(ScriptedDownloader::new(BTreeMap::new())),
        Arc::new(FixtureClock::new(1_000)),
    );

    let alice = complete_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/authentication",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("start Alice read"),
    );
    let bob = complete_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/authentication",
                "principal:bob",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("start Bob read"),
    );
    assert_eq!(
        fetcher.calls(),
        2,
        "identities must not share a cache entry"
    );
    assert_ne!(alice.content, bob.content);
    immediate_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/authentication",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("reuse Alice cache entry"),
    );
    immediate_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/authentication",
                "principal:bob",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("reuse Bob cache entry"),
    );
    assert_eq!(fetcher.calls(), 2);

    let first_short = complete_read(
        engine
            .start_resource(
                "issue://7",
                "/fixture/worktree-a",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("resolve first short resource"),
    );
    let second_short = complete_read(
        engine
            .start_resource(
                "issue://7",
                "/fixture/worktree-b",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("resolve second short resource"),
    );
    assert_eq!(
        fetcher.calls(),
        4,
        "unresolved short resources from different working directories must fetch independently"
    );
    assert!(first_short.content.contains("Repository: owner/worktree-a"));
    assert!(second_short
        .content
        .contains("Repository: owner/worktree-b"));
    immediate_read(
        engine
            .start_resource(
                "issue://7",
                "/fixture/worktree-a",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("reuse first short-resource cache entry"),
    );
    immediate_read(
        engine
            .start_resource(
                "issue://7",
                "/fixture/worktree-b",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("reuse second short-resource cache entry"),
    );
    assert_eq!(fetcher.calls(), 4);
}

#[test]
fn successful_same_resource_invalidation_refetches_without_evicting_failed_or_other_mutations() {
    let storage = tempfile::tempdir().expect("create cache storage");
    let fetcher = Arc::new(ProbeFetcher::ordinary());
    let engine = GithubReadEngine::new(
        sqlite_cache_store(storage.path().join("aft.db")),
        fetcher.clone(),
        Arc::new(ScriptedDownloader::new(BTreeMap::new())),
        Arc::new(FixtureClock::new(1_000)),
    );

    complete_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/mutation",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("seed cache before mutation"),
    );
    assert_eq!(fetcher.calls(), 1);

    immediate_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/mutation",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("failed gh-shim mutation leaves cache intact"),
    );
    assert_eq!(
        fetcher.calls(),
        1,
        "the failure path does not invoke the success-only invalidation seam"
    );

    assert_eq!(
        engine
            .invalidate(GithubResourceKind::Issue, "owner/repo", 8, None)
            .expect("invalidate unrelated gh-shim mutation resource"),
        0
    );
    immediate_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/mutation",
                "principal:alice",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("unrelated gh-shim mutation leaves control cached"),
    );
    assert_eq!(fetcher.calls(), 1);

    assert_eq!(
        engine
            .invalidate(GithubResourceKind::Issue, "owner/repo", 7, None)
            .expect("apply successful same-resource gh-shim invalidation"),
        1
    );
    let GithubReadStart::Deferred(refetch_pending) = engine
        .start_resource(
            RESOURCE,
            "/fixture/mutation",
            "principal:alice",
            None,
            GithubReadSelector::WholeDocument,
        )
        .expect("read after same-resource invalidation")
    else {
        panic!("a successful same-resource mutation must force a pending refetch");
    };
    let refetched = complete_read(GithubReadStart::Deferred(refetch_pending));
    assert!(refetched.content.contains("network revision 2"));
    assert_eq!(fetcher.calls(), 2);
}

#[test]
fn vision_capability_preserves_bug_report_text_and_only_downloads_for_explicit_true() {
    let storage = tempfile::tempdir().expect("create cache storage");
    let first = "https://user-images.githubusercontent.com/1234/first.png";
    let second = "https://github.com/user-attachments/files/73/second.png";
    let outcomes = BTreeMap::from([
        (
            first.to_string(),
            Ok(Some(successful_download(first, "image/png", png_bytes()))),
        ),
        (
            second.to_string(),
            Ok(Some(successful_download(second, "image/png", png_bytes()))),
        ),
    ]);
    let fetcher = Arc::new(StaticFixtureFetcher::new(bug_report_fixture()));
    let downloader = Arc::new(ScriptedDownloader::new(outcomes));
    let engine = GithubReadEngine::new(
        sqlite_cache_store(storage.path().join("aft.db")),
        fetcher.clone(),
        downloader.clone(),
        Arc::new(FixtureClock::new(1_000)),
    );

    let missing_capability = complete_read(
        engine
            .start_resource(
                "issue://cortexkit/aft/73",
                "/fixture/vision",
                "principal:vision",
                None,
                GithubReadSelector::WholeDocument,
            )
            .expect("read fixture without capability"),
    );
    assert!(missing_capability.content.contains(first));
    assert!(missing_capability.content.contains(second));
    assert!(missing_capability.attachments.is_empty());
    assert!(downloader.calls().is_empty());

    let false_capability = immediate_read(
        engine
            .start_resource(
                "issue://cortexkit/aft/73",
                "/fixture/vision",
                "principal:vision",
                Some(false),
                GithubReadSelector::WholeDocument,
            )
            .expect("read fixture with false capability"),
    );
    assert_eq!(false_capability.content, missing_capability.content);
    assert!(false_capability.attachments.is_empty());
    assert!(downloader.calls().is_empty());

    let explicit_vision = complete_read(
        engine
            .start_resource(
                "issue://cortexkit/aft/73",
                "/fixture/vision",
                "principal:vision",
                Some(true),
                GithubReadSelector::WholeDocument,
            )
            .expect("read fixture with explicit vision capability"),
    );
    assert_eq!(explicit_vision.content, missing_capability.content);
    assert_eq!(
        fetcher.calls(),
        1,
        "vision must reuse canonical cached text"
    );
    assert_eq!(
        explicit_vision
            .attachments
            .iter()
            .map(|attachment| attachment.source_url.as_str())
            .collect::<Vec<_>>(),
        [first, second],
        "eligible attachments must follow document order across both allowed host forms"
    );
    assert_eq!(
        downloader.calls(),
        vec![
            (first.to_string(), MAX_GITHUB_IMAGE_ATTACHMENT_BYTES),
            (
                second.to_string(),
                MAX_GITHUB_IMAGE_ATTACHMENT_BYTES - PNG_SIGNATURE.len(),
            ),
        ]
    );
}

#[test]
fn invalid_or_failed_attachment_candidates_leave_the_text_read_complete_without_partial_data() {
    let storage = tempfile::tempdir().expect("create cache storage");
    let unsupported = "https://github.com/user-attachments/files/73/unsupported.svg";
    let redirected = "https://user-images.githubusercontent.com/1234/redirected.png";
    let failed = "https://github.com/user-attachments/files/73/failure.png";
    let oversized = "https://user-images.githubusercontent.com/1234/oversized.png";
    let malformed = "https://github.com:99999/user-attachments/malformed.png";
    let ineligible_host = "https://example.test/bug.png";
    let ineligible_github_path = "https://github.com/cortexkit/aft/blob/main/bug.png";
    let document = GithubDocument {
        repository: "owner/repo".to_string(),
        kind: GithubDocumentKind::Issue,
        number: 7,
        title: "attachment failure fixture".to_string(),
        state: "OPEN".to_string(),
        body: [
            unsupported,
            redirected,
            failed,
            oversized,
            malformed,
            ineligible_host,
            ineligible_github_path,
        ]
        .join("\n"),
        ..GithubDocument::default()
    };
    let outcomes = BTreeMap::from([
        (
            unsupported.to_string(),
            Ok(Some(successful_download(
                unsupported,
                "image/svg+xml",
                png_bytes(),
            ))),
        ),
        (
            redirected.to_string(),
            Ok(Some(successful_download(
                "https://example.test/redirected.png",
                "image/png",
                png_bytes(),
            ))),
        ),
        (
            failed.to_string(),
            Err("fixture image host failed".to_string()),
        ),
        (
            oversized.to_string(),
            Ok(Some(successful_download(
                oversized,
                "image/png",
                oversized_png(),
            ))),
        ),
    ]);
    let downloader = Arc::new(ScriptedDownloader::new(outcomes));
    let engine = GithubReadEngine::new(
        sqlite_cache_store(storage.path().join("aft.db")),
        Arc::new(StaticFixtureFetcher::new(document)),
        downloader.clone(),
        Arc::new(FixtureClock::new(1_000)),
    );

    let completion = complete_read(
        engine
            .start_resource(
                RESOURCE,
                "/fixture/attachment-failures",
                "principal:vision",
                Some(true),
                GithubReadSelector::WholeDocument,
            )
            .expect("start attachment failure fixture read"),
    );
    assert!(completion.attachments.is_empty());
    for url in [
        unsupported,
        redirected,
        failed,
        oversized,
        malformed,
        ineligible_host,
        ineligible_github_path,
    ] {
        assert!(completion.content.contains(url), "text keeps {url}");
    }
    assert_eq!(
        downloader.calls(),
        vec![
            (unsupported.to_string(), MAX_GITHUB_IMAGE_ATTACHMENT_BYTES),
            (redirected.to_string(), MAX_GITHUB_IMAGE_ATTACHMENT_BYTES),
            (failed.to_string(), MAX_GITHUB_IMAGE_ATTACHMENT_BYTES),
            (oversized.to_string(), MAX_GITHUB_IMAGE_ATTACHMENT_BYTES),
        ],
        "malformed and ineligible URLs must never reach the image network seam"
    );
}

#[test]
fn attachment_count_and_aggregate_byte_budgets_stop_before_exposing_partial_images() {
    let count_urls = (0..=MAX_GITHUB_IMAGE_ATTACHMENTS)
        .map(|index| format!("https://user-images.githubusercontent.com/1234/count-{index}.png"))
        .collect::<Vec<_>>();
    let count_outcomes = count_urls
        .iter()
        .map(|url| {
            (
                url.clone(),
                Ok(Some(successful_download(url, "image/png", png_bytes()))),
            )
        })
        .collect();
    let count_downloader = ScriptedDownloader::new(count_outcomes);
    let count_attachments =
        download_github_image_attachments(&count_urls.join("\n"), &count_downloader);
    assert_eq!(count_attachments.len(), MAX_GITHUB_IMAGE_ATTACHMENTS);
    assert_eq!(
        count_attachments
            .iter()
            .map(|attachment| attachment.source_url.as_str())
            .collect::<Vec<_>>(),
        count_urls[..MAX_GITHUB_IMAGE_ATTACHMENTS]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert_eq!(count_downloader.calls().len(), MAX_GITHUB_IMAGE_ATTACHMENTS);

    let first = "https://github.com/user-attachments/files/73/almost-full.png";
    let second = "https://user-images.githubusercontent.com/1234/remainder.png";
    let third = "https://github.com/user-attachments/files/73/never-requested.png";
    let mut almost_full_png = vec![0; MAX_GITHUB_IMAGE_ATTACHMENT_BYTES - PNG_SIGNATURE.len()];
    almost_full_png[..PNG_SIGNATURE.len()].copy_from_slice(PNG_SIGNATURE);
    let aggregate_outcomes = BTreeMap::from([
        (
            first.to_string(),
            Ok(Some(successful_download(
                first,
                "image/png",
                almost_full_png,
            ))),
        ),
        (
            second.to_string(),
            Ok(Some(successful_download(second, "image/png", png_bytes()))),
        ),
        (
            third.to_string(),
            Ok(Some(successful_download(third, "image/png", png_bytes()))),
        ),
    ]);
    let aggregate_downloader = ScriptedDownloader::new(aggregate_outcomes);
    let aggregate_attachments = download_github_image_attachments(
        &[first, second, third].join("\n"),
        &aggregate_downloader,
    );
    assert_eq!(aggregate_attachments.len(), 2);
    assert_eq!(
        aggregate_attachments
            .iter()
            .map(|attachment| attachment.bytes.len())
            .sum::<usize>(),
        MAX_GITHUB_IMAGE_ATTACHMENT_BYTES
    );
    assert_eq!(
        aggregate_downloader.calls(),
        vec![
            (first.to_string(), MAX_GITHUB_IMAGE_ATTACHMENT_BYTES),
            (second.to_string(), PNG_SIGNATURE.len()),
        ],
        "an exhausted aggregate budget must skip later downloads instead of exposing partial data"
    );

    let rendered = render_document(&GithubDocument {
        repository: "owner/repo".to_string(),
        kind: GithubDocumentKind::Issue,
        number: 7,
        title: "bounded attachment fixture".to_string(),
        state: "OPEN".to_string(),
        body: [first, second, third].join("\n"),
        ..GithubDocument::default()
    });
    assert!(
        rendered.contains(third),
        "attachment bounds never remove text URLs"
    );
}
