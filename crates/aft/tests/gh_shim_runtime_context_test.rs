#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use ring::signature::Ed25519KeyPair;
use serde_json::{json, Value};
use subc_transport::connection_file::{self, ConnectionInfo, Endpoint, SCHEMA_VERSION};
use subc_transport::{DAEMON_ID_LEN, KEY_LEN};

const DEV_MANIFEST_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

fn aft_binary() -> PathBuf {
    std::env::var_os("AFT_TEST_AFT_BINARY")
        .or_else(|| std::env::var_os("NEXTEST_BIN_EXE_aft"))
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_aft"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_aft")))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_secs()
}

fn write_fresh_manifest(state_home: &Path, now: u64) {
    let mut manifest: Value =
        serde_json::from_str(include_str!("fixtures/gh_shim/initial-manifest-v1.json"))
            .expect("parse manifest fixture");
    manifest["issued_at_unix_secs"] = json!(now);
    let manifest_bytes = serde_json::to_vec(&manifest).expect("serialize fresh manifest");
    let key = Ed25519KeyPair::from_seed_unchecked(&DEV_MANIFEST_SEED).expect("build test key");
    let envelope = json!({
        "artifact_id": "gh-routing-manifest",
        "envelope_version": 2,
        "key_id": "gh-routing-dev-test-key-v1",
        "fetched_at_unix_secs": now,
        "signature": base64::engine::general_purpose::STANDARD.encode(key.sign(&manifest_bytes).as_ref()),
        "manifest_bytes": String::from_utf8(manifest_bytes).expect("manifest fixture is UTF-8"),
    });
    let manifest_path = state_home.join("cortexkit/aft/gh-shim/gh-routing-manifest.json");
    fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
        .expect("create shim state directory");
    fs::write(
        manifest_path,
        serde_json::to_vec(&envelope).expect("serialize manifest envelope"),
    )
    .expect("write manifest envelope");
}

fn write_dead_connection_file(root: &Path) -> PathBuf {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a loopback port");
    let port = listener
        .local_addr()
        .expect("read reserved loopback address")
        .port();
    drop(listener);

    let path = root.join("subc-connection.json");
    let connection = ConnectionInfo {
        schema: SCHEMA_VERSION,
        wire_version: None,
        endpoints: vec![Endpoint {
            host: "127.0.0.1".to_string(),
            port,
        }],
        key: vec![0x42; KEY_LEN],
        daemon_id: [0x24; DAEMON_ID_LEN],
        pid: std::process::id(),
        daemon_ver: "gh-shim-runtime-context-test".to_string(),
    };
    connection_file::write_atomic(&path, &connection).expect("write dead-daemon connection file");
    path
}

fn write_project_repo(root: &Path) -> PathBuf {
    let project = root.join("project");
    fs::create_dir_all(&project).expect("create project directory");
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&project)
        .status()
        .expect("initialize project repository");
    assert!(initialized.success(), "git init failed: {initialized}");
    let remote_added = Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/cortexkit/aft.git",
        ])
        .current_dir(&project)
        .status()
        .expect("configure project origin");
    assert!(
        remote_added.success(),
        "git remote add failed: {remote_added}"
    );
    project
}

fn write_upstream_gh(bin: &Path) {
    let gh = bin.join("gh");
    fs::create_dir_all(bin).expect("create fake upstream bin directory");
    fs::write(
        &gh,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$GH_SHIM_TEST_RECORD\"\nprintf 'r2-passthrough\\n'\nexit 73\n",
    )
    .expect("write fake upstream gh");
    let mut permissions = fs::metadata(&gh)
        .expect("read fake upstream gh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh, permissions).expect("make fake upstream gh executable");
}

fn write_user_config(config_home: &Path, connection_file: &Path, enabled: Option<bool>) {
    let config_dir = config_home.join("cortexkit");
    fs::create_dir_all(&config_dir).expect("create user config directory");
    let mut config = json!({
        "subc": { "connection_file": connection_file },
    });
    if let Some(enabled) = enabled {
        config["gh_shim"] = json!({ "enabled": enabled });
    }
    fs::write(
        config_dir.join("aft.jsonc"),
        serde_json::to_vec_pretty(&config).expect("serialize user config"),
    )
    .expect("write user config");
}

fn shim_command(
    args: &[&str],
    project: &Path,
    config_home: &Path,
    state_home: &Path,
    home: &Path,
    upstream_bin: &Path,
    recorder: &Path,
) -> Command {
    let inherited_path = std::env::var_os("PATH").expect("test PATH");
    let path = std::env::join_paths(
        std::iter::once(upstream_bin.to_path_buf()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("build test PATH");
    let mut shim = Command::new(aft_binary());
    shim.arg("gh-shim")
        .args(args)
        .current_dir(project)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_STATE_HOME", state_home)
        .env("HOME", home)
        .env("PATH", path)
        .env("GH_SHIM_TEST_RECORD", recorder)
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GH_SHIM_BYPASS");
    shim
}

#[test]
fn gh_shim_daemon_probe_from_sync_entry_is_r2_without_a_runtime_panic() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream_bin);
    write_fresh_manifest(&state_home, unix_seconds());
    write_user_config(&config_home, &connection_file, None);

    let output = shim_command(
        &["issue", "list"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn gh shim");

    assert_eq!(
        output.status.code(),
        Some(73),
        "R2 must delegate to the upstream gh stand-in; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "r2-passthrough\n",
        "the shim should pass the command through after determining R2"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked at"),
        "the sync CLI probe must not require a pre-existing Tokio runtime: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(&recorder).expect("read upstream invocation record"),
        "issue list\n"
    );

    let status = shim_command(
        &["--status"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn gh shim status");
    assert!(
        status.status.success(),
        "status should read the recorded rung: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let report: Value = serde_json::from_slice(&status.stdout).expect("parse gh shim status JSON");
    assert_eq!(report["last_rung"]["rung"], "R2");
    assert_eq!(
        report["last_rung"]["determination_inputs"]["daemon_unreachable"],
        "failed"
    );
}

#[test]
fn gh_shim_governed_binding_refuses_writes_when_daemon_is_unreachable() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream_bin);
    write_fresh_manifest(&state_home, unix_seconds());
    write_user_config(&config_home, &connection_file, None);

    let expected_stderr = "gh-shim: gh_shim_governance_unavailable: the governance daemon is unreachable and this repository's actions are identity-governed; retry after the daemon returns\n";
    for args in [
        &["issue", "comment", "42", "--body", "hello"][..],
        &["pr", "merge", "42"][..],
    ] {
        let output = shim_command(
            args,
            &project,
            &config_home,
            &state_home,
            &home,
            &upstream_bin,
            &recorder,
        )
        .output()
        .expect("spawn governed gh shim invocation");
        assert_eq!(output.status.code(), Some(86));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&output.stderr), expected_stderr);
        assert!(
            !recorder.exists(),
            "governed and admin actions must not reach upstream gh"
        );
    }

    let unclassified = shim_command(
        &["alias", "set", "shortcut", "issue list"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn unclassified gh shim invocation");
    assert_eq!(unclassified.status.code(), Some(86));
    assert!(unclassified.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&unclassified.stderr),
        "gh-shim: gh_shim_unclassified: no manifest declaration for this invocation (manifest 1)\n"
    );
    assert!(
        !recorder.exists(),
        "unclassified actions on a governed repository must not reach upstream gh"
    );

    let status = shim_command(
        &["--status"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn gh shim status");
    assert!(status.status.success());
    let report: Value = serde_json::from_slice(&status.stdout).expect("parse gh shim status JSON");
    assert_eq!(
        report["last_seam_refusal"]["code"],
        "gh_shim_governance_unavailable"
    );

    let mechanical = shim_command(
        &["issue", "view", "42"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn mechanical gh shim invocation");
    assert_eq!(mechanical.status.code(), Some(73));
    assert_eq!(
        String::from_utf8_lossy(&mechanical.stdout),
        "r2-passthrough\n"
    );
    assert_eq!(
        fs::read_to_string(&recorder).expect("read upstream invocation record"),
        "issue view 42\n"
    );
}

#[test]
fn gh_shim_without_manifest_keeps_unreachable_daemon_passthrough() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream_bin);
    write_user_config(&config_home, &connection_file, None);

    let output = shim_command(
        &["issue", "comment", "42", "--body", "hello"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn dormant gh shim invocation");
    assert_eq!(output.status.code(), Some(73));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "r2-passthrough\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(&recorder).expect("read upstream invocation record"),
        "issue comment 42 --body hello\n"
    );
}

#[test]
fn gh_shim_disabled_by_config_overrides_governance_stickiness() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream_bin);
    write_fresh_manifest(&state_home, unix_seconds());
    write_user_config(&config_home, &connection_file, Some(false));

    let output = shim_command(
        &["issue", "comment", "42", "--body", "hello"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn disabled gh shim invocation");
    assert_eq!(output.status.code(), Some(73));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "r2-passthrough\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(&recorder).expect("read upstream invocation record"),
        "issue comment 42 --body hello\n"
    );
}
