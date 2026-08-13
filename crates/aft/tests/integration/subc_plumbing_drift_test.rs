//! Drift guard: every native (non-agent) bridge command the plugins actually
//! send must be admitted by the subc gate, either as an agent core tool or on
//! the native-plumbing allowlist. History: bash_wait_detach, bash_kill,
//! bash_write, bash_notify and inspect_tier2_run each shipped in the plugins
//! without a gate entry and were silently rejected (fail-closed) under the
//! daemon transport until a log sweep caught the rejections in production.
//!
//! When this test fails you are adding a new plugin-side native bridge command:
//! admit it in `is_subc_native_plumbing_tool` (crates/aft/src/subc/manifest.rs)
//! with a rationale comment, or route it through the agent tool manifest.

use aft::subc::is_tool_call_admitted_for_test;

/// Native commands the OpenCode/Pi plugins send over the bound route today.
/// Sourced from the plugins' `send("...")` call sites (production code only,
/// not tests). Keep in sync when adding plugin bridge calls.
const PLUGIN_NATIVE_SENDS: &[&str] = &[
    "bash_abort_inflight",
    "bash_status",
    "bash_drain_completions",
    "bash_ack_completions",
    "bash_kill",
    "bash_write",
    "bash_notify",
    "bash_unnotify",
    "bash_wait_detach",
    "undo_preview",
    "checkpoint_paths",
    "inspect_tier2_run",
];

/// Native hashline commands invoked through plugin `callToolCall(...)` sites.
const PLUGIN_HASHLINE_NATIVE_CALLS: &[&str] = &["hashline_preflight"];

#[test]
fn plugin_native_calls_are_admitted_by_subc_gate() {
    let rejected: Vec<&str> = PLUGIN_NATIVE_SENDS
        .iter()
        .chain(PLUGIN_HASHLINE_NATIVE_CALLS)
        .copied()
        .filter(|name| !is_tool_call_admitted_for_test(name))
        .collect();
    assert!(
        rejected.is_empty(),
        "plugin native bridge commands rejected by the subc fail-closed gate \
         (add to is_subc_native_plumbing_tool with rationale): {rejected:?}"
    );
}

#[test]
fn plugin_hashline_native_calls_have_main_dispatch_arms() {
    let main_dispatch = include_str!("../../src/main.rs");
    for name in PLUGIN_HASHLINE_NATIVE_CALLS {
        let dispatch_arm = format!("\"{name}\" =>");
        assert!(
            main_dispatch.contains(&dispatch_arm),
            "plugin hashline plumbing command {name:?} has no main dispatch arm"
        );
    }
}
