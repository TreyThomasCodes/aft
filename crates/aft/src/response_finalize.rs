use crate::context::AppContext;
use crate::protocol::Response;

/// Apply finalizers in the established response order: background completions first, then status bar counts.
pub fn finalize_response(
    response: &mut Response,
    ctx: &AppContext,
    session_id: &str,
    attach_command: &str,
) {
    finalize_response_with_bg_completions(response, ctx, session_id, attach_command, true);
}

pub fn finalize_response_with_bg_completions(
    response: &mut Response,
    ctx: &AppContext,
    session_id: &str,
    attach_command: &str,
    allow_bg_completions: bool,
) {
    if allow_bg_completions {
        attach_bg_completions(response, ctx, session_id, attach_command);
    }
    attach_status_bar(response, ctx, session_id, attach_command);
}

pub enum DispatchOutcome {
    Immediate(Response),
    Deferred(PendingResponse),
}

pub type PendingResponsePoll = Box<dyn FnMut(&AppContext) -> Option<Response>>;

pub struct PendingResponse {
    pub request_id: String,
    pub session_id: String,
    pub attach_command: String,
    pub poll: PendingResponsePoll,
}

pub struct ResolvedPending {
    pub response: Response,
    pub session_id: String,
    pub attach_command: String,
}

#[derive(Default)]
pub struct PendingResponses {
    entries: Vec<PendingResponse>,
}

impl PendingResponses {
    pub fn register(&mut self, pending: PendingResponse) {
        self.entries
            .retain(|entry| entry.request_id != pending.request_id);
        self.entries.push(pending);
    }

    pub fn poll_ready(&mut self, ctx: &AppContext) -> Vec<ResolvedPending> {
        let mut ready = Vec::new();
        let mut waiting = Vec::with_capacity(self.entries.len());

        for mut pending in self.entries.drain(..) {
            if let Some(response) = (pending.poll)(ctx) {
                ready.push(ResolvedPending {
                    response,
                    session_id: pending.session_id,
                    attach_command: pending.attach_command,
                });
            } else {
                waiting.push(pending);
            }
        }

        self.entries = waiting;
        ready
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn drain_on_shutdown(&mut self) {
        self.entries.clear();
    }
}

pub fn attach_bg_completions(
    response: &mut Response,
    ctx: &AppContext,
    session_id: &str,
    command: &str,
) {
    if matches!(
        command,
        "configure"
            | "bash_abort_inflight"
            | "bash_status"
            | "bash_write"
            | "bash_promote"
            | "bash_wait_detach"
            | "bash_regex_match"
            | "bash_drain_completions"
            | "bash_notify"
            | "bash_unnotify"
            | "bash_ack_completions"
    ) {
        return;
    }
    if !ctx
        .bash_background()
        .has_completions_for_session(Some(session_id))
    {
        return;
    }
    let completions = ctx
        .bash_background()
        .drain_completions_for_session(Some(session_id));
    if completions.is_empty() {
        return;
    }
    let value = serde_json::json!(completions);
    match response.data.as_object_mut() {
        Some(data) => {
            data.insert("bg_completions".to_string(), value);
        }
        None => {
            response.data = serde_json::json!({ "bg_completions": value });
        }
    }
}

fn aft_status_segment(counts: &crate::context::StatusBarCounts) -> String {
    let stale_mark = if counts.tier2_stale { "~" } else { "" };
    format!(
        "E{} W{} | {}D{} U{} C{} | T{}",
        counts.errors,
        counts.warnings,
        stale_mark,
        counts.dead_code,
        counts.unused_exports,
        counts.duplicates,
        counts.todos
    )
}

fn holder_owns_status_bar(plane_live: bool, harness: Option<&crate::harness::Harness>) -> bool {
    plane_live && matches!(harness, Some(crate::harness::Harness::Opencode))
}

/// Attach the agent status-bar counts to the response envelope so the plugin
/// after-hook can surface the IDE-style status bar (emit-on-change). Skips
/// internal/transport commands that don't represent agent tool calls (their
/// responses never reach the agent, and bash-lifecycle commands fire rapidly).
/// `errors`/`warnings` are read live from the LSP store. Tier-2 and todo counts
/// come from a cached snapshot, so the payload stays omitted until that snapshot
/// has been populated.
pub fn attach_status_bar(
    response: &mut Response,
    ctx: &AppContext,
    _session_id: &str,
    command: &str,
) {
    // Cross-root indexed searches report on a borrowed project, so attaching the
    // session project's diagnostics footer would falsely attribute unrelated
    // counts to the external results. The command sets this private marker and
    // the finalizer removes it before the response reaches the caller.
    if response
        .data
        .as_object_mut()
        .and_then(|data| data.remove("_aft_suppress_status_bar"))
        .is_some()
    {
        return;
    }
    if matches!(
        command,
        "configure"
            | "ping"
            | "version"
            | "status"
            | "bash_abort_inflight"
            | "bash_status"
            | "bash_write"
            | "bash_promote"
            | "bash_wait_detach"
            | "bash_regex_match"
            | "bash_drain_completions"
            | "bash_notify"
            | "bash_unnotify"
            | "bash_ack_completions"
    ) {
        return;
    }
    let local_counts = ctx.status_bar_counts();
    let plane_live = ctx.fleet_status_client().is_some_and(|client| {
        let config = ctx.config();
        let Some(project_root) = config.project_root.as_deref() else {
            return false;
        };
        let aft_text = local_counts
            .as_ref()
            .map(aft_status_segment)
            .unwrap_or_default();
        client.publish(project_root, &aft_text)
    });
    let harness = ctx.harness_opt();
    if holder_owns_status_bar(plane_live, harness.as_ref()) {
        return;
    }
    let Some(counts) = local_counts else {
        return;
    };
    if !ctx.should_emit_status_bar(&counts) {
        return;
    }
    let value = serde_json::json!({
        "errors": counts.errors,
        "warnings": counts.warnings,
        "dead_code": counts.dead_code,
        "unused_exports": counts.unused_exports,
        "duplicates": counts.duplicates,
        "todos": counts.todos,
        "tier2_stale": counts.tier2_stale,
    });
    match response.data.as_object_mut() {
        Some(data) => {
            data.insert("status_bar".to_string(), value);
        }
        None => {
            response.data = serde_json::json!({ "status_bar": value });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{aft_status_segment, holder_owns_status_bar};
    use crate::context::StatusBarCounts;
    use crate::harness::Harness;

    #[test]
    fn live_holder_retires_only_opencode_response_bars() {
        assert_eq!(
            (
                holder_owns_status_bar(true, Some(&Harness::Opencode)),
                holder_owns_status_bar(true, Some(&Harness::Runner)),
            ),
            (true, false)
        );
        assert!(!holder_owns_status_bar(true, Some(&Harness::Pi)));
        assert!(!holder_owns_status_bar(false, Some(&Harness::Opencode)));
    }

    #[test]
    fn solo_bar_bytes_remain_the_existing_golden() {
        let counts = StatusBarCounts {
            errors: 2,
            warnings: 5,
            dead_code: 331,
            unused_exports: 221,
            duplicates: 1159,
            todos: 8,
            tier2_stale: false,
        };
        assert_eq!(
            format!("[AFT {}]", aft_status_segment(&counts)),
            "[AFT E2 W5 | D331 U221 C1159 | T8]"
        );
    }

    #[test]
    fn solo_bar_stale_marker_bytes_remain_the_existing_golden() {
        let counts = StatusBarCounts {
            dead_code: 10,
            tier2_stale: true,
            ..StatusBarCounts::default()
        };
        assert_eq!(
            format!("[AFT {}]", aft_status_segment(&counts)),
            "[AFT E0 W0 | ~D10 U0 C0 | T0]"
        );
    }
}
