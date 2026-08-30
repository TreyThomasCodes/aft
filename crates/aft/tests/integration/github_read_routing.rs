#![cfg(unix)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use aft::commands::read::build_read_outcome;
use aft::config::Config;
use aft::context::{default_language_provider_factory, AppContext};
use aft::protocol::RawRequest;
use aft::response_finalize::DispatchOutcome;
use aft::subc_translate::{subc_translate_with_context, TranslateContext};
use serde_json::{json, Value};

use super::helpers::AftProcess;

fn write_fake_gh(bin_dir: &Path) {
    fs::create_dir_all(bin_dir).expect("create fake gh directory");
    let script = bin_dir.join("gh");
    fs::write(
        &script,
        r#"#!/bin/sh
if [ -n "${AFT_TEST_GH_STARTED:-}" ]; then
  : > "$AFT_TEST_GH_STARTED"
fi
if [ -n "${AFT_TEST_GH_DELAY_SECONDS:-}" ]; then
  sleep "$AFT_TEST_GH_DELAY_SECONDS"
fi
case "$1" in
  issue)
    printf '%s\n' '{"number":7,"title":"Fixture issue","state":"OPEN","body":"one\ntwo\nthree\nhttps://github.com/user-attachments/files/7/screenshot.png","url":"https://github.com/owner/repo/issues/7","comments":[]}'
    ;;
  pr)
    printf '%s\n' '{"number":9,"title":"Fixture pull request","state":"OPEN","body":"pull request body","url":"https://github.com/owner/repo/pull/9","comments":[],"files":[],"reviews":[]}'
    ;;
  api)
    printf '%s\n' '{"data":{"repository":{"nameWithOwner":"owner/repo","pullRequest":{"number":9,"title":"Fixture pull request","state":"OPEN","body":"pull request body","comments":[],"files":[],"reviews":[]}}}}'
    ;;
  *)
    exit 1
    ;;
esac
"#,
    )
    .expect("write fake gh");
    let mut permissions = fs::metadata(&script).expect("stat fake gh").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("make fake gh executable");
}

fn spawned_with_fake_gh(bin_dir: &Path) -> AftProcess {
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin_dir.to_path_buf()).chain(std::env::split_paths(&original_path)),
    )
    .expect("join fake gh path entries");
    AftProcess::spawn_with_env(&[("PATH", path.as_os_str())])
}

fn spawned_with_slow_fake_gh(bin_dir: &Path, started: &Path) -> AftProcess {
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin_dir.to_path_buf()).chain(std::env::split_paths(&original_path)),
    )
    .expect("join fake gh path entries");
    AftProcess::spawn_with_env(&[
        ("PATH", path.as_os_str()),
        ("AFT_TEST_GH_STARTED", started.as_os_str()),
        ("AFT_TEST_GH_DELAY_SECONDS", OsStr::new("2")),
    ])
}

/// Configure a throwaway project whose config enables the gh_read gate; the
/// gate's default-off contract is pinned by the dedicated disabled-read tests.
fn configure_gh_read_enabled(aft: &mut AftProcess, project: &Path) {
    let config_dir = project.join(".cortexkit");
    fs::create_dir_all(&config_dir).expect("create project config dir");
    fs::write(
        config_dir.join("aft.jsonc"),
        "{\"gh_read\": {\"enabled\": true}}",
    )
    .expect("write gh_read project config");
    let response = aft.send(
        &json!({
            "id": "configure-gh-read",
            "command": "configure",
            "harness": "opencode",
            "project_root": project,
        })
        .to_string(),
    );
    assert_eq!(response["success"], true, "configure failed: {response:#}");
}

fn read_response(aft: &mut AftProcess, id: &str, file: &str, params: Value) -> Value {
    let mut request = params.as_object().cloned().expect("read params object");
    request.insert("id".to_string(), Value::String(id.to_string()));
    request.insert("command".to_string(), Value::String("read".to_string()));
    request.insert("file".to_string(), Value::String(file.to_string()));
    aft.send(&Value::Object(request).to_string())
}

#[test]
fn github_read_routes_short_and_explicit_forms_without_filesystem_rendering() {
    let temp = tempfile::tempdir().expect("create fake gh root");
    write_fake_gh(&temp.path().join("bin"));
    let mut aft = spawned_with_fake_gh(&temp.path().join("bin"));
    configure_gh_read_enabled(&mut aft, temp.path());

    let issue = read_response(&mut aft, "github-issue", "issue://7", json!({}));
    assert_eq!(issue["success"], true, "issue read failed: {issue:#}");
    assert_eq!(issue["content"].as_str(), Some("# Issue #7: Fixture issue\n\nRepository: owner/repo\nState: OPEN\n\n## Body\n\none\ntwo\nthree\nhttps://github.com/user-attachments/files/7/screenshot.png\n\n"));
    assert_eq!(
        issue["attachments"],
        json!([]),
        "missing vision capability must be false"
    );
    assert!(
        !issue["content"]
            .as_str()
            .unwrap_or_default()
            .starts_with("1: "),
        "GitHub text must remain the engine's canonical bytes"
    );

    let pull_request = read_response(&mut aft, "github-pr", "pr://owner/repo/9", json!({}));
    assert_eq!(
        pull_request["success"], true,
        "explicit PR read failed: {pull_request:#}"
    );
    assert_eq!(
        pull_request["content"].as_str(),
        Some("# Pull request #9: Fixture pull request\n\nRepository: owner/repo\nState: OPEN\n\n## Body\n\npull request body\n\n")
    );

    assert!(aft.shutdown().success());
}

#[test]
fn standalone_deferred_github_read_keeps_interleaved_request_responsive() {
    let temp = tempfile::tempdir().expect("create slow fake gh root");
    let bin_dir = temp.path().join("bin");
    write_fake_gh(&bin_dir);
    let fetch_started = temp.path().join("gh-fetch-started");
    let mut aft = spawned_with_slow_fake_gh(&bin_dir, &fetch_started);
    configure_gh_read_enabled(&mut aft, temp.path());

    aft.send_silent(
        &json!({
            "id": "slow-github-read",
            "command": "read",
            "file": "issue://7",
        })
        .to_string(),
    );
    let fetch_deadline = Instant::now() + Duration::from_secs(5);
    while !fetch_started.exists() {
        assert!(
            Instant::now() < fetch_deadline,
            "slow GitHub fetch did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let sibling = aft.send_with_timeout(
        &json!({
            "id": "interleaved-ping",
            "command": "ping",
        })
        .to_string(),
        Duration::from_millis(500),
    );
    assert_eq!(sibling["id"], "interleaved-ping");
    assert_eq!(sibling["success"], true);

    let github = loop {
        let frame = aft
            .try_read_next_timeout(Duration::from_secs(3))
            .expect("deferred GitHub read completes");
        if frame["id"] == "slow-github-read" {
            break frame;
        }
        assert!(
            frame.get("type").is_some() && frame.get("id").is_none(),
            "unexpected frame before deferred GitHub response: {frame:#}"
        );
    };
    assert_eq!(github["success"], true, "GitHub read failed: {github:#}");
    assert!(aft.shutdown().success());
}

#[test]
fn github_read_subc_translation_preserves_uri_selector_and_hidden_vision_capability() {
    let project = Path::new("/project");
    let with_vision = subc_translate_with_context(
        "read",
        &json!({
            "path": "issue://owner/repo/7",
            "offset": 3,
            "limit": 2,
            "vision_capability": true,
        }),
        project,
        TranslateContext::default(),
    )
    .expect("translate GitHub read with vision capability");
    assert_eq!(with_vision.command, "read");
    assert_eq!(with_vision.args["file"], "issue://owner/repo/7");
    assert_eq!(with_vision.args["start_line"], 3);
    assert_eq!(with_vision.args["end_line"], 4);
    assert_eq!(with_vision.args["vision_capability"], true);

    let without_vision = subc_translate_with_context(
        "read",
        &json!({ "path": "pr://9" }),
        project,
        TranslateContext::default(),
    )
    .expect("translate GitHub read without vision capability");
    assert!(
        without_vision.args.get("vision_capability").is_none(),
        "an absent hidden capability must remain absent for the read handler to treat as false"
    );
}

#[test]
fn github_read_returns_identical_selected_content_through_standalone_and_subc_paths() {
    let temp = tempfile::tempdir().expect("create fake gh root");
    write_fake_gh(&temp.path().join("bin"));
    let mut standalone = spawned_with_fake_gh(&temp.path().join("bin"));
    let mut subc = spawned_with_fake_gh(&temp.path().join("bin"));
    configure_gh_read_enabled(&mut standalone, temp.path());
    configure_gh_read_enabled(&mut subc, temp.path());

    let direct = read_response(
        &mut standalone,
        "github-selector-direct",
        "issue://7",
        json!({ "start_line": 3, "end_line": 4 }),
    );
    let translated = subc.send(
        &json!({
            "id": "github-selector-subc",
            "command": "tool_call",
            "session_id": "github-selector-session",
            "name": "read",
            "arguments": {
                "path": "issue://7",
                "offset": 3,
                "limit": 2,
            },
        })
        .to_string(),
    );

    assert_eq!(
        direct["success"], true,
        "standalone read failed: {direct:#}"
    );
    assert_eq!(
        translated["success"], true,
        "subc read failed: {translated:#}"
    );
    assert_eq!(direct["content"], "Repository: owner/repo\nState: OPEN\n");
    for field in [
        "content",
        "attachments",
        "total_lines",
        "lines_read",
        "start_line",
        "end_line",
        "truncated",
        "complete",
    ] {
        assert_eq!(
            direct[field], translated[field],
            "standalone/subc mismatch for {field}: standalone={direct:#} subc={translated:#}"
        );
    }

    assert!(standalone.shutdown().success());
    assert!(subc.shutdown().success());
}

#[test]
fn github_read_forced_restrict_refuses_before_any_filesystem_or_gh_work() {
    let project = tempfile::tempdir().expect("create restricted project");
    let ctx = AppContext::new(
        default_language_provider_factory(),
        Config {
            project_root: Some(project.path().to_path_buf()),
            ..Config::default()
        },
    );
    let request = RawRequest {
        id: "restricted-github-read".to_string(),
        command: "read".to_string(),
        lsp_hints: None,
        session_id: None,
        params: json!({ "file": "issue://7" }),
    };

    let request_id = request.id.clone();
    let outcome = ctx.with_force_restrict(&request_id, || build_read_outcome(request, &ctx));
    let DispatchOutcome::Immediate(response) = outcome else {
        panic!("restricted GitHub read must refuse synchronously");
    };
    assert!(!response.success);
    assert_eq!(response.data["code"], "external_fetch_restricted");
    assert!(
        response.data["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Network-backed GitHub reads are unavailable on restricted binds"),
        "restricted response must explain the refusal: {:#}",
        response.data
    );
}
