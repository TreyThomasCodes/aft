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
    fs::write(&gh, "#!/bin/sh\nprintf 'r2-passthrough\\n'\nexit 73\n")
        .expect("write fake upstream gh");
    let mut permissions = fs::metadata(&gh)
        .expect("read fake upstream gh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh, permissions).expect("make fake upstream gh executable");
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
    write_upstream_gh(&upstream_bin);
    write_fresh_manifest(&state_home, unix_seconds());

    let config_dir = config_home.join("cortexkit");
    fs::create_dir_all(&config_dir).expect("create user config directory");
    fs::write(
        config_dir.join("aft.jsonc"),
        format!(
            "{{\n  \"subc\": {{ \"connection_file\": \"{}\" }}\n}}\n",
            connection_file.display()
        ),
    )
    .expect("write user config");

    let inherited_path = std::env::var_os("PATH").expect("test PATH");
    let path = std::env::join_paths(
        std::iter::once(upstream_bin.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("build test PATH");
    let mut shim = Command::new(aft_binary());
    shim.args(["gh-shim", "issue", "list"])
        .current_dir(&project)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("HOME", &home)
        .env("PATH", path)
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN");
    let output = shim.output().expect("spawn gh shim");

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

    let status = Command::new(aft_binary())
        .args(["gh-shim", "--status"])
        .current_dir(&project)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("HOME", &home)
        .env("PATH", &upstream_bin)
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
