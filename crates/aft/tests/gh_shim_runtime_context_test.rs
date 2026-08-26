#![cfg(unix)]

#[path = "helpers/mod.rs"]
mod test_helpers;

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aft::config::Config;
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

fn write_fresh_r3_cache(state_home: &Path, now: u64) {
    let rung_path = state_home.join("cortexkit/aft/gh-shim/rung-cache.json");
    fs::create_dir_all(rung_path.parent().expect("rung cache parent"))
        .expect("create shim state directory");
    fs::write(
        rung_path,
        serde_json::to_vec(&json!({
            "rung": "R3",
            "as_of_unix_secs": now,
            "inputs": {
                "connection_file": "ready",
                "catalog_gh_route": "ready",
                "agent_binding": "ready",
                "manifest": "ready",
                "agent_credentials_present": "absent"
            },
            "manifest_version": 1
        }))
        .expect("serialize R3 rung cache"),
    )
    .expect("write R3 rung cache");
}

fn write_invalid_manifest(state_home: &Path, now: u64) {
    write_fresh_manifest(state_home, now);
    let manifest_path = state_home.join("cortexkit/aft/gh-shim/gh-routing-manifest.json");
    let mut envelope: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest envelope"))
            .expect("parse manifest envelope");
    envelope["key_id"] = json!("gh-routing-untrusted-test-key");
    fs::write(
        manifest_path,
        serde_json::to_vec(&envelope).expect("serialize invalid manifest envelope"),
    )
    .expect("write invalid manifest envelope");
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

fn write_upstream_gh_user_api(bin: &Path) {
    let gh = bin.join("gh");
    fs::create_dir_all(bin).expect("create fake upstream bin directory");
    fs::write(
        &gh,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$GH_SHIM_TEST_RECORD\"\n[ \"$1\" = api ] || exit 73\nprintf '289616620\\n'\n",
    )
    .expect("write fake upstream gh API");
    let mut permissions = fs::metadata(&gh)
        .expect("read fake upstream gh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh, permissions).expect("make fake upstream gh executable");
}

fn write_numeric_ids(state_home: &Path, ids: Value) {
    let path = state_home.join("cortexkit/aft/gh-shim/numeric-ids.json");
    fs::create_dir_all(path.parent().expect("numeric id cache parent"))
        .expect("create numeric id cache directory");
    fs::write(
        path,
        serde_json::to_vec(&ids).expect("serialize numeric ids"),
    )
    .expect("write numeric id cache");
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
fn gh_shim_governed_manifest_passthroughs_no_verb_and_help_invocations() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream_bin);
    let now = unix_seconds();
    write_fresh_manifest(&state_home, now);
    write_fresh_r3_cache(&state_home, now);
    write_user_config(&config_home, &connection_file, None);

    for args in [
        &[][..],
        &["--version"][..],
        &["--help"][..],
        &["-h"][..],
        &["help", "pr"][..],
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
        assert_eq!(
            output.status.code(),
            Some(73),
            "upstream passthrough: {args:?}"
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "r2-passthrough\n");
        assert!(
            output.stderr.is_empty(),
            "unexpected shim refusal: {args:?}"
        );
    }

    let undeclared_write = shim_command(
        &["release", "publish", "v1.0.0"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn undeclared governed gh shim invocation");
    assert_eq!(undeclared_write.status.code(), Some(86));
    assert!(undeclared_write.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&undeclared_write.stderr),
        "gh-shim: gh_shim_unclassified: no manifest declaration for this invocation (manifest 1)\n"
    );

    assert_eq!(
        fs::read_to_string(&recorder).expect("read upstream invocation record"),
        "\n--version\n--help\n-h\nhelp pr\n"
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
fn gh_shim_invalid_manifest_announces_ambient_credential_fallback() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream_bin);
    write_invalid_manifest(&state_home, unix_seconds());
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
    .expect("spawn invalid-manifest gh shim invocation");

    assert_eq!(output.status.code(), Some(73));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "r2-passthrough\n");
    assert_eq!(
        output.stderr,
        b"gh-shim: manifest invalid (untrusted manifest key id gh-routing-untrusted-test-key); executing with ambient gh credentials\n"
    );
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

#[test]
fn co_author_line_uses_the_cached_manifest_binding_and_numeric_id_offline() {
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
    write_numeric_ids(&state_home, json!({ "alfonso-aft": 289616620 }));
    write_user_config(&config_home, &connection_file, None);

    let output = shim_command(
        &["--co-author-line"],
        &project,
        &config_home,
        &state_home,
        &home,
        &upstream_bin,
        &recorder,
    )
    .output()
    .expect("spawn co-author self-report");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Co-authored-by: alfonso-aft <289616620+alfonso-aft@users.noreply.github.com>\n"
    );
    assert!(output.stderr.is_empty());
    assert!(
        !recorder.exists(),
        "a warm numeric-id cache must stay offline"
    );
}

#[test]
fn co_author_line_resolves_a_missing_numeric_id_once_and_caches_it() {
    let temp = tempfile::tempdir().expect("create test root");
    let config_home = temp.path().join("config");
    let state_home = temp.path().join("state");
    let home = temp.path().join("home");
    let project = write_project_repo(temp.path());
    let connection_file = write_dead_connection_file(temp.path());
    let upstream_bin = temp.path().join("upstream-bin");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh_user_api(&upstream_bin);
    write_fresh_manifest(&state_home, unix_seconds());
    write_user_config(&config_home, &connection_file, None);

    for _ in 0..2 {
        let output = shim_command(
            &["--co-author-line"],
            &project,
            &config_home,
            &state_home,
            &home,
            &upstream_bin,
            &recorder,
        )
        .output()
        .expect("spawn co-author self-report");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "Co-authored-by: alfonso-aft <289616620+alfonso-aft@users.noreply.github.com>\n"
        );
    }

    assert_eq!(
        fs::read_to_string(&recorder).expect("read API invocation record"),
        "api users/alfonso-aft --jq .id\n"
    );
    let ids: Value = serde_json::from_slice(
        &fs::read(state_home.join("cortexkit/aft/gh-shim/numeric-ids.json"))
            .expect("read numeric id cache"),
    )
    .expect("parse numeric id cache");
    assert_eq!(ids["alfonso-aft"], 289616620);
}

#[test]
fn auto_child_hook_commits_the_cached_bound_identity_exactly_once_on_amend() {
    let temp = tempfile::tempdir().expect("create test root");
    let state_home = temp.path().join("state");
    let config_home = temp.path().join("config");
    let home = temp.path().join("home");
    let storage = temp.path().join("storage");
    let project = write_project_repo(temp.path());
    write_fresh_manifest(&state_home, unix_seconds());
    write_numeric_ids(&state_home, json!({ "alfonso-aft": 289616620 }));
    fs::write(project.join("tracked.txt"), "joint work\n").expect("write tracked file");
    for args in [
        &["config", "user.name", "AFT Test"][..],
        &["config", "user.email", "aft-test@example.test"][..],
        &["add", "tracked.txt"][..],
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(&project)
            .status()
            .expect("run git setup command");
        assert!(status.success(), "git setup failed: {args:?}");
    }

    let binary = aft_binary();
    test_helpers::warm_executable(&binary, &["--version"]);
    let mut config = Config::default();
    config.gh_shim.enabled = false;
    config.gh_shim.binary_path = Some(binary);
    config.git.co_author = "auto".to_string();
    let inherited_path = std::env::var_os("PATH").expect("test PATH");
    let mut environment = std::collections::HashMap::from([
        (
            "PATH".to_string(),
            inherited_path.to_string_lossy().into_owned(),
        ),
        (
            "XDG_STATE_HOME".to_string(),
            state_home.to_string_lossy().into_owned(),
        ),
        (
            "XDG_CONFIG_HOME".to_string(),
            config_home.to_string_lossy().into_owned(),
        ),
        ("HOME".to_string(), home.to_string_lossy().into_owned()),
    ]);
    aft::agent_child_env::inject(&config, &storage, &mut environment)
        .expect("inject child Git environment");

    for args in [
        &["commit", "--quiet", "-m", "mason: joint work"][..],
        &["commit", "--quiet", "--amend", "--no-edit"][..],
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(&project)
            .envs(&environment)
            .status()
            .expect("run governed commit");
        assert!(status.success(), "governed commit failed: {args:?}");
    }

    let output = Command::new("git")
        .args(["log", "-1", "--format=%B"])
        .current_dir(&project)
        .output()
        .expect("read commit message");
    assert!(output.status.success());
    let message = String::from_utf8(output.stdout).expect("commit message UTF-8");
    assert_eq!(message.matches("Co-authored-by:").count(), 1);
    assert!(message
        .contains("Co-authored-by: alfonso-aft <289616620+alfonso-aft@users.noreply.github.com>"));
}

#[test]
fn shim_invoked_as_gh_skips_its_managed_path_entry_and_execs_upstream_once() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("create test root");
    let project = write_project_repo(temp.path());
    let shims = temp.path().join("shims");
    let upstream = temp.path().join("upstream");
    let recorder = temp.path().join("upstream-invocations.txt");
    fs::create_dir_all(&shims).expect("create shims directory");
    symlink(aft_binary(), shims.join("gh")).expect("create managed gh link");
    write_upstream_gh(&upstream);
    let inherited = std::env::var_os("PATH").expect("test PATH");
    let path = std::env::join_paths(
        [shims.clone(), upstream]
            .into_iter()
            .chain(std::env::split_paths(&inherited)),
    )
    .expect("build shim PATH");

    let output = Command::new(shims.join("gh"))
        .args(["issue", "list"])
        .current_dir(project)
        .env("PATH", path)
        .env("AFT_GH_SHIMS_DIR", &shims)
        .env("GH_SHIM_TEST_RECORD", &recorder)
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("spawn managed gh entry");

    assert_eq!(output.status.code(), Some(73));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "r2-passthrough\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(recorder).expect("read upstream invocation record"),
        "issue list\n"
    );
}

#[test]
fn co_author_line_is_silently_empty_without_a_cached_manifest_binding() {
    let temp = tempfile::tempdir().expect("create test root");
    let project = write_project_repo(temp.path());
    let upstream = temp.path().join("upstream");
    let recorder = temp.path().join("upstream-invocations.txt");
    write_upstream_gh(&upstream);

    let output = shim_command(
        &["--co-author-line"],
        &project,
        &temp.path().join("config"),
        &temp.path().join("state"),
        &temp.path().join("home"),
        &upstream,
        &recorder,
    )
    .output()
    .expect("spawn inert co-author self-report");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!recorder.exists());
}
