use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

use serde_json::{Map, Value};

use crate::alert_state::{AcceptedDiagnosticSnapshot, AcceptedObservationBatch};
use crate::context::AppContext;
use crate::inspect::diagnostics_category::run_diagnostics_category;
use crate::inspect::{
    format_wait_text, InspectCache, InspectCategory, InspectPhaseEntry, InspectPhaseId,
    InspectPhaseLog, InspectSnapshot, JobOutcome, JobScope,
};
use crate::lsp::manager::{
    ApplicabilityResolutionError, ApplicableServerSnapshot, ApplicableServerStartError,
};
use crate::protocol::{RawRequest, Response};
use crate::response_finalize::{DispatchOutcome, PendingResponse};

const DEFAULT_TOP_K: usize = 20;
const MAX_TOP_K: usize = 100;

pub fn handle_inspect(req: &RawRequest, ctx: &AppContext) -> Response {
    handle_inspect_payload(req, ctx, false, false)
}

pub fn handle_inspect_tool_call(req: &RawRequest, ctx: &AppContext) -> Response {
    let phase_log = InspectPhaseLog::for_request(req.id.clone());
    let snapshot = match inspect_preflight(req, ctx) {
        Ok(snapshot) => snapshot,
        Err(response) => {
            let detail = response
                .data
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            return build_inspect_terminal(
                &req.id,
                &phase_log,
                InspectTerminal::PhaseFailed {
                    failed_phase: None,
                    failure_reason: "root_resolution_failed",
                    failure_detail: detail,
                },
            );
        }
    };
    let applicability = {
        let lsp = ctx.lsp();
        lsp.resolve_applicable_servers_for_root(&snapshot.project_root, &snapshot.config)
    };
    match applicability {
        Ok(applicability) => run_blocking_inspect_body(req, ctx, applicability, phase_log),
        Err(error) => build_inspect_terminal(
            &req.id,
            &phase_log,
            InspectTerminal::PhaseFailed {
                failed_phase: None,
                failure_reason: applicability_failure_reason(&error),
                failure_detail: Some(applicability_failure_detail(error)),
            },
        ),
    }
}

/// Blocking inspections collect diagnostics for the entire root, even without
/// an explicit response scope. Scope controls rendered results, not freshness.
fn handle_inspect_payload(
    req: &RawRequest,
    ctx: &AppContext,
    force_root_diagnostics: bool,
    applicability_is_empty: bool,
) -> Response {
    let top_k = match parse_top_k(&req.params) {
        Ok(top_k) => top_k,
        Err(message) => return invalid_request(&req.id, message),
    };
    let sections = match parse_sections(req.params.get("sections")) {
        Ok(sections) => sections,
        Err(message) => return invalid_request(&req.id, message),
    };

    let scope_was_provided = scope_was_provided(req.params.get("scope"));
    let snapshot = match build_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(response) => return response.with_id(&req.id),
    };
    let scope = match parse_scope(req, ctx, &snapshot.project_root) {
        Ok(scope) => scope,
        Err(response) => return response,
    };

    let manager = ctx.inspect_manager();
    let mut tier2_receivers = BTreeMap::new();
    for category in InspectCategory::active()
        .iter()
        .copied()
        .filter(|category| category.is_tier2())
    {
        if !ctx.inspect_writer() {
            continue;
        }
        let manager = manager.clone();
        let snapshot = snapshot.clone();
        let scope = scope.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = if force_root_diagnostics {
                manager.tier2_run_with_reuse_blocking_fresh(snapshot, category, scope)
            } else {
                manager.tier2_run_with_reuse_blocking(snapshot, category, scope)
            };
            let _ = tx.send(outcome);
        });
        tier2_receivers.insert(category, rx);
    }

    let mut outcomes = BTreeMap::new();
    for category in InspectCategory::active() {
        let outcome = if *category == InspectCategory::Diagnostics {
            // Diagnostics use the serial LSP lane rather than the inspect worker
            // pool. A non-authoritative collection remains a non-fresh outcome;
            // it is never converted into a partial inspect payload below.
            run_diagnostics_category(
                ctx,
                &snapshot,
                &scope,
                scope_was_provided || force_root_diagnostics,
                applicability_is_empty,
            )
        } else if category.is_tier2() {
            if let Some(rx) = tier2_receivers.remove(category) {
                receive_tier2_completion(rx)
            } else {
                // A read-only daemon may serve a cached aggregate only when the
                // stat-verification path proves that artifact is still current.
                manager.tier2_read_cached_readonly(snapshot.clone(), *category, scope.clone())
            }
        } else {
            manager.submit_category_with_callgraph(snapshot.clone(), *category, scope.clone(), None)
        };
        outcomes.insert(*category, outcome);
    }

    // Truthful fleet-status values update from whatever this collection proved,
    // even when the freshness gate below refuses the payload: a verified count
    // stays verified, and pending or failed categories remain absent rather
    // than reading as zero.
    refresh_status_bar_counts(ctx, &outcomes);

    let payloads = match fresh_payloads(&outcomes) {
        Ok(payloads) => payloads,
        Err(message) => return Response::error(&req.id, "inspect_not_fresh", message),
    };

    let payload = build_inspect_payload(&snapshot, &payloads, &sections, top_k, ctx);
    Response::success(&req.id, payload)
}

/// Register one inspect completion whose poll closure only observes the result
/// channel. Keep payload construction and checks that require newly scanned data
/// in `handle_inspect_payload` and the scanners that produce those results.
pub fn handle_inspect_deferred(req: &RawRequest, ctx: Arc<AppContext>) -> DispatchOutcome {
    let request_id = req.id.clone();
    let phase_log = InspectPhaseLog::for_request(request_id.clone());
    let snapshot = match inspect_preflight(req, &ctx) {
        Ok(snapshot) => snapshot,
        Err(response) => {
            let detail = response
                .data
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            return deferred_response(
                request_id,
                build_inspect_terminal(
                    &req.id,
                    &phase_log,
                    InspectTerminal::PhaseFailed {
                        failed_phase: None,
                        failure_reason: "root_resolution_failed",
                        failure_detail: detail,
                    },
                ),
            );
        }
    };
    let applicability = {
        let lsp = ctx.lsp();
        lsp.resolve_applicable_servers_for_root(&snapshot.project_root, &snapshot.config)
    };
    let applicability = match applicability {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return deferred_response(
                request_id,
                build_inspect_terminal(
                    &req.id,
                    &phase_log,
                    InspectTerminal::PhaseFailed {
                        failed_phase: None,
                        failure_reason: applicability_failure_reason(&error),
                        failure_detail: Some(applicability_failure_detail(error)),
                    },
                ),
            );
        }
    };

    let request = RawRequest {
        id: req.id.clone(),
        command: req.command.clone(),
        lsp_hints: req.lsp_hints.clone(),
        session_id: req.session_id.clone(),
        params: req.params.clone(),
    };
    let completion_request_id = request_id.clone();
    let shutdown_log = phase_log.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let response = run_blocking_inspect_body(&request, &ctx, applicability, phase_log);
        let _ = tx.send(response);
    });
    DispatchOutcome::Deferred(PendingResponse {
        request_id: completion_request_id,
        session_id: String::new(),
        attach_command: String::new(),
        poll: Box::new(move |_| rx.try_recv().ok()),
        on_shutdown: Some(inspect_shutdown_terminal(request_id, shutdown_log)),
    })
}

fn inspect_preflight(req: &RawRequest, ctx: &AppContext) -> Result<InspectSnapshot, Response> {
    parse_top_k(&req.params).map_err(|message| invalid_request(&req.id, message))?;
    parse_sections(req.params.get("sections"))
        .map_err(|message| invalid_request(&req.id, message))?;
    let snapshot = build_snapshot(ctx).map_err(|response| response.with_id(&req.id))?;
    parse_scope(req, ctx, &snapshot.project_root)?;
    Ok(snapshot)
}

fn deferred_response(request_id: String, response: Response) -> DispatchOutcome {
    let (tx, rx) = mpsc::sync_channel(1);
    let _ = tx.send(response);
    DispatchOutcome::Deferred(PendingResponse {
        request_id,
        session_id: String::new(),
        attach_command: String::new(),
        poll: Box::new(move |_| rx.try_recv().ok()),
        on_shutdown: None,
    })
}

fn inspect_shutdown_terminal(
    request_id: String,
    phase_log: InspectPhaseLog,
) -> crate::response_finalize::PendingResponseShutdown {
    Box::new(move |_| {
        build_inspect_terminal(
            &request_id,
            &phase_log,
            InspectTerminal::PhaseFailed {
                failed_phase: phase_log.in_flight_entry(),
                failure_reason: "daemon_shutdown",
                failure_detail: None,
            },
        )
    })
}

/// Feed fleet-status values from inspect outcomes. Only a verified payload
/// supplies a category count; pending or failed categories remain absent in the
/// truthful values state instead of being replaced with zero.
fn refresh_status_bar_counts(ctx: &AppContext, outcomes: &BTreeMap<InspectCategory, JobOutcome>) {
    // `JobOutcome::payload()` exposes only Fresh data or a stat-verified stale
    // cache, so an unavailable category cannot overwrite a proven value.
    let count_of = |category: InspectCategory| -> Option<usize> {
        outcomes
            .get(&category)
            .and_then(JobOutcome::payload)
            .and_then(|payload| available_count_from_payload(category, payload))
    };
    let any_tier2_stale = [
        InspectCategory::DeadCode,
        InspectCategory::UnusedExports,
        InspectCategory::Duplicates,
    ]
    .iter()
    .any(|category| {
        matches!(
            outcomes.get(category),
            Some(JobOutcome::Stale { .. } | JobOutcome::Pending { .. })
        )
    });
    let todos = outcomes
        .get(&InspectCategory::Todos)
        .and_then(JobOutcome::payload)
        .and_then(|payload| payload.get("count"))
        .and_then(Value::as_u64)
        .map(|count| count as usize);

    ctx.update_status_bar_tier2(
        count_of(InspectCategory::DeadCode),
        count_of(InspectCategory::UnusedExports),
        count_of(InspectCategory::Duplicates),
        todos,
        any_tier2_stale,
    );
}

/// A blocking `aft_inspect` may update alert state only from accepted snapshots
/// whose document versions were verified. Other inspect operations compute
/// payloads or fleet values and must not update alert state.
fn record_blocking_inspect_observations(
    ctx: &AppContext,
    req: &RawRequest,
    snapshot: &InspectSnapshot,
    accepted_snapshots: Vec<AcceptedDiagnosticSnapshot>,
) {
    if accepted_snapshots.is_empty() {
        return;
    }

    let batch = match AcceptedObservationBatch::from_diagnostic_snapshots(
        req.session(),
        &snapshot.project_root,
        accepted_snapshots,
    ) {
        Ok(batch) => batch,
        Err(error) => {
            crate::slog_warn!(
                "[inspect:diagnostics] omitted duplicate producer observation batch: {error}"
            );
            return;
        }
    };
    if let Err(error) = ctx.accept_alert_observation_batch(&batch) {
        crate::slog_warn!("[inspect:diagnostics] failed to accept observation batch: {error}");
    }
}

fn run_blocking_inspect_body(
    req: &RawRequest,
    ctx: &AppContext,
    applicability: ApplicableServerSnapshot,
    phase_log: InspectPhaseLog,
) -> Response {
    let starts = applicability
        .server_keys
        .iter()
        .map(|server| {
            (
                server.clone(),
                phase_log.start(InspectPhaseEntry::lsp(InspectPhaseId::LspStart, server)),
            )
        })
        .collect::<Vec<_>>();
    if let Err(error) = {
        let mut lsp = ctx.lsp();
        lsp.start_applicable_servers(&applicability, &ctx.config())
    } {
        finish_start_failure(starts, &error);
        return build_inspect_terminal(
            &req.id,
            &phase_log,
            InspectTerminal::PhaseFailed {
                failed_phase: Some(InspectPhaseEntry::lsp(
                    InspectPhaseId::LspStart,
                    &error.server_key,
                )),
                failure_reason: "server_start_failed",
                failure_detail: Some(start_failure_detail(&error)),
            },
        );
    }
    for (_, phase) in starts {
        phase.complete();
    }

    let quiescence = applicability
        .server_keys
        .iter()
        .map(|server| {
            phase_log.start(InspectPhaseEntry::lsp(
                InspectPhaseId::LspQuiescence,
                server,
            ))
        })
        .collect::<Vec<_>>();
    // A blocking inspection is an explicit diagnostics observation source. Keep
    // accepted producer snapshots intact until the inspect response is built;
    // flattened category payloads cannot recover producer ownership.
    let accepted_snapshots = ctx.lsp().drain_events().accepted_snapshots;
    let inspect_snapshot = build_snapshot(ctx).ok();
    let response = handle_inspect_payload(req, ctx, true, applicability.server_keys.is_empty());
    if let Some(inspect_snapshot) = &inspect_snapshot {
        record_blocking_inspect_observations(ctx, req, inspect_snapshot, accepted_snapshots);
    }
    for phase in quiescence {
        phase.complete();
    }
    if response.success {
        build_inspect_terminal(&req.id, &phase_log, InspectTerminal::Fresh(response.data))
    } else {
        build_inspect_terminal(
            &req.id,
            &phase_log,
            InspectTerminal::PhaseFailed {
                failed_phase: phase_log.in_flight_entry(),
                failure_reason: "inspect_not_fresh",
                failure_detail: response
                    .data
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
        )
    }
}

fn finish_start_failure(
    starts: Vec<(
        crate::lsp::roots::ServerKey,
        crate::inspect::phase_log::InspectPhaseHandle,
    )>,
    error: &ApplicableServerStartError,
) {
    for (server, phase) in starts {
        if server == error.server_key {
            phase.fail(start_failure_detail(error));
        } else {
            phase.complete();
        }
    }
}

fn start_failure_detail(error: &ApplicableServerStartError) -> String {
    match &error.result {
        crate::lsp::manager::ServerAttemptResult::BinaryNotInstalled { binary } => {
            format!("{binary} is unavailable")
        }
        crate::lsp::manager::ServerAttemptResult::SpawnFailed { reason, .. } => reason.clone(),
        crate::lsp::manager::ServerAttemptResult::NoRootMarker { .. }
        | crate::lsp::manager::ServerAttemptResult::Ok { .. } => {
            "server could not be started".to_string()
        }
    }
}

fn applicability_failure_reason(error: &ApplicabilityResolutionError) -> &'static str {
    match error {
        ApplicabilityResolutionError::MissingExecutable { .. } => "missing_executable",
        ApplicabilityResolutionError::RootUnreadable { .. }
        | ApplicabilityResolutionError::CachedSpawnFailure { .. } => {
            "applicability_resolution_failed"
        }
    }
}

fn applicability_failure_detail(error: ApplicabilityResolutionError) -> String {
    match error {
        ApplicabilityResolutionError::RootUnreadable { root, reason } => {
            format!("cannot resolve {}: {reason}", root.display())
        }
        ApplicabilityResolutionError::MissingExecutable { server_key, binary } => {
            format!("{binary} is required for {}", server_key.kind.id_str())
        }
        ApplicabilityResolutionError::CachedSpawnFailure { server_key, result } => format!(
            "cached failure for {}: {result:?}",
            server_key.kind.id_str()
        ),
    }
}

#[allow(dead_code)]
enum InspectTerminal {
    Fresh(Value),
    Interrupted,
    PhaseFailed {
        failed_phase: Option<InspectPhaseEntry>,
        failure_reason: &'static str,
        failure_detail: Option<String>,
    },
}

fn build_inspect_terminal(
    request_id: &str,
    log: &InspectPhaseLog,
    terminal: InspectTerminal,
) -> Response {
    let (phases, blocking_waited) = log.terminal_inputs();
    match terminal {
        InspectTerminal::Fresh(mut payload) => {
            let Some(payload) = payload.as_object_mut() else {
                return Response::error(
                    request_id,
                    "inspect_terminal_invalid",
                    "inspect payload was not an object",
                );
            };
            payload.insert(
                "inspect_terminal".to_string(),
                Value::String("fresh".to_string()),
            );
            payload.insert(
                "wait_stamp".to_string(),
                serde_json::json!({
                    "text": format_wait_text(&phases, blocking_waited),
                    "phases": phases,
                }),
            );
            Response::success(request_id, Value::Object(payload.clone()))
        }
        InspectTerminal::Interrupted => Response {
            id: request_id.to_string(),
            success: false,
            data: serde_json::json!({"inspect_terminal": "interrupted", "completed_phases": phases}),
        },
        InspectTerminal::PhaseFailed {
            failed_phase,
            failure_reason,
            failure_detail,
        } => {
            let mut data = serde_json::json!({
                "inspect_terminal": "phase_failed",
                "completed_phases": phases,
                "failure_reason": failure_reason,
            });
            if let Some(phase) = failed_phase {
                data["failed_phase"] = serde_json::json!(phase.id);
                if let Some(producer) = phase.producer {
                    data["producer"] = Value::String(producer);
                }
                if let Some(category) = phase.category {
                    data["category"] = Value::String(category);
                }
            }
            if let Some(detail) = failure_detail {
                data["failure_detail"] = Value::String(detail);
            }
            Response {
                id: request_id.to_string(),
                success: false,
                data,
            }
        }
    }
}

pub fn handle_inspect_tier2_run(req: &RawRequest, ctx: &AppContext) -> Response {
    let categories = match parse_tier2_categories(req.params.get("categories")) {
        Ok(categories) => categories,
        Err(message) => return invalid_request(&req.id, message),
    };

    if !ctx.inspect_writer() {
        let skipped = categories
            .iter()
            .map(|category| {
                serde_json::json!({
                    "category": category.as_str(),
                    "reason": "inspect_read_only",
                })
            })
            .collect::<Vec<_>>();
        return Response::success(
            &req.id,
            serde_json::json!({
                "queued_categories": [],
                "in_flight_categories": [],
                "errors": [],
                "skipped_categories": skipped,
            }),
        );
    }

    let snapshot = match build_snapshot(ctx) {
        Ok(snapshot) => snapshot,
        Err(response) => return response.with_id(&req.id),
    };
    let manager = ctx.inspect_manager();
    let submission = manager.submit_tier2_run_with_reuse_serial_background(snapshot, categories);
    if submission.has_new_work() {
        ctx.note_tier2_refresh_started();
    }

    let queued = submission
        .queued_categories
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let errors = submission
        .errors
        .iter()
        .map(|error| {
            serde_json::json!({
                "category": error.category.as_str(),
                "message": error.message.as_str(),
            })
        })
        .collect::<Vec<_>>();

    Response::success(
        &req.id,
        serde_json::json!({
            "queued_categories": queued.clone(),
            "in_flight_categories": queued,
            "errors": errors,
        }),
    )
}

trait ResponseIdExt {
    fn with_id(self, id: &str) -> Self;
}

impl ResponseIdExt for Response {
    fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_string();
        self
    }
}

#[derive(Debug, Clone)]
struct Sections {
    detail_categories: BTreeSet<InspectCategory>,
}

impl Sections {
    fn summary_only() -> Self {
        Self {
            detail_categories: BTreeSet::new(),
        }
    }

    fn all() -> Self {
        Self {
            detail_categories: InspectCategory::active().iter().copied().collect(),
        }
    }

    fn includes(&self, category: InspectCategory) -> bool {
        self.detail_categories.contains(&category)
    }
}

fn build_snapshot(ctx: &AppContext) -> Result<InspectSnapshot, Response> {
    if ctx.harness_opt().is_none() {
        return Err(Response::error(
            "inspect",
            "not_configured",
            "inspect: configure must run before aft_inspect so the harness-scoped cache path is known",
        ));
    }

    let config = ctx.config();
    let project_root = config
        .project_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    // Normalized, not bare-canonical: the diagnostics collection filters
    // LSP-reported (normalized) paths against this root with starts_with,
    // so a Windows verbatim root here silently drops every diagnostic.
    let project_root = crate::inspect::job::canonicalize_normalized(&project_root);
    Ok(InspectSnapshot::new_with_capabilities(
        project_root,
        ctx.inspect_dir(),
        config,
        ctx.symbol_cache(),
        ctx.inspect_writer(),
        ctx.callgraph_writer(),
    ))
}

fn receive_tier2_completion(rx: std::sync::mpsc::Receiver<JobOutcome>) -> JobOutcome {
    rx.recv().unwrap_or_else(|_| JobOutcome::Failed {
        message: "inspect Tier-2 worker disconnected before completion".to_string(),
    })
}

fn fresh_payloads(
    outcomes: &BTreeMap<InspectCategory, JobOutcome>,
) -> Result<BTreeMap<InspectCategory, Value>, String> {
    let mut payloads = BTreeMap::new();
    for category in InspectCategory::active() {
        match outcomes.get(category) {
            Some(JobOutcome::Fresh { payload }) => {
                payloads.insert(*category, payload.clone());
            }
            Some(JobOutcome::Stale { .. }) => {
                return Err(format!("{} could not be stat-verified", category.as_str()));
            }
            Some(JobOutcome::Pending { .. }) => {
                return Err(format!("{} did not complete", category.as_str()));
            }
            Some(JobOutcome::Failed { message }) => {
                return Err(format!("{} failed: {message}", category.as_str()));
            }
            None => return Err(format!("{} did not produce an outcome", category.as_str())),
        }
    }
    Ok(payloads)
}

fn parse_top_k(params: &Value) -> Result<usize, String> {
    let Some(value) = params.get("topK").or_else(|| params.get("top_k")) else {
        return Ok(DEFAULT_TOP_K);
    };
    if value.is_null() || empty_string(value) {
        return Ok(DEFAULT_TOP_K);
    }
    let Some(top_k) = value.as_u64() else {
        return Err("inspect: topK must be a positive integer".to_string());
    };
    if top_k == 0 {
        return Err("inspect: topK must be greater than 0".to_string());
    }
    Ok((top_k as usize).min(MAX_TOP_K))
}

fn parse_sections(value: Option<&Value>) -> Result<Sections, String> {
    let Some(value) = value else {
        return Ok(Sections::summary_only());
    };
    if value.is_null() || empty_string(value) || empty_array(value) {
        return Ok(Sections::summary_only());
    }

    let mut categories = BTreeSet::new();
    match value {
        Value::String(section) => add_section(section, &mut categories)?,
        Value::Array(sections) => {
            for section in sections {
                if section.is_null() || empty_string(section) {
                    continue;
                }
                let Some(section) = section.as_str() else {
                    return Err("inspect: sections array entries must be strings".to_string());
                };
                add_section(section, &mut categories)?;
            }
        }
        _ => return Err("inspect: sections must be a string or string array".to_string()),
    }

    if categories.len() == InspectCategory::active().len() {
        Ok(Sections::all())
    } else {
        Ok(Sections {
            detail_categories: categories,
        })
    }
}

fn scope_was_provided(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    !(value.is_null() || empty_string(value) || empty_array(value))
}

fn add_section(section: &str, categories: &mut BTreeSet<InspectCategory>) -> Result<(), String> {
    let section = section.trim();
    if section.is_empty() {
        return Ok(());
    }
    if section == "all" {
        categories.extend(InspectCategory::active().iter().copied());
        return Ok(());
    }
    let category = section
        .parse::<InspectCategory>()
        .map_err(|error| format!("inspect: {error}"))?;
    if !category.is_active() {
        return Err(format!(
            "inspect: category '{category}' is registered but disabled in v0.33"
        ));
    }
    categories.insert(category);
    Ok(())
}

fn parse_tier2_categories(value: Option<&Value>) -> Result<Vec<InspectCategory>, String> {
    let sections = parse_sections(value)?.detail_categories;
    let categories = if sections.is_empty() {
        InspectCategory::active()
            .iter()
            .copied()
            .filter(|category| category.is_tier2())
            .collect::<Vec<_>>()
    } else {
        sections
            .into_iter()
            .filter(|category| category.is_tier2())
            .collect::<Vec<_>>()
    };
    Ok(categories)
}

fn parse_scope(
    req: &RawRequest,
    ctx: &AppContext,
    project_root: &Path,
) -> Result<JobScope, Response> {
    let Some(value) = req.params.get("scope") else {
        return Ok(JobScope::for_project(project_root.to_path_buf()));
    };
    if value.is_null() || empty_string(value) || empty_array(value) {
        return Ok(JobScope::for_project(project_root.to_path_buf()));
    }

    let raw_scopes = match value {
        Value::String(scope) => vec![scope.clone()],
        Value::Array(scopes) => {
            let mut values = Vec::new();
            for scope in scopes {
                if scope.is_null() || empty_string(scope) {
                    continue;
                }
                let Some(scope) = scope.as_str() else {
                    return Err(Response::error(
                        &req.id,
                        "invalid_request",
                        "inspect: scope array entries must be strings",
                    ));
                };
                values.push(scope.to_string());
            }
            values
        }
        _ => {
            return Err(Response::error(
                &req.id,
                "invalid_request",
                "inspect: scope must be a string or string array",
            ));
        }
    };

    let mut roots = Vec::new();
    for scope in raw_scopes {
        let raw_path = PathBuf::from(scope);
        let candidate = if raw_path.is_absolute() {
            raw_path
        } else {
            project_root.join(raw_path)
        };
        let validated = ctx.validate_path(&req.id, &candidate)?;
        roots.push(std::fs::canonicalize(&validated).unwrap_or(validated));
    }

    Ok(JobScope::from_roots(project_root.to_path_buf(), roots))
}

fn build_inspect_payload(
    snapshot: &InspectSnapshot,
    payloads: &BTreeMap<InspectCategory, Value>,
    sections: &Sections,
    top_k: usize,
    ctx: &AppContext,
) -> Value {
    let mut summary = Map::new();
    let mut details = Map::new();

    for category in InspectCategory::active() {
        // `fresh_payloads` established this invariant before this emitter runs.
        // Keeping the fresh payload separate from JobOutcome prevents accidental
        // reintroduction of a stale or pending branch into a successful response.
        let payload = payloads
            .get(category)
            .expect("all active categories have a fresh inspect payload");
        summary.insert(
            category.as_str().to_string(),
            summary_for(*category, payload),
        );
        if sections.includes(*category) {
            details.insert(
                category.as_str().to_string(),
                details_for(*category, payload, top_k),
            );
            if matches!(
                *category,
                InspectCategory::DeadCode | InspectCategory::UnusedExports
            ) {
                let test_only_detail = test_only_details_for(payload, top_k);
                if test_only_detail
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                {
                    details.insert(format!("{}_test_only", category.as_str()), test_only_detail);
                }
            }
            if matches!(
                *category,
                InspectCategory::DeadCode
                    | InspectCategory::UnusedExports
                    | InspectCategory::Duplicates
            ) {
                let generated_detail = generated_details_for(payload, top_k);
                if generated_detail
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                {
                    details.insert(format!("{}_generated", category.as_str()), generated_detail);
                }
            }
        } else if *category == InspectCategory::Diagnostics {
            // Diagnostics detail is actionable even without an explicit section.
            // `top_k` limits rows only; summaries are always computed in full.
            let detail = details_for(*category, payload, top_k);
            if detail.as_array().is_some_and(|items| !items.is_empty()) {
                details.insert(category.as_str().to_string(), detail);
            }
        }
    }

    let text = render_inspect_text(&summary, &details);
    let mut payload = serde_json::json!({
        "summary": Value::Object(summary),
        "text": text,
        "scanner_state": {
            "tier2_last_run": tier2_last_run(snapshot),
            "tier2_trigger_reason": ctx.tier2_trigger_reason(),
            "disabled_categories": InspectCategory::disabled()
                .iter()
                .map(|category| category.as_str())
                .collect::<Vec<_>>(),
        }
    });
    if !details.is_empty() {
        payload["details"] = Value::Object(details);
    }
    payload
}

/// Render the compact agent-facing body. One source of truth for OpenCode + Pi.
fn render_inspect_text(summary: &Map<String, Value>, details: &Map<String, Value>) -> String {
    let mut lines: Vec<String> = Vec::new();

    // This renderer receives only the verified payload map produced above. It
    // therefore contains findings, never cache-state guidance or partial counts.
    render_group_category(&mut lines, "Duplicates", summary, details, "duplicates");
    render_cycles_category(&mut lines, summary, details);
    render_symbol_category(&mut lines, "Dead code", summary, details, "dead_code");
    render_symbol_category(
        &mut lines,
        "Unused exports",
        summary,
        details,
        "unused_exports",
    );
    render_todos(&mut lines, summary, details);

    lines.join("\n")
}

fn render_cycles_category(
    lines: &mut Vec<String>,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
) {
    if !details.contains_key("cycles") {
        return;
    }
    let Some(section) = summary.get("cycles") else {
        return;
    };
    if let Some(status) = section.get("status").and_then(Value::as_str) {
        lines.push(format!("Import cycles: {status}"));
        return;
    }
    let count = section.get("count").and_then(Value::as_u64).unwrap_or(0);
    if count == 0 {
        lines.push("Import cycles: 0".to_string());
        return;
    }
    let largest = section.get("largest").and_then(Value::as_u64).unwrap_or(0);
    let cycle_word = if count == 1 { "cycle" } else { "cycles" };
    let file_word = if largest == 1 { "file" } else { "files" };
    lines.push(format!(
        "Import cycles: {count} import {cycle_word} (largest: {largest} {file_word})"
    ));
    let Some(items) = details.get("cycles").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let cycle = item.get("cycle").and_then(Value::as_str).unwrap_or("?");
        let edge_kind = item
            .get("edge_kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        lines.push(format!("  {cycle} [{edge_kind}]"));
        if let Some(edges) = item.get("edges").and_then(Value::as_array) {
            for edge in edges {
                let from = edge.get("from").and_then(Value::as_str).unwrap_or("?");
                let to = edge.get("to").and_then(Value::as_str).unwrap_or("?");
                let imports = edge
                    .get("imports")
                    .and_then(Value::as_array)
                    .map(|imports| {
                        imports
                            .iter()
                            .map(render_cycle_import)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                if imports.is_empty() {
                    lines.push(format!("    {from} -> {to}"));
                } else {
                    lines.push(format!("    {from} -> {to} via {imports}"));
                }
            }
        }
    }
}

fn render_cycle_import(import: &Value) -> String {
    let specifier = import
        .get("specifier")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let kind = import
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("import");
    let line = import.get("line").and_then(Value::as_u64).unwrap_or(0);
    if line == 0 {
        format!("{kind} '{specifier}'")
    } else {
        format!("{kind} '{specifier}' line {line}")
    }
}

/// Pick the fuller drill-down list when present (sections requested), else the
/// summary's ranked `top` preview.
fn category_items<'a>(
    summary: &'a Map<String, Value>,
    details: &'a Map<String, Value>,
    key: &str,
) -> Option<&'a Vec<Value>> {
    details
        .get(key)
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .or_else(|| {
            summary
                .get(key)
                .and_then(|s| s.get("top"))
                .and_then(Value::as_array)
        })
}

/// Categories whose findings are `{file, symbol}` (dead_code, unused_exports).
fn render_symbol_category(
    lines: &mut Vec<String>,
    label: &str,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    let Some(section) = summary.get(key) else {
        return;
    };
    if key == "dead_code"
        && section.get("callgraph_available").and_then(Value::as_bool) == Some(false)
    {
        lines.push("Dead code analysis unavailable (no callgraph)".to_string());
        return;
    }
    if let Some(status) = section.get("status").and_then(Value::as_str) {
        if let Some(reason) = section.get("reason").and_then(Value::as_str) {
            lines.push(format!("{label}: {status} ({reason})"));
        } else {
            lines.push(format!("{label}: {status}"));
        }
        return;
    }
    let count = section.get("count").and_then(Value::as_u64).unwrap_or(0);
    let suffix = dead_code_language_suffix(section);
    let skipped_suffix = dead_code_skipped_language_suffix(section);
    let generated_suffix = generated_count_suffix(section);
    if count == 0 {
        lines.push(format!("{label}: 0{generated_suffix}{skipped_suffix}"));
    } else {
        lines.push(format!(
            "{label}: {count}{suffix}{generated_suffix}{skipped_suffix}:"
        ));
        if let Some(items) = category_items(summary, details, key) {
            for item in items.iter().filter(|item| !item_is_generated(item)) {
                let file = item.get("file").and_then(Value::as_str).unwrap_or("?");
                let symbol = item.get("symbol").and_then(Value::as_str).unwrap_or("?");
                lines.push(format!("  {file}::{symbol}"));
            }
        }
    }
    render_generated_symbol_usage(lines, summary, details, key);
    render_test_only_usage(lines, summary, details, key);
}

fn generated_count_suffix(section: &Value) -> String {
    let generated_count = section
        .get("generated_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if generated_count == 0 {
        String::new()
    } else {
        format!(" (generated: {generated_count})")
    }
}

fn item_is_generated(item: &Value) -> bool {
    item.get("generated").and_then(Value::as_bool) == Some(true)
}

fn render_generated_symbol_usage(
    lines: &mut Vec<String>,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    let generated_count = summary
        .get(key)
        .and_then(|section| section.get("generated_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if generated_count == 0 {
        return;
    }
    lines.push(format!("  generated: {generated_count}:"));
    if let Some(items) = generated_items(summary, details, key) {
        for item in items {
            let file = item.get("file").and_then(Value::as_str).unwrap_or("?");
            let symbol = item.get("symbol").and_then(Value::as_str).unwrap_or("?");
            lines.push(format!("    {file}::{symbol}"));
        }
    }
}

fn render_test_only_usage(
    lines: &mut Vec<String>,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    let test_only_count = summary
        .get(key)
        .and_then(|section| section.get("test_only_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if test_only_count == 0 {
        return;
    }
    lines.push(format!("  test-only usage: {test_only_count}:"));
    if let Some(items) = test_only_items(summary, details, key) {
        for item in items {
            let file = item.get("file").and_then(Value::as_str).unwrap_or("?");
            let symbol = item.get("symbol").and_then(Value::as_str).unwrap_or("?");
            let used_by = format_used_by_tests(item.get("used_by"));
            lines.push(format!("    {file}::{symbol} — used by {used_by}"));
        }
    }
}

fn test_only_items<'a>(
    summary: &'a Map<String, Value>,
    details: &'a Map<String, Value>,
    key: &str,
) -> Option<&'a Vec<Value>> {
    details
        .get(&format!("{key}_test_only"))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .or_else(|| {
            summary
                .get(key)
                .and_then(|s| s.get("test_only_top"))
                .and_then(Value::as_array)
        })
}

fn generated_items<'a>(
    summary: &'a Map<String, Value>,
    details: &'a Map<String, Value>,
    key: &str,
) -> Option<&'a Vec<Value>> {
    details
        .get(&format!("{key}_generated"))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .or_else(|| {
            summary
                .get(key)
                .and_then(|s| s.get("generated_top"))
                .and_then(Value::as_array)
        })
}

fn format_used_by_tests(value: Option<&Value>) -> String {
    let names = value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if names.is_empty() {
        "test file".to_string()
    } else {
        names.join(", ")
    }
}

/// `(rust 214, ts 143)` language breakdown for dead_code; empty for others.
fn dead_code_language_suffix(section: &Value) -> String {
    let Some(by_lang) = section.get("by_language").and_then(Value::as_object) else {
        return String::new();
    };
    if by_lang.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(&String, u64)> = by_lang
        .iter()
        .map(|(k, v)| (k, v.as_u64().unwrap_or(0)))
        .collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let rendered = pairs
        .iter()
        .map(|(lang, n)| format!("{} {n}", short_lang(lang)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(" ({rendered})")
}

fn dead_code_skipped_language_suffix(section: &Value) -> String {
    let Some(languages) = section.get("languages_skipped").and_then(Value::as_array) else {
        return String::new();
    };
    if languages.is_empty() {
        return String::new();
    }
    let mut languages = languages
        .iter()
        .filter_map(Value::as_str)
        .map(short_lang)
        .collect::<Vec<_>>();
    languages.sort_unstable();
    languages.dedup();
    if languages.is_empty() {
        String::new()
    } else {
        format!(" ({} not analyzed)", languages.join(", "))
    }
}

fn short_lang(lang: &str) -> &str {
    match lang {
        "typescript" => "ts",
        "javascript" => "js",
        "python" => "py",
        other => other,
    }
}

/// Duplicates: `{cost, files: [a, b, ...]}`.
fn render_group_category(
    lines: &mut Vec<String>,
    label: &str,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    if key == "duplicates" {
        render_duplicates_category(lines, label, summary, details, key);
        return;
    }

    let Some(section) = summary.get(key) else {
        return;
    };
    if let Some(status) = section.get("status").and_then(Value::as_str) {
        lines.push(format!("{label}: {status}"));
        return;
    }
    let count = section.get("count").and_then(Value::as_u64).unwrap_or(0);
    if count == 0 {
        lines.push(format!("{label}: 0"));
        return;
    }
    lines.push(format!("{label}: {count} (top by cost):"));
    if let Some(items) = category_items(summary, details, key) {
        for item in items.iter().filter(|item| !item_is_generated(item)) {
            let cost = item.get("cost").and_then(Value::as_u64).unwrap_or(0);
            let files: Vec<&str> = item
                .get("files")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            lines.push(format!("  {cost}  {}", files.join(" == ")));
        }
    }
}

fn render_duplicates_category(
    lines: &mut Vec<String>,
    label: &str,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    let Some(section) = summary.get(key) else {
        return;
    };
    if let Some(status) = section.get("status").and_then(Value::as_str) {
        lines.push(format!("{label}: {status}"));
        return;
    }

    let count = section.get("count").and_then(Value::as_u64).unwrap_or(0);
    let generated_count = section
        .get("generated_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let generated_suffix = if generated_count == 0 {
        String::new()
    } else {
        format!(" (generated: {generated_count})")
    };
    let Some(duplicated_lines) = section.get("duplicated_lines").and_then(Value::as_u64) else {
        if count == 0 {
            lines.push(format!("{label}: 0{generated_suffix}"));
            render_generated_duplicate_usage(lines, summary, details, key);
            return;
        }
        lines.push(format!("{label}: {count}{generated_suffix} (top by cost):"));
        render_duplicate_rows(lines, summary, details, key);
        render_generated_duplicate_usage(lines, summary, details, key);
        return;
    };

    let total_lines = section
        .get("total_analyzed_lines")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let percent = section
        .get("duplicated_percent")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| duplicate_percent(duplicated_lines, total_lines));
    let file_count = section
        .get("duplicated_file_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let group_count = count;
    let suffix = if count > 0 { " (top by cost):" } else { "" };
    // A zero denominator means analyzed-line counts are missing (pre-v0.44
    // cached contributions); print no percentage rather than a false "0.0%".
    let percent_clause = if total_lines > 0 {
        format!(
            " ({}% of {total_lines} analyzed lines)",
            format_percent(percent)
        )
    } else {
        String::new()
    };
    lines.push(format!(
        "{label}: {duplicated_lines} duplicated lines{percent_clause} across {file_count} files, {group_count} {}{generated_suffix}{suffix}",
        plural_group(group_count),
    ));
    render_duplicate_suppression(lines, section);
    if count > 0 {
        render_duplicate_rows(lines, summary, details, key);
    }
    render_generated_duplicate_usage(lines, summary, details, key);
}

fn render_duplicate_rows(
    lines: &mut Vec<String>,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    if let Some(items) = category_items(summary, details, key) {
        for item in items.iter().filter(|item| !item_is_generated(item)) {
            let cost = item.get("cost").and_then(Value::as_u64).unwrap_or(0);
            let files: Vec<&str> = item
                .get("files")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            lines.push(format!("  {cost}  {}", files.join(" == ")));
            if duplicate_group_file_count(&files) >= 3 {
                lines
                    .push("      suggestion: consider extracting into a shared module".to_string());
            }
        }
    }
}

fn render_generated_duplicate_usage(
    lines: &mut Vec<String>,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
    key: &str,
) {
    let generated_count = summary
        .get(key)
        .and_then(|section| section.get("generated_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if generated_count == 0 {
        return;
    }
    lines.push(format!("  generated: {generated_count}:"));
    if let Some(items) = generated_items(summary, details, key) {
        for item in items {
            let cost = item.get("cost").and_then(Value::as_u64).unwrap_or(0);
            let files: Vec<&str> = item
                .get("files")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            lines.push(format!("    {cost}  {}", files.join(" == ")));
        }
    }
}

fn render_duplicate_suppression(lines: &mut Vec<String>, section: &Value) {
    let mirror = section
        .get("mirror_suppressed_groups")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if mirror > 0 {
        lines.push(format!(
            "  {mirror} mirror {} suppressed by expected_mirrors",
            plural_group(mirror)
        ));
    }
    let marker = section
        .get("marker_suppressed_groups")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if marker > 0 {
        lines.push(format!(
            "  {marker} marker {} suppressed by aft:expected-duplicate",
            plural_group(marker)
        ));
    }
}

fn plural_group(count: u64) -> &'static str {
    if count == 1 {
        "group"
    } else {
        "groups"
    }
}

fn duplicate_group_file_count(files: &[&str]) -> usize {
    files
        .iter()
        .map(|file| display_file_from_duplicate_occurrence(file))
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

fn display_file_from_duplicate_occurrence(value: &str) -> &str {
    let Some((file, range)) = value.rsplit_once(':') else {
        return value;
    };
    let Some((start, end)) = range.split_once('-') else {
        return value;
    };
    if start.chars().all(|char| char.is_ascii_digit())
        && end.chars().all(|char| char.is_ascii_digit())
    {
        file
    } else {
        value
    }
}

fn duplicate_percent(duplicated_lines: u64, total_lines: u64) -> f64 {
    if total_lines == 0 {
        0.0
    } else {
        (duplicated_lines as f64 * 100.0) / total_lines as f64
    }
}

fn format_percent(percent: f64) -> String {
    format!("{percent:.1}")
}

fn render_todos(
    lines: &mut Vec<String>,
    summary: &Map<String, Value>,
    details: &Map<String, Value>,
) {
    let Some(section) = summary.get("todos") else {
        return;
    };
    let count = section.get("count").and_then(Value::as_u64).unwrap_or(0);
    if count == 0 {
        return;
    }
    let by_kind = section
        .get("by_kind")
        .and_then(Value::as_object)
        .map(|map| {
            let mut pairs: Vec<(&String, u64)> = map
                .iter()
                .map(|(k, v)| (k, v.as_u64().unwrap_or(0)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            pairs
                .iter()
                .map(|(kind, n)| format!("{kind} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if by_kind.is_empty() {
        lines.push(format!("TODOs: {count}"));
    } else {
        lines.push(format!("TODOs: {count} ({by_kind})"));
    }
    // Detail rows only when explicitly drilled into (sections: ["todos"]) — the
    // scanner populates details["todos"] only then, keeping the default summary
    // compact while honoring an explicit request for the items.
    if let Some(items) = details.get("todos").and_then(Value::as_array) {
        for item in items {
            let file = item.get("file").and_then(Value::as_str).unwrap_or("?");
            let line = item.get("line").and_then(Value::as_u64).unwrap_or(0);
            let marker = item.get("marker").and_then(Value::as_str).unwrap_or("?");
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            lines.push(format!("  {file}:{line} {marker} {text}"));
        }
    }
}

fn summary_for(category: InspectCategory, payload: &Value) -> Value {
    computed_summary_for(category, payload)
}

fn computed_summary_for(category: InspectCategory, payload: &Value) -> Value {
    match category {
        InspectCategory::Diagnostics => diagnostics_summary_for(payload),
        InspectCategory::Metrics => serde_json::json!({
            "files": payload.get("files").or_else(|| payload.pointer("/totals/file_count")).and_then(Value::as_u64).unwrap_or(0),
            "symbols": payload.get("symbols").or_else(|| payload.pointer("/totals/symbol_count")).and_then(Value::as_u64).unwrap_or(0),
            "loc": payload.get("loc").or_else(|| payload.pointer("/totals/loc")).and_then(Value::as_u64).unwrap_or(0),
        }),
        InspectCategory::Todos => serde_json::json!({
            "count": count_from_payload(Some(payload)),
            "by_kind": payload.get("by_kind").or_else(|| payload.get("by_marker")).cloned().unwrap_or_else(|| serde_json::json!({})),
        }),
        InspectCategory::DeadCode
            if payload.get("callgraph_available").and_then(Value::as_bool) == Some(false) =>
        {
            // This is a terminal capability result, not a partial scan: dead-code
            // analysis cannot run without the callgraph, so it must not claim zero.
            serde_json::json!({ "callgraph_available": false })
        }
        InspectCategory::DeadCode => serde_json::json!({
            "count": count_from_payload(Some(payload)),
            "generated_count": generated_count_from_payload(Some(payload)),
            "total_count": total_count_from_payload(Some(payload)),
            "test_only_count": test_only_count_from_payload(Some(payload)),
            "by_language": payload.get("by_language").cloned().unwrap_or_else(|| serde_json::json!({})),
            "languages_skipped": payload.get("languages_skipped").cloned().unwrap_or_else(|| serde_json::json!([])),
            "top": top_preview_from_payload(Some(payload)),
            "generated_top": generated_top_from_payload(Some(payload)),
            "test_only_top": test_only_top_from_payload(Some(payload)),
        }),
        InspectCategory::UnusedExports => serde_json::json!({
            "count": count_from_payload(Some(payload)),
            "generated_count": generated_count_from_payload(Some(payload)),
            "total_count": total_count_from_payload(Some(payload)),
            "test_only_count": test_only_count_from_payload(Some(payload)),
            "top": top_preview_from_payload(Some(payload)),
            "generated_top": generated_top_from_payload(Some(payload)),
            "test_only_top": test_only_top_from_payload(Some(payload)),
        }),
        InspectCategory::Duplicates => {
            let mut section = Map::new();
            section.insert(
                "count".to_string(),
                serde_json::json!(count_from_payload(Some(payload))),
            );
            section.insert(
                "total_groups".to_string(),
                serde_json::json!(payload
                    .get("total_groups")
                    .or_else(|| payload.get("groups_count"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| count_from_payload(Some(payload)))),
            );
            for key in [
                "generated_count",
                "total_count",
                "duplicated_lines",
                "duplicated_percent",
                "duplicated_file_count",
                "generated_duplicated_lines",
                "generated_duplicated_file_count",
                "total_duplicated_lines",
                "total_duplicated_file_count",
                "total_analyzed_lines",
                "suppressed_groups",
                "mirror_suppressed_groups",
                "marker_suppressed_groups",
            ] {
                if let Some(value) = payload.get(key).cloned() {
                    section.insert(key.to_string(), value);
                }
            }
            section.insert("top".to_string(), top_preview_from_payload(Some(payload)));
            section.insert(
                "generated_top".to_string(),
                generated_top_from_payload(Some(payload)),
            );
            Value::Object(section)
        }
        InspectCategory::Cycles => serde_json::json!({
            "count": count_from_payload(Some(payload)),
            "largest": payload.get("largest").and_then(Value::as_u64).unwrap_or(0),
        }),
        _ => serde_json::json!({ "count": count_from_payload(Some(payload)) }),
    }
}

fn diagnostics_summary_for(payload: &Value) -> Value {
    serde_json::json!({
        "errors": payload.get("errors").and_then(Value::as_u64).unwrap_or(0),
        "warnings": payload.get("warnings").and_then(Value::as_u64).unwrap_or(0),
        "info": payload.get("info").and_then(Value::as_u64).unwrap_or(0),
        "hints": payload.get("hints").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn details_for(category: InspectCategory, payload: &Value, top_k: usize) -> Value {
    if category == InspectCategory::Metrics {
        return computed_summary_for(category, payload);
    }
    let items = payload
        .get("items")
        .or_else(|| payload.get("groups"))
        .and_then(Value::as_array);
    match items {
        Some(items) => Value::Array(items.iter().take(top_k).cloned().collect()),
        None => serde_json::json!([]),
    }
}

fn test_only_details_for(payload: &Value, top_k: usize) -> Value {
    match payload.get("test_only_items").and_then(Value::as_array) {
        Some(items) => Value::Array(items.iter().take(top_k).cloned().collect()),
        None => serde_json::json!([]),
    }
}

fn generated_details_for(payload: &Value, top_k: usize) -> Value {
    match payload.get("generated_items").and_then(Value::as_array) {
        Some(items) => Value::Array(items.iter().take(top_k).cloned().collect()),
        None => serde_json::json!([]),
    }
}

fn available_count_from_payload(category: InspectCategory, payload: &Value) -> Option<usize> {
    if category == InspectCategory::DeadCode
        && payload.get("callgraph_available").and_then(Value::as_bool) == Some(false)
    {
        return None;
    }
    payload
        .get("count")
        .and_then(Value::as_u64)
        .map(|count| count as usize)
}

fn count_from_payload(payload: Option<&Value>) -> u64 {
    payload
        .and_then(|payload| payload.get("count"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// Pass through the scanner's already-ranked `top` preview (highest-signal
/// findings) into the summary view. Omitted (empty array) when absent so the
/// summary stays compact for empty/legacy payloads.
fn top_preview_from_payload(payload: Option<&Value>) -> Value {
    payload
        .and_then(|payload| payload.get("top"))
        .filter(|top| top.is_array())
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]))
}

fn test_only_count_from_payload(payload: Option<&Value>) -> u64 {
    payload
        .and_then(|payload| payload.get("test_only_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn generated_count_from_payload(payload: Option<&Value>) -> u64 {
    payload
        .and_then(|payload| payload.get("generated_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn total_count_from_payload(payload: Option<&Value>) -> u64 {
    payload
        .and_then(|payload| payload.get("total_count"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            count_from_payload(payload)
                + test_only_count_from_payload(payload)
                + generated_count_from_payload(payload)
        })
}

fn test_only_top_from_payload(payload: Option<&Value>) -> Value {
    payload
        .and_then(|payload| payload.get("test_only_top"))
        .filter(|top| top.is_array())
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]))
}

fn generated_top_from_payload(payload: Option<&Value>) -> Value {
    payload
        .and_then(|payload| payload.get("generated_top"))
        .filter(|top| top.is_array())
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]))
}

fn tier2_last_run(snapshot: &InspectSnapshot) -> Option<i64> {
    let cache =
        InspectCache::open_readonly(snapshot.inspect_dir.clone(), snapshot.project_root.clone())
            .ok()
            .flatten()?;
    InspectCategory::active()
        .iter()
        .copied()
        .filter(|category| category.is_tier2())
        .filter_map(|category| cache.last_full_run(category).ok().flatten())
        .max()
}

fn empty_string(value: &Value) -> bool {
    value.as_str().is_some_and(|value| value.trim().is_empty())
}

fn empty_array(value: &Value) -> bool {
    value.as_array().is_some_and(|value| value.is_empty())
}

fn invalid_request(id: &str, message: String) -> Response {
    Response::error(id, "invalid_request", message)
}

#[cfg(test)]
mod status_bar_refresh_tests {
    use super::*;
    use crate::parser::TreeSitterProvider;

    fn ctx() -> AppContext {
        AppContext::new(Box::new(TreeSitterProvider::new()), Default::default())
    }

    fn outcomes(
        entries: Vec<(InspectCategory, JobOutcome)>,
    ) -> BTreeMap<InspectCategory, JobOutcome> {
        entries.into_iter().collect()
    }

    // #1: a Pending-only Tier-2 (no scan has ever produced counts) must NOT
    // populate the status bar — otherwise it renders fabricated `~D0 U0 C0`
    // zeros that lie about project health.
    #[test]
    fn pending_tier2_does_not_populate_status_bar() {
        let ctx = ctx();
        assert!(ctx.status_bar_counts().is_none());

        refresh_status_bar_counts(
            &ctx,
            &outcomes(vec![
                (
                    InspectCategory::DeadCode,
                    JobOutcome::Pending { in_flight: true },
                ),
                (
                    InspectCategory::UnusedExports,
                    JobOutcome::Pending { in_flight: true },
                ),
                (
                    InspectCategory::Duplicates,
                    JobOutcome::Pending { in_flight: true },
                ),
            ]),
        );

        assert!(
            ctx.status_bar_counts().is_none(),
            "Pending Tier-2 must leave the bar unpopulated (no fabricated zeros)"
        );
    }

    // Stale-without-cache is equally untrustworthy — also must not populate.
    #[test]
    fn stale_without_cache_does_not_populate_status_bar() {
        let ctx = ctx();
        refresh_status_bar_counts(
            &ctx,
            &outcomes(vec![(
                InspectCategory::DeadCode,
                JobOutcome::Stale {
                    cached: None,
                    in_flight: true,
                },
            )]),
        );
        assert!(ctx.status_bar_counts().is_none());
    }

    // A real Fresh outcome populates the bar with the actual counts.
    #[test]
    fn fresh_tier2_populates_status_bar() {
        let ctx = ctx();
        refresh_status_bar_counts(
            &ctx,
            &outcomes(vec![
                (
                    InspectCategory::DeadCode,
                    JobOutcome::Fresh {
                        payload: serde_json::json!({ "count": 7 }),
                    },
                ),
                (
                    InspectCategory::UnusedExports,
                    JobOutcome::Fresh {
                        payload: serde_json::json!({ "count": 3 }),
                    },
                ),
                (
                    InspectCategory::Duplicates,
                    JobOutcome::Fresh {
                        payload: serde_json::json!({ "count": 1 }),
                    },
                ),
            ]),
        );
        let counts = ctx.status_bar_counts().expect("populated");
        assert_eq!(counts.dead_code, 7);
        assert_eq!(counts.unused_exports, 3);
        assert_eq!(counts.duplicates, 1);
        assert!(!counts.tier2_stale);
    }

    // Stale-WITH-cache populates (last-known counts) and marks the bar stale.
    // All three categories must carry a cached value — the bar stays suppressed
    // until every Tier-2 category is real, never fabricating a 0 (#1).
    #[test]
    fn stale_with_cache_populates_and_marks_stale() {
        let ctx = ctx();
        let stale_cache = |count: i64| JobOutcome::Stale {
            cached: Some(serde_json::json!({ "count": count })),
            in_flight: true,
        };
        refresh_status_bar_counts(
            &ctx,
            &outcomes(vec![
                (InspectCategory::DeadCode, stale_cache(12)),
                (InspectCategory::UnusedExports, stale_cache(4)),
                (InspectCategory::Duplicates, stale_cache(2)),
            ]),
        );
        let counts = ctx.status_bar_counts().expect("populated");
        assert_eq!(counts.dead_code, 12);
        assert_eq!(counts.unused_exports, 4);
        assert_eq!(counts.duplicates, 2);
        assert!(counts.tier2_stale);
    }

    // A single category (others Pending) must NOT surface the bar — the core
    // partial-completion fabrication guard at the sync refresh path (#1).
    #[test]
    fn single_category_does_not_populate_status_bar() {
        let ctx = ctx();
        refresh_status_bar_counts(
            &ctx,
            &outcomes(vec![(
                InspectCategory::DeadCode,
                JobOutcome::Fresh {
                    payload: serde_json::json!({ "count": 9 }),
                },
            )]),
        );
        assert!(
            ctx.status_bar_counts().is_none(),
            "one real category must not surface a bar with fabricated U0 C0"
        );
    }
}

#[cfg(test)]
mod render_text_tests {
    use super::*;

    fn summary_map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    fn render(summary: Value) -> String {
        render_inspect_text(&summary_map(summary), &Map::new())
    }

    fn render_with_details(summary: Value, details: Value) -> String {
        render_inspect_text(&summary_map(summary), &summary_map(details))
    }

    #[test]
    fn renders_unavailable_dead_code_without_a_zero_count() {
        let text = render(serde_json::json!({
            "dead_code": { "callgraph_available": false }
        }));

        assert_eq!(text, "Dead code analysis unavailable (no callgraph)");
        assert!(!text.contains("Dead code: 0"));
    }

    #[test]
    fn renders_todo_detail_rows_when_drilled_into() {
        let text = render_with_details(
            serde_json::json!({ "todos": { "count": 2, "by_kind": { "BUG": 1, "TODO": 1 } } }),
            serde_json::json!({
                "todos": [
                    { "file": "src/a.ts", "line": 10, "marker": "BUG", "text": "leak here" },
                    { "file": "src/b.ts", "line": 4, "marker": "TODO", "text": "wire it" },
                ]
            }),
        );
        // Summary line still present, plus per-item rows.
        assert!(
            text.contains("TODOs: 2 (BUG 1, TODO 1)"),
            "summary:\n{text}"
        );
        assert!(
            text.contains("  src/a.ts:10 BUG leak here"),
            "row a:\n{text}"
        );
        assert!(text.contains("  src/b.ts:4 TODO wire it"), "row b:\n{text}");
    }

    #[test]
    fn omits_todo_detail_rows_without_drill_in() {
        // No details → count/by_kind only, no per-item rows (default compact).
        let text = render(serde_json::json!({
            "todos": { "count": 2, "by_kind": { "BUG": 1, "TODO": 1 } }
        }));
        assert!(
            text.contains("TODOs: 2 (BUG 1, TODO 1)"),
            "summary:\n{text}"
        );
        assert!(!text.contains("\n  "), "no detail rows expected:\n{text}");
    }

    #[test]
    fn renders_populated_categories_highest_signal_first() {
        let text = render(serde_json::json!({
            "duplicates": {
                "count": 2,
                "top": [
                    { "cost": 1083, "files": ["a/x.ts:1-9", "b/x.ts:1-9"] },
                    { "cost": 500, "files": ["a/y.ts:1-3", "b/y.ts:1-3"] },
                ],
            },
            "dead_code": {
                "count": 357,
                "by_language": { "rust": 214, "typescript": 143 },
                "top": [ { "file": "crates/aft/src/x.rs", "symbol": "foo" } ],
            },
            "unused_exports": {
                "count": 1,
                "top": [ { "file": "packages/aft-bridge/src/log.ts", "symbol": "sessionLog" } ],
            },
            "todos": { "count": 8, "by_kind": { "BUG": 2, "TODO": 3 } },
        }));

        // Order: duplicates → dead_code → unused_exports → todos.
        let dup = text.find("Duplicates:").expect("duplicates");
        let dead = text.find("Dead code:").expect("dead code");
        let unused = text.find("Unused exports:").expect("unused");
        let todos = text.find("TODOs:").expect("todos");
        assert!(
            dup < dead && dead < unused && unused < todos,
            "wrong order:\n{text}"
        );

        // Cost-ranked duplicate rows with `==` separator between the file pair.
        assert!(
            text.contains("1083  a/x.ts:1-9 == b/x.ts:1-9"),
            "dup row:\n{text}"
        );
        // dead_code language breakdown uses short names, count-desc.
        assert!(
            text.contains("Dead code: 357 (rust 214, ts 143):"),
            "dead head:\n{text}"
        );
        assert!(
            text.contains("  crates/aft/src/x.rs::foo"),
            "dead row:\n{text}"
        );
        assert!(
            text.contains("  packages/aft-bridge/src/log.ts::sessionLog"),
            "unused row:\n{text}"
        );
        assert!(text.contains("TODOs: 8 (BUG 2, TODO 3)"), "todos:\n{text}");

        // Metrics + scanner_state are NOT in the agent text.
        assert!(!text.contains("loc"), "metrics leaked into text:\n{text}");
        assert!(
            !text.contains("scanner_state"),
            "scanner_state leaked:\n{text}"
        );
        // Diagnostics + status bar are appended by the plugin layer, not here.
        assert!(
            !text.contains("diagnostics"),
            "diagnostics must be plugin-rendered:\n{text}"
        );
        assert!(
            !text.contains("[AFT"),
            "status bar must be plugin-appended:\n{text}"
        );
    }

    #[test]
    fn renders_test_only_usage_after_headline_items() {
        let text = render_with_details(
            serde_json::json!({
                "dead_code": {
                    "count": 1,
                    "top": [ { "file": "src/api.ts", "symbol": "plantedDead" } ],
                    "test_only_count": 2,
                    "test_only_top": [
                        { "file": "src/api.ts", "symbol": "testOnly", "used_by": ["api.test.ts"] },
                    ],
                },
                "unused_exports": {
                    "count": 0,
                    "top": [],
                    "test_only_count": 1,
                    "test_only_top": [
                        { "file": "src/barrel-target.ts", "symbol": "throughBarrel", "used_by": ["barrel.test.ts"] },
                    ],
                }
            }),
            serde_json::json!({
                "dead_code": [ { "file": "src/api.ts", "symbol": "plantedDead" } ],
                "dead_code_test_only": [
                    { "file": "src/api.ts", "symbol": "testOnly", "used_by": ["api.test.ts"] },
                    { "file": "src/barrel-target.ts", "symbol": "throughBarrel", "used_by": ["barrel.test.ts"] },
                ],
            }),
        );

        assert!(text.contains("Dead code: 1:"), "{text}");
        assert!(text.contains("  src/api.ts::plantedDead"), "{text}");
        assert!(text.contains("  test-only usage: 2:"), "{text}");
        assert!(
            text.contains("    src/api.ts::testOnly — used by api.test.ts"),
            "{text}"
        );
        assert!(
            text.contains("    src/barrel-target.ts::throughBarrel — used by barrel.test.ts"),
            "{text}"
        );
        assert!(text.contains("Unused exports: 0"), "{text}");
        assert!(
            text.contains("    src/barrel-target.ts::throughBarrel — used by barrel.test.ts"),
            "{text}"
        );
    }

    #[test]
    fn renders_dead_code_skipped_languages_as_not_analyzed() {
        let text = render(serde_json::json!({
            "dead_code": {
                "count": 0,
                "by_language": {},
                "languages_skipped": ["kotlin", "java"],
                "top": [],
            }
        }));

        assert!(
            text.contains("Dead code: 0 (java, kotlin not analyzed)"),
            "dead-code skipped language note missing:\n{text}"
        );
    }

    #[test]
    fn renders_generated_usage_after_headline_items() {
        let text = render_with_details(
            serde_json::json!({
                "duplicates": {
                    "count": 1,
                    "generated_count": 1,
                    "total_groups": 2,
                    "duplicated_lines": 6,
                    "duplicated_percent": 3.0,
                    "duplicated_file_count": 2,
                    "total_analyzed_lines": 200,
                    "top": [
                        { "cost": 10, "files": ["src/a.ts:1-3", "src/b.ts:1-3"] },
                    ],
                    "generated_top": [
                        { "cost": 100, "files": ["gen/a.ts:1-9", "gen/b.ts:1-9"], "generated": true },
                    ],
                },
                "dead_code": {
                    "count": 1,
                    "generated_count": 2,
                    "total_count": 3,
                    "top": [ { "file": "src/hand.ts", "symbol": "handDead" } ],
                    "generated_top": [
                        { "file": "gen/schema_pb.ts", "symbol": "generatedPathDead", "generated": true },
                    ],
                },
                "unused_exports": {
                    "count": 0,
                    "generated_count": 1,
                    "total_count": 1,
                    "top": [],
                    "generated_top": [
                        { "file": "src/banner.ts", "symbol": "bannerUnused", "generated": true },
                    ],
                }
            }),
            serde_json::json!({
                "duplicates": [
                    { "cost": 10, "files": ["src/a.ts:1-3", "src/b.ts:1-3"] },
                    { "cost": 100, "files": ["gen/a.ts:1-9", "gen/b.ts:1-9"], "generated": true },
                ],
                "duplicates_generated": [
                    { "cost": 100, "files": ["gen/a.ts:1-9", "gen/b.ts:1-9"], "generated": true },
                ],
                "dead_code": [
                    { "file": "src/hand.ts", "symbol": "handDead" },
                    { "file": "gen/schema_pb.ts", "symbol": "generatedPathDead", "generated": true },
                ],
                "dead_code_generated": [
                    { "file": "gen/schema_pb.ts", "symbol": "generatedPathDead", "generated": true },
                    { "file": "src/banner.ts", "symbol": "bannerDead", "generated": true },
                ],
            }),
        );

        assert!(
            text.contains("Duplicates: 6 duplicated lines (3.0% of 200 analyzed lines) across 2 files, 1 group (generated: 1) (top by cost):"),
            "{text}"
        );
        assert!(
            text.contains("  10  src/a.ts:1-3 == src/b.ts:1-3"),
            "{text}"
        );
        assert!(text.contains("  generated: 1:"), "{text}");
        assert!(
            text.contains("    100  gen/a.ts:1-9 == gen/b.ts:1-9"),
            "{text}"
        );

        assert!(text.contains("Dead code: 1 (generated: 2):"), "{text}");
        assert!(text.contains("  src/hand.ts::handDead"), "{text}");
        assert!(
            text.contains("    gen/schema_pb.ts::generatedPathDead"),
            "{text}"
        );
        assert!(text.contains("    src/banner.ts::bannerDead"), "{text}");

        assert!(text.contains("Unused exports: 0 (generated: 1)"), "{text}");
        assert!(text.contains("    src/banner.ts::bannerUnused"), "{text}");
    }

    #[test]
    fn renders_duplicate_framing_suppression_and_extraction_suggestions() {
        let text = render(serde_json::json!({
            "duplicates": {
                "count": 1,
                "total_groups": 1,
                "duplicated_lines": 42,
                "duplicated_percent": 10.4,
                "duplicated_file_count": 3,
                "total_analyzed_lines": 404,
                "mirror_suppressed_groups": 2,
                "marker_suppressed_groups": 1,
                "top": [
                    { "cost": 1083, "files": ["a/x.ts:1-9", "b/x.ts:1-9", "c/x.ts:1-9"] }
                ]
            }
        }));

        assert!(
            text.contains(
                "Duplicates: 42 duplicated lines (10.4% of 404 analyzed lines) across 3 files, 1 group (top by cost):"
            ),
            "{text}"
        );
        assert!(
            text.contains("2 mirror groups suppressed by expected_mirrors"),
            "{text}"
        );
        assert!(
            text.contains("1 marker group suppressed by aft:expected-duplicate"),
            "{text}"
        );
        assert!(
            text.contains("suggestion: consider extracting into a shared module"),
            "{text}"
        );
    }

    #[test]
    fn zero_counts_render_as_clean_zero() {
        let text = render(serde_json::json!({
            "duplicates": { "count": 0 },
            "dead_code": { "count": 0, "by_language": {} },
            "unused_exports": { "count": 0 },
            "todos": { "count": 0 },
        }));
        assert!(text.contains("Duplicates: 0"), "{text}");
        assert!(text.contains("Dead code: 0"), "{text}");
        assert!(text.contains("Unused exports: 0"), "{text}");
        // Zero todos are omitted entirely (no noise).
        assert!(
            !text.contains("TODOs:"),
            "zero todos should be omitted:\n{text}"
        );
    }

    #[test]
    fn fresh_text_never_renders_status_sentinels() {
        let text = render(serde_json::json!({
            "duplicates": { "count": 1, "top": [] },
            "dead_code": { "count": 1, "top": [] },
        }));
        assert!(!text.contains("pending"), "{text}");
        assert!(!text.contains("stale"), "{text}");
    }

    #[test]
    fn fresh_text_has_no_cache_state_note() {
        let text = render_inspect_text(&Map::new(), &Map::new());
        assert!(
            !text.contains("note:"),
            "fresh text must not describe partial state: {text}"
        );
    }

    // Fresh summaries are derived only from verified category payloads.
    #[test]
    fn fresh_summary_has_no_stale_flag() {
        let payload = serde_json::json!({ "count": 357, "by_language": { "rust": 214 } });
        let summary = summary_for(InspectCategory::DeadCode, &payload);
        assert_eq!(summary.get("count").and_then(Value::as_u64), Some(357));
        assert!(summary.get("stale").is_none(), "{summary}");
        assert!(summary.get("status").is_none(), "{summary}");
    }

    // Diagnostics summaries retain only verified severity totals.
    #[test]
    fn diagnostics_summary_has_only_verified_counts() {
        let summary = diagnostics_summary_for(&serde_json::json!({
            "errors": 1,
            "warnings": 2,
            "info": 3,
            "hints": 4,
        }));
        assert_eq!(
            summary,
            serde_json::json!({
                "errors": 1,
                "warnings": 2,
                "info": 3,
                "hints": 4,
            })
        );
    }
}

#[cfg(test)]
mod fresh_payload_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::config::Config;
    use crate::parser::SymbolCache;

    fn snapshot() -> InspectSnapshot {
        InspectSnapshot::new(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.aft"),
            Arc::new(Config::default()),
            Arc::new(RwLock::new(SymbolCache::new())),
        )
    }

    fn fresh_payloads_for_all_categories() -> BTreeMap<InspectCategory, Value> {
        InspectCategory::active()
            .iter()
            .copied()
            .map(|category| {
                let payload = match category {
                    InspectCategory::Diagnostics => serde_json::json!({
                        "errors": 2,
                        "warnings": 0,
                        "info": 0,
                        "hints": 0,
                        "items": [
                            { "file": "src/a.rs", "line": 1, "severity": "error" },
                            { "file": "src/b.rs", "line": 2, "severity": "error" },
                        ],
                    }),
                    InspectCategory::Metrics => serde_json::json!({
                        "files": 2,
                        "symbols": 3,
                        "loc": 10,
                    }),
                    InspectCategory::Todos => serde_json::json!({ "count": 1, "by_kind": {} }),
                    InspectCategory::DeadCode | InspectCategory::UnusedExports => {
                        serde_json::json!({ "count": 1, "items": [] })
                    }
                    InspectCategory::Duplicates => serde_json::json!({ "count": 1, "groups": [] }),
                    InspectCategory::Cycles => serde_json::json!({ "count": 0, "largest": 0 }),
                    _ => unreachable!("only active categories are emitted"),
                };
                (category, payload)
            })
            .collect()
    }

    fn assert_no_banned_field(value: &Value) {
        // `callgraph_available` is capability disclosure, not partiality, so fresh
        // payloads may report terminal callgraph unavailability.
        const BANNED_KEYS: &[&str] = &[
            "provisional",
            "provisional_counts",
            "pending_categories",
            "stale_categories",
            "incomplete_categories",
            "scope_truncated",
            "servers_pending",
            "servers_not_installed",
            "files_without_server",
            "failed_categories",
            "complete",
        ];

        match value {
            Value::Array(values) => {
                for value in values {
                    assert_no_banned_field(value);
                }
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    assert!(
                        !BANNED_KEYS.contains(&key.as_str()),
                        "banned inspect field {key} leaked into {value}"
                    );
                    assert!(key != "stale", "stale sentinel leaked into {value}");
                    if key == "server_ran" {
                        assert_ne!(
                            value.as_bool(),
                            Some(false),
                            "unrun server leaked into payload"
                        );
                    }
                    if key == "status" {
                        assert!(
                            !matches!(value.as_str(), Some("pending" | "stale" | "failed")),
                            "partial category status leaked into payload: {value}"
                        );
                    }
                    assert_no_banned_field(value);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn fresh_payload_is_recursive_banned_field_free_and_top_k_only_caps_rows() {
        let ctx = AppContext::new(
            Box::new(crate::parser::TreeSitterProvider::new()),
            Default::default(),
        );
        let payload = build_inspect_payload(
            &snapshot(),
            &fresh_payloads_for_all_categories(),
            &Sections::all(),
            1,
            &ctx,
        );

        // These containers are the minimum top-level fields required in the
        // payload; the recursive walk still checks every descendant.
        for container in ["scanner_state", "summary", "details"] {
            assert!(
                payload.get(container).is_some(),
                "missing {container}: {payload}"
            );
        }
        assert_no_banned_field(&payload);
        assert_eq!(payload["summary"]["diagnostics"]["errors"], 2);
        assert_eq!(
            payload["details"]["diagnostics"].as_array().map(Vec::len),
            Some(1)
        );
        assert!(payload.get("topK").is_none());
        assert!(payload.get("top_k").is_none());
    }

    #[test]
    fn nonfresh_outcomes_cannot_reach_the_payload_emitter() {
        let outcomes = InspectCategory::active()
            .iter()
            .copied()
            .map(|category| {
                let outcome = if category == InspectCategory::Diagnostics {
                    JobOutcome::Pending { in_flight: true }
                } else {
                    JobOutcome::Fresh {
                        payload: serde_json::json!({}),
                    }
                };
                (category, outcome)
            })
            .collect();

        assert!(fresh_payloads(&outcomes).is_err());
    }
}

#[cfg(test)]
mod deferred_terminal_tests {
    use super::*;

    #[test]
    fn deferred_preflight_uses_one_terminal_poll_response() {
        let ctx = Arc::new(AppContext::new(
            Box::new(crate::parser::TreeSitterProvider::new()),
            crate::config::Config::default(),
        ));
        let request: RawRequest = serde_json::from_value(serde_json::json!({
            "id": "inspect-preflight",
            "command": "inspect"
        }))
        .expect("request parses");
        let mut deferred = match handle_inspect_deferred(&request, Arc::clone(&ctx)) {
            DispatchOutcome::Deferred(pending) => pending,
            DispatchOutcome::Immediate(_) => panic!("inspect must use the deferred seam"),
        };
        let response = (deferred.poll)(&ctx).expect("preflight terminal response");
        assert!(!response.success);
        assert!(response.data.get("failed_phase").is_none());
        assert_eq!(response.data["failure_reason"], "root_resolution_failed");
        assert!(
            (deferred.poll)(&ctx).is_none(),
            "terminal response must be emitted once"
        );
    }

    #[test]
    fn terminal_builder_uses_one_phase_shape_for_all_outcomes() {
        let log = InspectPhaseLog::for_request("inspect-terminal-shapes");
        log.start(InspectPhaseEntry::category(
            InspectPhaseId::StatVerification,
            InspectCategory::DeadCode,
        ))
        .complete();
        let fresh = build_inspect_terminal(
            "inspect-terminal-shapes",
            &log,
            InspectTerminal::Fresh(serde_json::json!({})),
        );
        assert_eq!(
            fresh.data["wait_stamp"]["phases"][0]["id"],
            "stat_verification"
        );
        let interrupted = build_inspect_terminal(
            "inspect-terminal-shapes",
            &log,
            InspectTerminal::Interrupted,
        );
        assert_eq!(
            interrupted.data["completed_phases"][0]["category"],
            "dead_code"
        );
        let failed = build_inspect_terminal(
            "inspect-terminal-shapes",
            &log,
            InspectTerminal::PhaseFailed {
                failed_phase: None,
                failure_reason: "missing_executable",
                failure_detail: None,
            },
        );
        assert_eq!(
            failed.data["completed_phases"][0]["id"],
            "stat_verification"
        );
        assert!(failed.data.get("failed_phase").is_none());
    }
}
