use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::job::{InspectSnapshot, JobOutcome, JobScope};
use crate::config::Config;
use crate::context::AppContext;
use crate::lsp::diagnostics::{DiagnosticSeverity, StoredDiagnostic};
use crate::lsp::manager::{
    EnsureServerOutcomes, InspectDiagnosticsWake, PreEditSnapshot, PullFileOutcome, PullFileResult,
    ServerAttemptResult,
};
use crate::lsp::registry::servers_for_file;
use crate::lsp::roots::ServerKey;
use crate::lsp::tsconfig_membership::TsconfigMembershipCache;

const BLOCKING_DIAGNOSTICS_PHASE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
struct CollectedDiagnostic {
    diagnostic: StoredDiagnostic,
    provisional: bool,
}

#[derive(Default)]
struct DiagnosticsCollection {
    diagnostics: Vec<CollectedDiagnostic>,
    server_ran: bool,
    applicability_is_empty: bool,
    servers_pending: BTreeSet<String>,
    servers_not_installed: BTreeSet<String>,
    /// An explicit unsupported file prevents the inspected set from being
    /// mechanically verified, even though it does not have a server to wait on.
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

/// Collect diagnostics for the explicit inspect path.
///
/// A collection becomes Fresh only after all pending, unavailable, or otherwise
/// unverified diagnostic sources have been resolved. If any such condition
/// remains, the command emits a terminal non-fresh response instead of
/// serializing zero counts or cache-state fields.
pub(crate) fn run_diagnostics_category(
    ctx: &AppContext,
    snapshot: &InspectSnapshot,
    scope: &JobScope,
    scope_was_provided: bool,
    applicability_is_empty: bool,
) -> JobOutcome {
    let collection = if applicability_is_empty {
        // No applicable producer means there is no diagnostic artifact to wait
        // for; the empty category is authoritative for this applicability snapshot.
        DiagnosticsCollection {
            applicability_is_empty: true,
            ..DiagnosticsCollection::default()
        }
    } else if scope_was_provided {
        // A deferred inspection can collect diagnostics for the entire root
        // without an explicit scope. Scope filters rendered results, not work.
        match collect_scoped_diagnostics(ctx, snapshot, scope) {
            Ok(collection) => collection,
            Err(message) => return JobOutcome::Failed { message },
        }
    } else {
        collect_warm_working_set(ctx, snapshot)
    };

    if collection.is_complete() {
        JobOutcome::Fresh {
            payload: collection.into_payload(snapshot),
        }
    } else {
        JobOutcome::Pending { in_flight: true }
    }
}

#[allow(dead_code)]
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
) -> Result<DiagnosticsCollection, String> {
    let deadline = Instant::now() + BLOCKING_DIAGNOSTICS_PHASE_TIMEOUT;
    let config = ctx.config().clone();
    let mut tsconfig_membership = TsconfigMembershipCache::new();
    let whole_root = JobScope::from_roots(
        snapshot.project_root.clone(),
        vec![snapshot.project_root.clone()],
    );
    let scoped = scoped_lsp_files(snapshot, &whole_root, &config, &mut tsconfig_membership);
    let mut files = scoped.files.into_iter().collect::<BTreeSet<_>>();
    for root in scope.roots().iter().filter(|root| root.is_file()) {
        files.insert(crate::inspect::job::canonicalize_normalized(root));
    }
    let mut collection = DiagnosticsCollection {
        applicability_is_empty: files.is_empty() && scoped.explicit_files_without_server == 0,
        files_without_server: scoped.explicit_files_without_server,
        ..DiagnosticsCollection::default()
    };
    let mut opened_documents = ScopedInspectDocuments::new(ctx);

    for file in files {
        check_diagnostics_phase_boundary(deadline)?;
        collect_scoped_file(
            ctx,
            &config,
            &file,
            &mut collection,
            &mut opened_documents,
            deadline,
        )?;
        // Pull-only servers may leave didOpen diagnostics, telemetry, and log
        // notifications queued because they do not enter the push waiter.
        ctx.lsp().drain_events();
    }

    check_diagnostics_phase_boundary(deadline)?;
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
    Ok(collection)
}

fn wait_for_inspect_diagnostics_without_manager_lock(
    ctx: &AppContext,
    file: &Path,
    expected: &[(ServerKey, PreEditSnapshot)],
    deadline: Instant,
) -> Result<(), String> {
    let wait = {
        let mut lsp = ctx.lsp();
        lsp.start_inspect_diagnostics_wait(file, expected)
    };
    let result = loop {
        if {
            let mut lsp = ctx.lsp();
            lsp.poll_inspect_diagnostics_wait(&wait, None)
        } {
            break Ok(());
        }
        if let Err(message) = check_diagnostics_phase_boundary(deadline) {
            break Err(message);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(wake) = wait.next_event_timeout(Duration::from_millis(50).min(remaining)) else {
            continue;
        };
        if matches!(&wake, InspectDiagnosticsWake::Disconnected) {
            break Ok(());
        }
        if {
            let mut lsp = ctx.lsp();
            lsp.poll_inspect_diagnostics_wait(&wait, Some(wake))
        } {
            break Ok(());
        }
    };
    ctx.lsp().finish_inspect_diagnostics_wait(wait);
    result
}

fn check_diagnostics_phase_boundary(deadline: Instant) -> Result<(), String> {
    if crate::executor::current_job_cancellation()
        .is_some_and(|token| token.cancel_requested_before_commit())
    {
        return Err("inspect request cancelled during LSP quiescence".to_string());
    }
    if Instant::now() >= deadline {
        return Err(format!(
            "lsp_quiescence_timeout: diagnostics did not complete within {}s",
            BLOCKING_DIAGNOSTICS_PHASE_TIMEOUT.as_secs()
        ));
    }
    Ok(())
}

fn collect_scoped_file(
    ctx: &AppContext,
    config: &Config,
    file: &Path,
    collection: &mut DiagnosticsCollection,
    opened_documents: &mut ScopedInspectDocuments<'_>,
    deadline: Instant,
) -> Result<(), String> {
    check_diagnostics_phase_boundary(deadline)?;
    // One canonical form across the LSP boundary: bare fs::canonicalize yields
    // verbatim paths on Windows, which no longer match normalized server keys.
    let canonical = crate::inspect::job::canonicalize_normalized(file);
    let outcomes: EnsureServerOutcomes = {
        let mut lsp = ctx.lsp();
        lsp.ensure_server_for_file_detailed(&canonical, config)
    };

    record_attempt_gaps(&outcomes, collection);
    if outcomes.only_inapplicable_root_markers() {
        collection.files_without_server += 1;
        return Ok(());
    }
    if outcomes.no_server_registered() {
        collection.files_without_server += 1;
        return Ok(());
    }
    if outcomes.successful.is_empty() {
        return Ok(());
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
    check_diagnostics_phase_boundary(deadline)?;

    let push_fallback_servers =
        record_pull_results(&outcomes.successful, &pull_results, collection);
    if push_fallback_servers.is_empty() {
        return Ok(());
    }

    let expected = push_fallback_servers
        .iter()
        .map(|key| {
            (
                key.clone(),
                pre_push_snapshot.get(key).copied().unwrap_or_default(),
            )
        })
        .collect::<Vec<(ServerKey, PreEditSnapshot)>>();
    wait_for_inspect_diagnostics_without_manager_lock(ctx, &canonical, &expected, deadline)?;

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
    Ok(())
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
    /// Count of explicit file roots with no registered LSP server. Directory
    /// walks skip non-code files, but an explicitly requested unsupported file
    /// prevents a fresh diagnostics claim.
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
            if tsconfig_membership.should_skip_diagnostics(path)
                || servers_for_file(path, config).is_empty()
            {
                continue;
            }
            files.insert(crate::inspect::job::canonicalize_normalized(path));
        }
    }

    ScopedLspFiles {
        files: files.into_iter().collect(),
        explicit_files_without_server,
    }
}

impl DiagnosticsCollection {
    fn is_complete(&self) -> bool {
        (self.server_ran || self.applicability_is_empty)
            && self.servers_pending.is_empty()
            && self.servers_not_installed.is_empty()
            && self.files_without_server == 0
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.provisional)
    }

    fn into_payload(mut self, snapshot: &InspectSnapshot) -> Value {
        debug_assert!(self.is_complete());
        self.sort_and_dedup();
        let (errors, warnings, info, hints) = severity_counts(&self.diagnostics);
        let items = self
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic_item(snapshot, diagnostic))
            .collect::<Vec<_>>();

        serde_json::json!({
            "errors": errors,
            "warnings": warnings,
            "info": info,
            "hints": hints,
            "items": items,
        })
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

fn severity_counts(diagnostics: &[CollectedDiagnostic]) -> (usize, usize, usize, usize) {
    severity_counts_filtered(diagnostics, |diagnostic| !diagnostic.provisional)
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
                    message: "verified result".into(),
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
    fn empty_applicability_is_vacuously_complete() {
        let collection = DiagnosticsCollection {
            applicability_is_empty: true,
            ..DiagnosticsCollection::default()
        };

        assert!(collection.is_complete());
    }

    #[test]
    fn incomplete_collection_cannot_be_promoted_to_a_payload() {
        let mut collection = collection();
        collection.servers_pending.insert("rust-analyzer".into());
        assert!(!collection.is_complete());
    }

    #[test]
    fn provisional_collection_cannot_be_promoted_to_a_payload() {
        let mut collection = collection();
        collection.diagnostics[0].provisional = true;
        assert!(!collection.is_complete());
    }

    #[test]
    fn fresh_payload_contains_only_authoritative_counts_and_items() {
        let payload = collection().into_payload(&snapshot());
        assert_eq!(payload["errors"], 1);
        assert_eq!(payload["warnings"], 0);
        assert!(payload.get("server_ran").is_none());
        assert!(payload.get("complete").is_none());
        assert!(payload.get("status").is_none());
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
