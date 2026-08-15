use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unicode_normalization::UnicodeNormalization;

use crate::lsp::diagnostics::{DiagnosticSeverity, StoredDiagnostic};
use crate::lsp::roots::ServerKey;

/// Idle alert sessions are reaped by the embedding runtime. The duration is a
/// product constant; callers inject `now` when they run a sweep so lifecycle
/// behavior does not depend on wall-clock timing in tests.
pub const ALERT_SESSION_IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// A stable identifier for the server partition that produced a diagnostic
/// snapshot. The workspace root is included because a server kind can run for
/// more than one nested workspace beneath one dispatch root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProducerKey(String);

impl ProducerKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn from_server_key(server: &ServerKey) -> Self {
        Self(format!(
            "{}@{}",
            server.kind.id_str(),
            canonicalize_for_alert(&server.root).display()
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The canonical, error-only identity used by the alert delta engine.
///
/// The source/server field is intentionally part of the identity. Two
/// producers can report otherwise identical findings without sharing an alert
/// lifecycle or suppressing each other.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DiagnosticIdentity {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub severity: String,
    pub source: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

impl DiagnosticIdentity {
    /// Construct an alert identity for an error diagnostic. Non-errors never
    /// enter the alert state, although they remain in an accepted producer
    /// snapshot for other consumers.
    pub fn from_stored(canonical_root: &Path, diagnostic: &StoredDiagnostic) -> Option<Self> {
        (diagnostic.severity == DiagnosticSeverity::Error).then(|| Self {
            file: canonical_root_relative_file(canonical_root, &diagnostic.file),
            line: diagnostic.line,
            column: diagnostic.column,
            end_line: diagnostic.end_line,
            end_column: diagnostic.end_column,
            severity: diagnostic.severity.as_str().to_owned(),
            source: diagnostic.source.clone(),
            code: diagnostic.code.clone(),
            message: normalize_diagnostic_message(&diagnostic.message),
        })
    }
}

/// Normalize the message component of a diagnostic identity.
///
/// This intentionally performs only the canonical identity rules: first line,
/// trimming, whitespace-run collapse, and NFC. Do not add presentation or
/// path-oriented rewrites here; changing this function changes alert identity.
pub fn normalize_diagnostic_message(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .nfc()
        .collect()
}

/// One complete diagnostics snapshot that LSP accepted for one producer and
/// one document version. An empty `diagnostics` vector is an accepted clean
/// report, not an omitted or pending report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedDiagnosticSnapshot {
    pub server_key: ServerKey,
    pub document_version: i32,
    pub diagnostics: Vec<StoredDiagnostic>,
}

impl AcceptedDiagnosticSnapshot {
    pub fn new(
        server_key: ServerKey,
        document_version: i32,
        diagnostics: Vec<StoredDiagnostic>,
    ) -> Self {
        Self {
            server_key,
            document_version,
            diagnostics,
        }
    }

    pub fn producer_key(&self) -> ProducerKey {
        ProducerKey::from_server_key(&self.server_key)
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// An accepted observation is the only input that may mutate alert delta
/// state. Sources must assemble this from a complete, document-version-
/// verified producer snapshot before diagnostics are flattened for a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedObservation {
    pub session_id: String,
    pub canonical_root: PathBuf,
    pub producer_key: ProducerKey,
    pub accepted_document_version: i32,
    pub diagnostics: Vec<StoredDiagnostic>,
}

impl AcceptedObservation {
    pub fn new(
        session_id: impl Into<String>,
        canonical_root: impl AsRef<Path>,
        producer_key: ProducerKey,
        accepted_document_version: i32,
        diagnostics: Vec<StoredDiagnostic>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            canonical_root: canonicalize_for_alert(canonical_root.as_ref()),
            producer_key,
            accepted_document_version,
            diagnostics,
        }
    }

    pub fn from_snapshot(
        session_id: impl Into<String>,
        canonical_root: impl AsRef<Path>,
        snapshot: AcceptedDiagnosticSnapshot,
    ) -> Self {
        Self::new(
            session_id,
            canonical_root,
            snapshot.producer_key(),
            snapshot.document_version,
            snapshot.diagnostics,
        )
    }

    pub fn partition_key(&self) -> AlertPartitionKey {
        AlertPartitionKey {
            session_id: self.session_id.clone(),
            canonical_root: self.canonical_root.clone(),
            producer_key: self.producer_key.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// A terminal, all-or-nothing group of accepted producer snapshots. Producers
/// that are pending, warming, timed out, exited, or unversioned are represented
/// by absence from this batch and therefore cannot clear a partition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcceptedObservationBatch {
    observations: Vec<AcceptedObservation>,
}

impl AcceptedObservationBatch {
    /// Convert LSP's complete per-producer snapshots into this state engine's
    /// only mutating input. This is intentionally the conversion boundary: do
    /// not flatten snapshot diagnostics before calling it.
    pub fn from_diagnostic_snapshots(
        session_id: impl Into<String>,
        canonical_root: impl AsRef<Path>,
        snapshots: impl IntoIterator<Item = AcceptedDiagnosticSnapshot>,
    ) -> Result<Self, ObservationError> {
        let session_id = session_id.into();
        let canonical_root = canonicalize_for_alert(canonical_root.as_ref());
        Self::new(
            snapshots
                .into_iter()
                .map(|snapshot| {
                    AcceptedObservation::from_snapshot(
                        session_id.clone(),
                        canonical_root.clone(),
                        snapshot,
                    )
                })
                .collect(),
        )
    }

    pub fn new(observations: Vec<AcceptedObservation>) -> Result<Self, ObservationError> {
        let mut partitions = HashSet::new();
        for observation in &observations {
            let key = observation.partition_key();
            if !partitions.insert(key.clone()) {
                return Err(ObservationError::DuplicatePartition(key));
            }
        }
        Ok(Self { observations })
    }

    pub fn observations(&self) -> &[AcceptedObservation] {
        &self.observations
    }

    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlertPartitionKey {
    pub session_id: String,
    pub canonical_root: PathBuf,
    pub producer_key: ProducerKey,
}

impl AlertPartitionKey {
    pub fn new(
        session_id: impl Into<String>,
        canonical_root: impl AsRef<Path>,
        producer_key: ProducerKey,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            canonical_root: canonicalize_for_alert(canonical_root.as_ref()),
            producer_key,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LifecycleEpisodeId(u64);

impl LifecycleEpisodeId {
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The alert fields owned by exactly one `(session, root, producer)` partition.
/// `rendered` is deliberately partition-local: a snapshot from one producer may
/// never prune or suppress another producer's lifecycle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlertPartitionState {
    pub baseline_established: bool,
    pub live: HashMap<DiagnosticIdentity, LifecycleEpisodeId>,
    pub rendered: HashSet<DiagnosticIdentity>,
    pub closed_episodes: HashSet<LifecycleEpisodeId>,
}

impl AlertPartitionState {
    pub fn episode_for(&self, identity: &DiagnosticIdentity) -> Option<LifecycleEpisodeId> {
        self.live.get(identity).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedIdentity {
    pub identity: DiagnosticIdentity,
    pub episode_id: LifecycleEpisodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnteredIdentity {
    pub identity: DiagnosticIdentity,
    pub episode_id: LifecycleEpisodeId,
}

/// The state transition for one accepted producer snapshot. The first
/// observation establishes a silent baseline, so its `entered` identities are
/// returned separately from later alert-eligible entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedObservationResult {
    pub partition: AlertPartitionKey,
    pub accepted_document_version: i32,
    pub accepted_empty_snapshot: bool,
    pub baseline_established_now: bool,
    pub baselined: Vec<EnteredIdentity>,
    pub entered: Vec<EnteredIdentity>,
    pub closed: Vec<ClosedIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationError {
    DuplicatePartition(AlertPartitionKey),
}

impl std::fmt::Display for ObservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicatePartition(key) => write!(
                formatter,
                "accepted observation batch contains duplicate partition ({}, {}, {})",
                key.session_id,
                key.canonical_root.display(),
                key.producer_key.as_str()
            ),
        }
    }
}

impl std::error::Error for ObservationError {}

/// Session-scoped alert delta state. The caller supplies the authoritative
/// observation batch and an injected monotonic time; no passive diagnostic
/// reads, timers, or heartbeats have a state-mutating API here.
#[derive(Debug, Clone, Default)]
pub struct AlertDeltaState {
    partitions: HashMap<AlertPartitionKey, AlertPartitionState>,
    session_last_touched: HashMap<String, Instant>,
    next_episode_id: u64,
}

impl AlertDeltaState {
    pub fn accept_batch(
        &mut self,
        batch: &AcceptedObservationBatch,
    ) -> Result<Vec<AcceptedObservationResult>, ObservationError> {
        self.accept_batch_at(batch, Instant::now())
    }

    /// Apply every accepted producer snapshot as one terminal transition. The
    /// staging clone means a malformed batch cannot expose an intermediate
    /// multi-producer state to a concurrent finalizer.
    pub fn accept_batch_at(
        &mut self,
        batch: &AcceptedObservationBatch,
        now: Instant,
    ) -> Result<Vec<AcceptedObservationResult>, ObservationError> {
        // Batches built by `AcceptedObservationBatch::new` are already valid,
        // but validate again so deserialization or future constructors cannot
        // accidentally weaken the atomicity guarantee.
        validate_batch(batch)?;

        let mut staged = self.clone();
        let mut results = Vec::with_capacity(batch.observations.len());
        for observation in &batch.observations {
            results.push(staged.accept_observation(observation, now));
        }
        *self = staged;
        Ok(results)
    }

    pub fn partition(&self, key: &AlertPartitionKey) -> Option<&AlertPartitionState> {
        self.partitions.get(key)
    }

    pub fn partitions_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> impl Iterator<Item = (&'a AlertPartitionKey, &'a AlertPartitionState)> + 'a {
        self.partitions
            .iter()
            .filter(move |(key, _)| key.session_id == session_id)
    }

    pub fn remove_session(&mut self, session_id: &str) -> bool {
        let before = self.partitions.len();
        self.partitions
            .retain(|key, _| key.session_id != session_id);
        let touched = self.session_last_touched.remove(session_id).is_some();
        touched || self.partitions.len() != before
    }

    /// Reap all state belonging to sessions idle for at least `idle_for`.
    /// `now` is injected by the runtime to make the lifecycle independently
    /// testable without pinning the product duration.
    pub fn reap_idle_sessions_at(&mut self, now: Instant, idle_for: Duration) -> Vec<String> {
        let mut reaped = self
            .session_last_touched
            .iter()
            .filter_map(|(session_id, last_touched)| {
                (now.saturating_duration_since(*last_touched) >= idle_for)
                    .then(|| session_id.clone())
            })
            .collect::<Vec<_>>();
        reaped.sort();
        for session_id in &reaped {
            self.remove_session(session_id);
        }
        reaped
    }

    pub fn reap_idle_sessions(&mut self) -> Vec<String> {
        self.reap_idle_sessions_at(Instant::now(), ALERT_SESSION_IDLE_TTL)
    }

    fn accept_observation(
        &mut self,
        observation: &AcceptedObservation,
        now: Instant,
    ) -> AcceptedObservationResult {
        let partition_key = observation.partition_key();
        self.session_last_touched
            .insert(observation.session_id.clone(), now);

        let current = observation
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                DiagnosticIdentity::from_stored(&observation.canonical_root, diagnostic)
            })
            .collect::<HashSet<_>>();
        let baseline_established = self
            .partitions
            .get(&partition_key)
            .is_some_and(|partition| partition.baseline_established);

        if !baseline_established {
            let mut baselined = current
                .into_iter()
                .map(|identity| EnteredIdentity {
                    episode_id: self.mint_episode(),
                    identity,
                })
                .collect::<Vec<_>>();
            baselined.sort_by(|left, right| left.identity.cmp(&right.identity));
            let partition = self.partitions.entry(partition_key.clone()).or_default();
            for entered in &baselined {
                partition
                    .live
                    .insert(entered.identity.clone(), entered.episode_id);
                partition.rendered.insert(entered.identity.clone());
            }
            partition.baseline_established = true;
            return AcceptedObservationResult {
                partition: partition_key,
                accepted_document_version: observation.accepted_document_version,
                accepted_empty_snapshot: observation.is_empty(),
                baseline_established_now: true,
                baselined,
                entered: Vec::new(),
                closed: Vec::new(),
            };
        }

        let previous = self
            .partitions
            .get(&partition_key)
            .map(|partition| partition.live.keys().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();
        let closed_identities = previous.difference(&current).cloned().collect::<Vec<_>>();
        let mut entered = current
            .difference(&previous)
            .cloned()
            .map(|identity| EnteredIdentity {
                episode_id: self.mint_episode(),
                identity,
            })
            .collect::<Vec<_>>();
        entered.sort_by(|left, right| left.identity.cmp(&right.identity));

        let partition = self.partitions.entry(partition_key.clone()).or_default();
        let mut closed = closed_identities
            .into_iter()
            .filter_map(|identity| {
                partition.live.remove(&identity).map(|episode_id| {
                    partition.rendered.remove(&identity);
                    partition.closed_episodes.insert(episode_id);
                    ClosedIdentity {
                        identity,
                        episode_id,
                    }
                })
            })
            .collect::<Vec<_>>();
        closed.sort_by(|left, right| left.identity.cmp(&right.identity));
        for item in &entered {
            partition
                .live
                .insert(item.identity.clone(), item.episode_id);
        }

        AcceptedObservationResult {
            partition: partition_key,
            accepted_document_version: observation.accepted_document_version,
            accepted_empty_snapshot: observation.is_empty(),
            baseline_established_now: false,
            baselined: Vec::new(),
            entered,
            closed,
        }
    }

    fn mint_episode(&mut self) -> LifecycleEpisodeId {
        self.next_episode_id = self.next_episode_id.wrapping_add(1).max(1);
        LifecycleEpisodeId(self.next_episode_id)
    }
}

fn validate_batch(batch: &AcceptedObservationBatch) -> Result<(), ObservationError> {
    let mut partitions = HashSet::new();
    for observation in &batch.observations {
        let key = observation.partition_key();
        if !partitions.insert(key.clone()) {
            return Err(ObservationError::DuplicatePartition(key));
        }
    }
    Ok(())
}

fn canonical_root_relative_file(canonical_root: &Path, file: &Path) -> PathBuf {
    let root = canonicalize_for_alert(canonical_root);
    let file = canonicalize_for_alert(file);
    file.strip_prefix(&root).unwrap_or(&file).to_path_buf()
}

fn canonicalize_for_alert(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
