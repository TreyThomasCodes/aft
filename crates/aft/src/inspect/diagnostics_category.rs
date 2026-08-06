use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::job::{InspectSnapshot, JobOutcome, JobScope};
use crate::config::Config;
use crate::context::AppContext;
use crate::lsp::diagnostics::{DiagnosticSeverity, StoredDiagnostic};
use crate::lsp::manager::{
    EnsureServerOutcomes, PullFileOutcome, PullFileResult, ServerAttemptResult,
};
use crate::lsp::registry::servers_for_file;
use crate::lsp::roots::ServerKey;
use crate::lsp::tsconfig_membership::TsconfigMembershipCache;

/// How long a SCOPED diagnostics pull waits for the LSP server to become ready
/// and publish before reporting `pending`. Only the scoped (active-pull) path
/// uses this — no-scope warm reads never wait.
///
/// 1s was too short for a cold language server: `ensure_server_for_file_detailed`
/// spawns the server asynchronously, so the first scoped call on a fresh bridge
/// almost always hit the deadline before the server finished initializing and
/// returned `pending`, forcing the agent to re-run. When an agent explicitly
/// scopes to a file it is asking "what's wrong with this" — it should get the
/// answer in one call. 8s covers typical tsserver/rust-analyzer cold start while
/// staying well under the 30s bridge transport timeout. The wait is bounded and
/// only the FIRST scoped call per server pays the cold-start cost (subsequent
/// calls hit a warm server). Tradeoff: diagnostics run on the single-threaded
/// dispatch loop (the LSP manager is `!Send`), so this wait blocks other requests
/// on the same bridge for its duration — acceptable because it is bounded and
/// cold-start-only. A genuinely slow/unresponsive server still falls back to an
/// honest `pending` at the deadline. For a directory scope the budget is shared
/// across files, so the first cold file warms the server and the rest resolve
/// within the remaining budget (or are reported truncated, honestly).
const INSPECT_DIAGNOSTICS_DEADLINE: Duration = Duration::from_secs(8);
const SCOPED_FILE_CAP: usize = 200;

#[derive(Debug, Clone)]
struct CollectedDiagnostic {
    diagnostic: StoredDiagnostic,
    provisional: bool,
}

#[derive(Default)]
struct DiagnosticsCollection {
    diagnostics: Vec<CollectedDiagnostic>,
    server_ran: bool,
    servers_pending: BTreeSet<String>,
    servers_not_installed: BTreeSet<String>,
    scope_truncated: bool,
    /// Number of scoped files whose extension has NO registered LSP server.
    /// These files will never produce diagnostics — distinct from "pending"
    /// (a server is running but hasn't reported yet). Without this, a scope of
    /// only unsupported file types reported `status: "pending"` forever,
    /// implying results were still coming when none ever would.
    files_without_server: usize,
}

struct ScopedInspectDocuments<'a> {
    ctx: &'a AppContext,
    opened: Vec<(PathBuf, Vec<ServerKey>)>,
}

impl<'a> ScopedInspectDocuments<'a> {
    fn new(ctx: &'a AppContext) -> Self {
        Self {
            ctx,
            opened: Vec::new(),
        }
    }

    fn record(&mut self, file: PathBuf, server_keys: Vec<ServerKey>) {
        if !server_keys.is_empty() {
            self.opened.push((file, server_keys));
        }
    }
}

impl Drop for ScopedInspectDocuments<'_> {
    fn drop(&mut self) {
        let mut lsp = self.ctx.lsp();
        for (file, server_keys) in std::mem::take(&mut self.opened) {
            if let Err(err) = lsp.close_file_for_servers(&file, &server_keys) {
                crate::slog_warn!(
                    "[inspect:diagnostics] failed to close scoped document {}: {err}",
                    file.display()
                );
            }
        }
    }
}

/// Main-thread implementation for the `diagnostics` inspect category.
///
/// The LSP manager is owned by `AppContext` and is part of the serial LSP/status
/// lane, so this category must never be dispatched through the rayon inspect
/// worker pool. `handle_inspect` calls this directly, alongside the Tier-1 reads,
/// while Tier-2 categories keep using the cache/worker path.
pub(crate) fn run_diagnostics_category(
    ctx: &AppContext,
    snapshot: &InspectSnapshot,
    scope: &JobScope,
    scope_was_provided: bool,
) -> JobOutcome {
    let collection = if scope_was_provided {
        collect_scoped_diagnostics(ctx, snapshot, scope)
    } else {
        collect_warm_working_set(ctx, snapshot)
    };

    JobOutcome::Fresh {
        payload: collection.into_payload(snapshot),
    }
}

fn collect_warm_working_set(ctx: &AppContext, snapshot: &InspectSnapshot) -> DiagnosticsCollection {
    let mut collection = DiagnosticsCollection::default();
    let mut tsconfig_membership = TsconfigMembershipCache::new();
    {
        let mut lsp = ctx.lsp();
        // No-scope inspect is intentionally cheap: drain already queued LSP
        // events, then read only the warm diagnostics store. It does not open
        // files or spawn servers.
        lsp.drain_events();
        collection.server_ran = lsp.has_any_diagnostic_reports();
        if !collection.server_ran {
            collection.servers_pending.extend(
                lsp.active_server_keys()
                    .into_iter()
                    .map(|key| server_id(&key)),
            );
        }
        collection.servers_pending.extend(
            lsp.provisional_server_keys()
                .into_iter()
                .map(|key| server_id(&key)),
        );
        collection.diagnostics = lsp
            .get_all_diagnostics_with_provisional()
            .into_iter()
            .map(|(diagnostic, provisional)| CollectedDiagnostic {
                diagnostic: diagnostic.clone(),
                provisional,
            })
            .collect();
    }

    collection.diagnostics.retain(|diagnostic| {
        diagnostic
            .diagnostic
            .file
            .starts_with(&snapshot.project_root)
            && !tsconfig_membership.should_skip_diagnostics(&diagnostic.diagnostic.file)
    });
    collection.sort_and_dedup();
    collection
}

fn collect_scoped_diagnostics(
    ctx: &AppContext,
    snapshot: &InspectSnapshot,
    scope: &JobScope,
) -> DiagnosticsCollection {
    collect_scoped_diagnostics_until(
        ctx,
        snapshot,
        scope,
        Instant::now() + INSPECT_DIAGNOSTICS_DEADLINE,
    )
}

fn collect_scoped_diagnostics_until(
    ctx: &AppContext,
    snapshot: &InspectSnapshot,
    scope: &JobScope,
    deadline: Instant,
) -> DiagnosticsCollection {
    let config = ctx.config().clone();
    let mut tsconfig_membership = TsconfigMembershipCache::new();
    let scoped = scoped_lsp_files(snapshot, scope, &config, &mut tsconfig_membership);
    let files = scoped.files;
    let mut collection = DiagnosticsCollection {
        scope_truncated: scoped.truncated,
        files_without_server: scoped.explicit_files_without_server,
        ..DiagnosticsCollection::default()
    };
    let mut opened_documents = ScopedInspectDocuments::new(ctx);

    for file in files {
        if Instant::now() >= deadline {
            collection.scope_truncated = true;
            break;
        }
        collect_scoped_file(
            ctx,
            &config,
            &file,
            deadline,
            &mut collection,
            &mut opened_documents,
        );
        // Pull-only servers may leave didOpen diagnostics, telemetry, and log
        // notifications queued because they do not enter the push waiter.
        ctx.lsp().drain_events();
    }

    collection.diagnostics =
        scoped_warm_diagnostics(ctx, snapshot, scope, &mut tsconfig_membership);
    collection.servers_pending.extend(
        ctx.lsp()
            .provisional_server_keys()
            .into_iter()
            .map(|key| server_id(&key)),
    );
    collection.sort_and_dedup();
    drop(opened_documents);
    collection
}

#[doc(hidden)]
pub fn run_scoped_diagnostics_with_deadline_for_test(
    ctx: &AppContext,
    snapshot: &InspectSnapshot,
    scope: &JobScope,
    timeout: Duration,
) -> JobOutcome {
    let collection =
        collect_scoped_diagnostics_until(ctx, snapshot, scope, Instant::now() + timeout);
    JobOutcome::Fresh {
        payload: collection.into_payload(snapshot),
    }
}

fn collect_scoped_file(
    ctx: &AppContext,
    config: &Config,
    file: &Path,
    deadline: Instant,
    collection: &mut DiagnosticsCollection,
    opened_documents: &mut ScopedInspectDocuments<'_>,
) {
    // One canonical form across the LSP boundary: bare fs::canonicalize
    // yields verbatim paths on Windows, which no longer match the
    // normalized server keys and diagnostics-store paths.
    let canonical = crate::inspect::job::canonicalize_normalized(file);
    let outcomes: EnsureServerOutcomes = {
        let mut lsp = ctx.lsp();
        lsp.ensure_server_for_file_detailed(&canonical, config)
    };

    record_attempt_gaps(&outcomes, collection);
    if outcomes.only_inapplicable_root_markers() {
        // Every server registered for this file's extension failed the root
        // marker check (e.g. a `.ts` file in a project with no `.oxlintrc.json`
        // for oxlint). No applicable server will ever answer for this file, so
        // count it as a no-server file — otherwise the status falls through to
        // "pending" forever even after every truly-applicable server answered.
        collection.files_without_server += 1;
        return;
    }
    if outcomes.no_server_registered() || outcomes.successful.is_empty() {
        // No-server files are already excluded from the candidate set by
        // scoped_lsp_files (which counts explicit file-roots into
        // files_without_server); reaching here means the server exists but
        // isn't ready, which record_attempt_gaps already tracked.
        return;
    }

    let pre_push_snapshot = {
        let lsp = ctx.lsp();
        lsp.snapshot_pre_edit_state(&canonical)
    };

    let pull_results = {
        let mut lsp = ctx.lsp();
        match lsp.pull_file_diagnostics_tracked(&canonical, config) {
            Ok(tracked) => {
                opened_documents.record(canonical.clone(), tracked.newly_opened);
                tracked.results
            }
            Err(err) => {
                crate::slog_warn!(
                    "[inspect:diagnostics] pull_file_diagnostics failed for {}: {err}",
                    canonical.display()
                );
                for key in &outcomes.successful {
                    collection.servers_pending.insert(server_id(key));
                }
                Vec::new()
            }
        }
    };

    let push_fallback_servers =
        record_pull_results(&outcomes.successful, &pull_results, collection);
    if push_fallback_servers.is_empty() {
        return;
    }

    if Instant::now() < deadline {
        let mut lsp = ctx.lsp();
        let _ = lsp.wait_for_file_diagnostics(&canonical, config, deadline);
    }

    let lsp = ctx.lsp();
    for key in push_fallback_servers {
        let pre = pre_push_snapshot.get(&key).copied().unwrap_or_default();
        if lsp.diagnostic_entry_is_fresh_for_document(&canonical, &key, pre)
            || lsp.has_diagnostic_report_for_server_file(&key, &canonical)
        {
            collection.server_ran = true;
        } else {
            collection.servers_pending.insert(server_id(&key));
        }
    }
}

fn record_attempt_gaps(outcomes: &EnsureServerOutcomes, collection: &mut DiagnosticsCollection) {
    for attempt in &outcomes.attempts {
        match &attempt.result {
            ServerAttemptResult::Ok { .. } => {}
            ServerAttemptResult::BinaryNotInstalled { .. } => {
                collection
                    .servers_not_installed
                    .insert(attempt.server_id.clone());
            }
            ServerAttemptResult::SpawnFailed { .. } => {
                collection
                    .servers_not_installed
                    .insert(attempt.server_id.clone());
            }
            ServerAttemptResult::NoRootMarker { .. } => {
                // The server is registered for this file's extension but none of
                // its root markers exist in the project (e.g. oxlint registered
                // for `.ts` with no `.oxlintrc.json`). That's a filesystem fact
                // that never changes mid-scan — the server simply does not apply
                // to this project. It is NOT "pending" (results are never
                // coming), so treating it as a gap would leave scoped diagnostics
                // reporting `pending` forever even after every applicable server
                // answered. Ignore it. The all-not-applicable edge (a file whose
                // only registered servers all lack root markers) is handled by
                // the caller, which counts it into `files_without_server`.
            }
        }
    }
}

fn record_pull_results(
    expected_servers: &[ServerKey],
    pull_results: &[PullFileResult],
    collection: &mut DiagnosticsCollection,
) -> Vec<ServerKey> {
    let mut push_fallback_servers = Vec::new();

    for key in expected_servers {
        let Some(result) = pull_results.iter().find(|result| result.server_key == *key) else {
            collection.servers_pending.insert(server_id(key));
            continue;
        };

        match &result.outcome {
            PullFileOutcome::Full { .. } | PullFileOutcome::Unchanged => {
                collection.server_ran = true;
            }
            PullFileOutcome::PullNotSupported => {
                push_fallback_servers.push(key.clone());
            }
            PullFileOutcome::RequestFailed { reason } if request_failure_needs_push(reason) => {
                push_fallback_servers.push(key.clone());
            }
            PullFileOutcome::PartialNotSupported | PullFileOutcome::RequestFailed { .. } => {
                collection.servers_pending.insert(server_id(key));
            }
        }
    }

    push_fallback_servers
}

fn request_failure_needs_push(reason: &str) -> bool {
    reason == "no_cache_for_unchanged" || reason.starts_with("pull_rejected_push_fallback:")
}

fn scoped_warm_diagnostics(
    ctx: &AppContext,
    snapshot: &InspectSnapshot,
    scope: &JobScope,
    tsconfig_membership: &mut TsconfigMembershipCache,
) -> Vec<CollectedDiagnostic> {
    let roots = if scope.roots().is_empty() {
        vec![snapshot.project_root.clone()]
    } else {
        scope.roots().to_vec()
    };

    let lsp = ctx.lsp();
    roots
        .iter()
        .flat_map(|root| {
            let diagnostics = if root.is_file() {
                lsp.get_diagnostics_for_file_with_provisional(root)
            } else {
                lsp.get_diagnostics_for_directory_with_provisional(root)
            };
            diagnostics
                .into_iter()
                .filter(|(diagnostic, _)| {
                    scope.contains(&diagnostic.file)
                        && diagnostic.file.starts_with(&snapshot.project_root)
                        && !tsconfig_membership.should_skip_diagnostics(&diagnostic.file)
                })
                .map(|(diagnostic, provisional)| CollectedDiagnostic {
                    diagnostic: diagnostic.clone(),
                    provisional,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

struct ScopedLspFiles {
    files: Vec<PathBuf>,
    truncated: bool,
    /// Count of explicit file-roots in the scope that have no registered LSP
    /// server. Directory walks intentionally skip non-code files silently
    /// (you don't want a `.md` in a walked dir flagged), but a scope that names
    /// a specific file we cannot diagnose is a real "no server" signal the
    /// agent must see — otherwise the status reads "pending" forever.
    explicit_files_without_server: usize,
}

fn scoped_lsp_files(
    snapshot: &InspectSnapshot,
    scope: &JobScope,
    config: &Config,
    tsconfig_membership: &mut TsconfigMembershipCache,
) -> ScopedLspFiles {
    let roots = if scope.roots().is_empty() {
        vec![snapshot.project_root.clone()]
    } else {
        scope.roots().to_vec()
    };

    let mut files = BTreeSet::new();
    let mut truncated = false;
    let mut explicit_files_without_server = 0usize;
    for root in roots {
        if root.is_file() {
            if tsconfig_membership.should_skip_diagnostics(&root) {
                continue;
            }
            if servers_for_file(&root, config).is_empty() {
                explicit_files_without_server += 1;
                continue;
            }
            files.insert(crate::inspect::job::canonicalize_normalized(&root));
            continue;
        }

        let walker = ignore::WalkBuilder::new(&root)
            .standard_filters(true)
            .add_custom_ignore_filename(".aftignore")
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                !matches!(
                    name.as_ref(),
                    ".git" | "node_modules" | "target" | "dist" | "build" | ".next" | ".turbo"
                )
            })
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                continue;
            }
            let path = entry.path();
            if tsconfig_membership.should_skip_diagnostics(path) {
                continue;
            }
            if servers_for_file(path, config).is_empty() {
                continue;
            }
            files.insert(crate::inspect::job::canonicalize_normalized(path));
            if files.len() >= SCOPED_FILE_CAP {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    ScopedLspFiles {
        files: files.into_iter().collect(),
        truncated,
        explicit_files_without_server,
    }
}

impl DiagnosticsCollection {
    fn into_payload(mut self, snapshot: &InspectSnapshot) -> Value {
        self.sort_and_dedup();
        let authoritative = severity_counts(&self.diagnostics);
        let all = severity_counts_including_provisional(&self.diagnostics);
        let provisional_only = severity_counts_provisional_only(&self.diagnostics);
        let complete = self.server_ran
            && self.servers_pending.is_empty()
            && self.servers_not_installed.is_empty()
            && !self.scope_truncated;
        let status = diagnostics_status(
            complete,
            self.scope_truncated,
            &self.servers_not_installed,
            &self.servers_pending,
            self.files_without_server,
        );
        let counts_are_provisional = matches!(status, Some("incomplete" | "pending"));
        let (errors, warnings, info, hints) = if counts_are_provisional {
            (0, 0, 0, 0)
        } else {
            authoritative
        };
        let provisional_counts = if counts_are_provisional {
            Some(all)
        } else if provisional_only != (0, 0, 0, 0) {
            Some(provisional_only)
        } else {
            None
        };
        let items = self
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic_item(snapshot, diagnostic))
            .collect::<Vec<_>>();

        let mut payload = serde_json::json!({
            "errors": errors,
            "warnings": warnings,
            "info": info,
            "hints": hints,
            "server_ran": self.server_ran,
            "complete": complete,
            "status": status,
            "servers_pending": self.servers_pending.into_iter().collect::<Vec<_>>(),
            "servers_not_installed": self.servers_not_installed.into_iter().collect::<Vec<_>>(),
            "files_without_server": self.files_without_server,
            "items": items,
        });
        if let Some((errors, warnings, info, hints)) = provisional_counts {
            payload["provisional_counts"] = serde_json::json!({
                "errors": errors,
                "warnings": warnings,
                "info": info,
                "hints": hints,
            });
        }
        payload
    }

    fn sort_and_dedup(&mut self) {
        self.diagnostics.sort_by(|left, right| {
            left.diagnostic
                .file
                .cmp(&right.diagnostic.file)
                .then(left.diagnostic.line.cmp(&right.diagnostic.line))
                .then(left.diagnostic.column.cmp(&right.diagnostic.column))
                .then(left.diagnostic.end_line.cmp(&right.diagnostic.end_line))
                .then(left.diagnostic.end_column.cmp(&right.diagnostic.end_column))
                .then(left.diagnostic.severity.as_str().cmp(right.diagnostic.severity.as_str()))
                .then(left.diagnostic.message.cmp(&right.diagnostic.message))
                .then(left.diagnostic.source.cmp(&right.diagnostic.source))
                // Prefer an authoritative copy when multiple servers report the
                // same diagnostic, so detail rows do not retain a warming tag.
                .then(left.provisional.cmp(&right.provisional))
        });
        self.diagnostics.dedup_by(|left, right| {
            left.diagnostic.file == right.diagnostic.file
                && left.diagnostic.line == right.diagnostic.line
                && left.diagnostic.column == right.diagnostic.column
                && left.diagnostic.end_line == right.diagnostic.end_line
                && left.diagnostic.end_column == right.diagnostic.end_column
                && left.diagnostic.severity == right.diagnostic.severity
                && left.diagnostic.message == right.diagnostic.message
                && left.diagnostic.source == right.diagnostic.source
        });
    }
}

fn diagnostics_status(
    complete: bool,
    scope_truncated: bool,
    servers_not_installed: &BTreeSet<String>,
    servers_pending: &BTreeSet<String>,
    files_without_server: usize,
) -> Option<&'static str> {
    if complete {
        None
    } else if scope_truncated || !servers_not_installed.is_empty() {
        // Bounded gap: truncated scope, or a server exists but isn't installed.
        Some("incomplete")
    } else if !servers_pending.is_empty() {
        // A registered server is running but hasn't reported yet — results are
        // genuinely still coming.
        Some("pending")
    } else if files_without_server > 0 {
        // No registered server matched the scoped file type(s). Nothing will
        // ever arrive — report that honestly instead of "pending" forever.
        Some("no_server")
    } else {
        // Not complete, but no pending server and no unsupported files either:
        // treat as still-settling rather than asserting completeness.
        Some("pending")
    }
}

fn severity_counts(diagnostics: &[CollectedDiagnostic]) -> (usize, usize, usize, usize) {
    severity_counts_filtered(diagnostics, |diagnostic| !diagnostic.provisional)
}

fn severity_counts_including_provisional(
    diagnostics: &[CollectedDiagnostic],
) -> (usize, usize, usize, usize) {
    severity_counts_filtered(diagnostics, |_| true)
}

fn severity_counts_provisional_only(
    diagnostics: &[CollectedDiagnostic],
) -> (usize, usize, usize, usize) {
    severity_counts_filtered(diagnostics, |diagnostic| diagnostic.provisional)
}

fn severity_counts_filtered(
    diagnostics: &[CollectedDiagnostic],
    include: impl Fn(&CollectedDiagnostic) -> bool,
) -> (usize, usize, usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    let mut info = 0;
    let mut hints = 0;

    for diagnostic in diagnostics {
        if !include(diagnostic)
            || crate::lsp::environmental::is_environmental_diagnostic(&diagnostic.diagnostic)
        {
            continue;
        }
        match diagnostic.diagnostic.severity {
            DiagnosticSeverity::Error => errors += 1,
            DiagnosticSeverity::Warning => warnings += 1,
            DiagnosticSeverity::Information => info += 1,
            DiagnosticSeverity::Hint => hints += 1,
        }
    }

    (errors, warnings, info, hints)
}

/// Detail-row message for `aft_inspect` items (file:line:col severity message).
/// Environmental and warming diagnostics are tagged so summary counts and
/// listed rows explain why a row is excluded from authoritative totals.
fn diagnostic_detail_message(diagnostic: &CollectedDiagnostic) -> String {
    let mut message = diagnostic.diagnostic.message.clone();
    if crate::lsp::environmental::is_environmental_diagnostic(&diagnostic.diagnostic) {
        message.push_str(" [environmental]");
    }
    if diagnostic.provisional {
        message.push_str(" (analyzer warming)");
    }
    message
}

fn diagnostic_item(snapshot: &InspectSnapshot, diagnostic: &CollectedDiagnostic) -> Value {
    serde_json::json!({
        "file": display_path(snapshot, &diagnostic.diagnostic.file),
        "line": diagnostic.diagnostic.line,
        "column": diagnostic.diagnostic.column,
        "severity": diagnostic.diagnostic.severity.as_str(),
        "message": diagnostic_detail_message(diagnostic),
        "source": diagnostic.diagnostic.source.as_deref().unwrap_or("lsp"),
    })
}

fn display_path(snapshot: &InspectSnapshot, path: &Path) -> String {
    path.strip_prefix(&snapshot.project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn server_id(key: &ServerKey) -> String {
    key.kind.id_str().to_string()
}

#[cfg(test)]
mod payload_count_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    use super::{CollectedDiagnostic, DiagnosticsCollection};
    use crate::config::Config;
    use crate::inspect::job::InspectSnapshot;
    use crate::lsp::diagnostics::{DiagnosticSeverity, StoredDiagnostic};
    use crate::parser::SymbolCache;

    fn snapshot() -> InspectSnapshot {
        InspectSnapshot::new(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.aft"),
            Arc::new(Config::default()),
            Arc::new(RwLock::new(SymbolCache::new())),
        )
    }

    fn collection() -> DiagnosticsCollection {
        DiagnosticsCollection {
            diagnostics: vec![CollectedDiagnostic {
                diagnostic: StoredDiagnostic {
                    file: PathBuf::from("/repo/src/main.rs"),
                    line: 1,
                    column: 1,
                    end_line: 1,
                    end_column: 2,
                    severity: DiagnosticSeverity::Error,
                    message: "warming result".into(),
                    code: None,
                    source: None,
                },
                provisional: false,
            }],
            server_ran: true,
            ..DiagnosticsCollection::default()
        }
    }

    #[test]
    fn pending_counts_are_provisional_and_authoritative_counts_are_zero() {
        let mut collection = collection();
        collection.servers_pending.insert("rust-analyzer".into());
        let payload = collection.into_payload(&snapshot());
        assert_eq!(payload["status"], "pending");
        assert_eq!(payload["errors"], 0);
        assert_eq!(payload["warnings"], 0);
        assert_eq!(payload["provisional_counts"]["errors"], 1);
        assert_eq!(payload["provisional_counts"]["warnings"], 0);
    }

    #[test]
    fn incomplete_counts_are_provisional_and_authoritative_counts_are_zero() {
        let mut collection = collection();
        collection.scope_truncated = true;
        let payload = collection.into_payload(&snapshot());
        assert_eq!(payload["status"], "incomplete");
        assert_eq!(payload["errors"], 0);
        assert_eq!(payload["provisional_counts"]["errors"], 1);
    }

    #[test]
    fn complete_counts_stay_at_the_authoritative_top_level() {
        let payload = collection().into_payload(&snapshot());
        assert_eq!(payload["complete"], true);
        assert_eq!(payload["errors"], 1);
        assert!(payload.get("provisional_counts").is_none());
    }

    #[test]
    fn no_server_counts_stay_at_the_authoritative_top_level() {
        let mut collection = collection();
        collection.server_ran = false;
        collection.files_without_server = 1;
        let payload = collection.into_payload(&snapshot());
        assert_eq!(payload["status"], "no_server");
        assert_eq!(payload["errors"], 1);
        assert!(payload.get("provisional_counts").is_none());
    }
}

#[cfg(test)]
mod environmental_count_tests {
    use std::path::PathBuf;

    use super::{severity_counts, CollectedDiagnostic};
    use crate::lsp::diagnostics::{DiagnosticSeverity, StoredDiagnostic};

    fn diag(line: u32, message: &str) -> StoredDiagnostic {
        StoredDiagnostic {
            file: PathBuf::from("/repo/src/mixed.ts"),
            line,
            column: 1,
            end_line: line,
            end_column: 2,
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            code: None,
            source: None,
        }
    }

    #[test]
    fn severity_counts_exclude_environmental_on_same_file() {
        let diagnostics = vec![
            CollectedDiagnostic {
                diagnostic: diag(1, "Cannot find name 'x'."),
                provisional: false,
            },
            CollectedDiagnostic {
                diagnostic: diag(
                    2,
                    "Failed to load schema from https://example.com/schema.json",
                ),
                provisional: false,
            },
        ];
        let (errors, warnings, _, _) = severity_counts(&diagnostics);
        assert_eq!(
            errors, 1,
            "inspect summary must count only non-environmental errors"
        );
        assert_eq!(warnings, 0);
    }
}

#[cfg(test)]
mod environmental_render_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    use super::diagnostic_item;
    use crate::config::Config;
    use crate::inspect::job::InspectSnapshot;
    use crate::lsp::diagnostics::{DiagnosticSeverity, StoredDiagnostic};
    use crate::parser::SymbolCache;

    fn snapshot() -> InspectSnapshot {
        InspectSnapshot::new(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.aft"),
            Arc::new(Config::default()),
            Arc::new(RwLock::new(SymbolCache::new())),
        )
    }

    fn stored(message: &str) -> StoredDiagnostic {
        StoredDiagnostic {
            file: PathBuf::from("/repo/package.json"),
            line: 2,
            column: 5,
            end_line: 2,
            end_column: 6,
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            code: None,
            source: Some("json".into()),
        }
    }

    #[test]
    fn detail_row_tags_environmental_message() {
        let item = diagnostic_item(
            &snapshot(),
            &super::CollectedDiagnostic {
                diagnostic: stored("Failed to load schema from https://example.com/schema.json"),
                provisional: false,
            },
        );
        assert_eq!(
            item["message"].as_str(),
            Some("Failed to load schema from https://example.com/schema.json [environmental]")
        );
    }

    #[test]
    fn detail_row_leaves_real_errors_untagged() {
        let item = diagnostic_item(
            &snapshot(),
            &super::CollectedDiagnostic {
                diagnostic: stored("Cannot find name 'typo'."),
                provisional: false,
            },
        );
        assert_eq!(item["message"].as_str(), Some("Cannot find name 'typo'."));
    }

    #[test]
    fn detail_row_tags_warming_diagnostics() {
        let item = diagnostic_item(
            &snapshot(),
            &super::CollectedDiagnostic {
                diagnostic: stored("temporary analyzer result"),
                provisional: true,
            },
        );
        assert_eq!(
            item["message"].as_str(),
            Some("temporary analyzer result (analyzer warming)")
        );
    }
}
