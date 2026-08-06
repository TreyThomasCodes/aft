#![cfg(unix)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::time::{Duration, Instant};

use serde_json::json;

use super::helpers::{user_config, AftProcess};

const SHORT_WAIT_MS: &str = "600";

fn spawn_with_wait(wait_ms: &str) -> AftProcess {
    AftProcess::spawn_with_env(&[("AFT_TEST_FOREGROUND_WAIT_MS", OsStr::new(wait_ms))])
}

fn configure_bash_background(
    aft: &mut AftProcess,
    dir: &tempfile::TempDir,
    configured_wait_ms: u64,
) {
    let response = aft.send(
        &json!({
            "id": "cfg-bash-orchestrate",
            "command": "configure",
            "harness": "opencode",
            "project_root": dir.path(),
            "storage_dir": dir.path().join("storage"),
            "config": user_config(json!({
                "bash": {
                    "background": true,
                    "foreground_wait_window_ms": configured_wait_ms,
                }
            })),
        })
        .to_string(),
    );
    assert_eq!(response["success"], true, "configure failed: {response:?}");
}

fn bash_request(id: &str, params: serde_json::Value) -> String {
    json!({
        "id": id,
        "method": "bash",
        "params": params,
    })
    .to_string()
}

#[test]
fn orchestrated_fast_foreground_returns_single_terminal_response() {
    let mut aft = spawn_with_wait(SHORT_WAIT_MS);

    let response = aft.send(&bash_request(
        "bash-fast-orchestrated",
        json!({
            "command": "echo hi",
            "foreground_orchestrate": true,
        }),
    ));

    assert_eq!(response["id"], "bash-fast-orchestrated");
    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "completed", "response: {response:?}");
    assert!(response["output"].as_str().unwrap().contains("hi"));
    assert_eq!(response["exit_code"], 0);
    assert!(response["task_id"].is_string());

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_non_zero_exit_appends_exit_code_marker() {
    let mut aft = spawn_with_wait(SHORT_WAIT_MS);

    let response = aft.send(&bash_request(
        "bash-nonzero-orchestrated",
        json!({
            "command": "sh -c 'exit 3'",
            "foreground_orchestrate": true,
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["exit_code"], 3);
    assert!(response["output"]
        .as_str()
        .unwrap()
        .contains("[exit code: 3]"));

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_foreground_promotes_after_wait_window() {
    let mut aft = spawn_with_wait(SHORT_WAIT_MS);
    let dir = tempfile::tempdir().unwrap();
    configure_bash_background(&mut aft, &dir, 600);

    let started = Instant::now();
    let response = aft.send(&bash_request(
        "bash-promote-orchestrated",
        json!({
            "command": "sleep 5",
            "foreground_orchestrate": true,
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "running", "response: {response:?}");
    let task_id = response["task_id"].as_str().expect("task_id");
    let output = response["output"].as_str().unwrap();
    assert!(output.contains(&format!("promoted to background: {task_id}")));
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "promotion response took too long: {response:?}"
    );

    let status = aft.send(
        &json!({
            "id": "bash-promote-status",
            "method": "bash_status",
            "params": { "task_id": task_id },
        })
        .to_string(),
    );
    assert_eq!(status["success"], true, "status: {status:?}");
    assert_eq!(status["task_id"], task_id);

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_block_to_completion_does_not_promote() {
    let mut aft = spawn_with_wait("300");
    let started = Instant::now();

    let response = aft.send(&bash_request(
        "bash-block-orchestrated",
        json!({
            "command": "sleep 1",
            "timeout": 3_000,
            "foreground_orchestrate": true,
            "block_to_completion": true,
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "completed", "response: {response:?}");
    assert!(!response["output"]
        .as_str()
        .unwrap()
        .contains("promoted to background"));
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "block_to_completion returned before the command could finish: {response:?}"
    );

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_wait_true_does_not_promote() {
    let mut aft = spawn_with_wait("100");
    let started = Instant::now();

    let response = aft.send(&bash_request(
        "bash-wait-orchestrated",
        json!({
            "command": "printf 'wait-early\\n'; sleep 1; printf 'wait-late\\n'",
            "timeout": 3_000,
            "foreground_orchestrate": true,
            "wait": true,
            "compressed": false,
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "completed", "response: {response:?}");
    assert_eq!(
        response["output_preview"].as_str().unwrap(),
        "wait-early\nwait-late\n"
    );
    assert!(response["output"].as_str().unwrap().contains("wait-early"));
    assert!(response["output"].as_str().unwrap().contains("wait-late"));
    assert!(!response["output"]
        .as_str()
        .unwrap()
        .contains("promoted to background"));
    assert!(
        started.elapsed() >= Duration::from_millis(800),
        "wait:true returned before the command could finish: {response:?}"
    );

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_wait_true_honors_short_timeout() {
    let mut aft = spawn_with_wait("5000");

    let response = aft.send(&bash_request(
        "bash-wait-timeout-orchestrated",
        json!({
            "command": "sleep 2; printf 'too-late\\n'",
            "timeout": 200,
            "foreground_orchestrate": true,
            "wait": true,
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "timed_out", "response: {response:?}");
    assert_eq!(response["timed_out"], true, "response: {response:?}");
    assert!(response["output"]
        .as_str()
        .unwrap()
        .contains("[command timed out]"));
    assert!(!response["output"]
        .as_str()
        .unwrap()
        .contains("promoted to background"));

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_wait_rejects_background_and_pty() {
    let mut aft = spawn_with_wait(SHORT_WAIT_MS);

    let background = aft.send(&bash_request(
        "bash-wait-background-reject",
        json!({
            "command": "echo should-not-run",
            "foreground_orchestrate": true,
            "wait": true,
            "background": true,
        }),
    ));
    assert_eq!(background["success"], false, "response: {background:?}");
    assert_eq!(background["code"], "invalid_request");
    assert!(background["message"]
        .as_str()
        .unwrap()
        .contains("wait:true cannot be used with background:true"));

    let pty = aft.send(&bash_request(
        "bash-wait-pty-reject",
        json!({
            "command": "echo should-not-run",
            "foreground_orchestrate": true,
            "wait": true,
            "pty": true,
        }),
    ));
    assert_eq!(pty["success"], false, "response: {pty:?}");
    assert_eq!(pty["code"], "invalid_request");
    assert!(pty["message"]
        .as_str()
        .unwrap()
        .contains("wait:true cannot be used with pty:true"));

    assert!(aft.shutdown().success());
}

#[test]
fn orchestrated_background_returns_formatted_launch_immediately() {
    let mut aft = spawn_with_wait(SHORT_WAIT_MS);
    let dir = tempfile::tempdir().unwrap();
    configure_bash_background(&mut aft, &dir, 600);

    let response = aft.send(&bash_request(
        "bash-bg-orchestrated",
        json!({
            "command": "sleep 1",
            "foreground_orchestrate": true,
            "background": true,
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "running");
    assert!(response["task_id"].is_string());
    assert!(response["output"]
        .as_str()
        .unwrap()
        .starts_with("Background task started: "));

    assert!(aft.shutdown().success());
}

#[test]
fn bash_gate_off_still_returns_spawn_response() {
    let mut aft = spawn_with_wait(SHORT_WAIT_MS);

    let response = aft.send(&bash_request(
        "bash-gate-off",
        json!({
            "command": "echo gate-off",
        }),
    ));

    assert_eq!(response["success"], true, "response: {response:?}");
    assert_eq!(response["status"], "running");
    assert_eq!(response["mode"], "pipes");
    assert!(response["task_id"].is_string());
    assert!(
        response.get("output").is_none(),
        "spawn response leaked output: {response:?}"
    );
    let keys: BTreeSet<_> = response
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from(["id", "mode", "status", "success", "task_id"])
    );

    assert!(aft.shutdown().success());
}

#[test]
fn pending_orchestrated_bash_does_not_starve_push_frames() {
    let mut aft = spawn_with_wait("5000");
    let dir = tempfile::tempdir().unwrap();

    aft.send_silent(&bash_request(
        "bash-drain-orchestrated",
        json!({
            "command": "sleep 2",
            "foreground_orchestrate": true,
            "block_to_completion": true,
        }),
    ));

    let configure = aft.send(
        &json!({
            "id": "cfg-while-bash-pending",
            "command": "configure",
            "harness": "opencode",
            "project_root": dir.path(),
            "storage_dir": dir.path().join("storage"),
        })
        .to_string(),
    );
    assert_eq!(
        configure["success"], true,
        "configure failed: {configure:?}"
    );

    // The invariant: the pending deferred bash response must not starve push
    // frames — configure_warnings has to arrive BEFORE that deferred response.
    // Under host load the 2s sleep can finish while configure is still running,
    // so the task's own completion push may legitimately land first; pushes
    // overtaking pushes is not starvation. Tolerate the task's push frames and
    // fail only if the deferred RESPONSE (an "id" frame) beats the warnings.
    let push = loop {
        let frame = aft
            .try_read_next_timeout(Duration::from_secs(12))
            .expect("configure warning push before deferred bash response");
        if frame.get("type").is_some() {
            if frame["type"] == "configure_warnings" {
                break frame;
            }
            continue;
        }
        panic!("deferred response overtook the configure push (starvation): {frame:?}");
    };
    assert_eq!(
        push["type"], "configure_warnings",
        "unexpected frame: {push:?}"
    );

    let bash_response = loop {
        let value = aft
            .try_read_next_timeout(Duration::from_secs(12))
            .expect("deferred bash response after command completion");
        if value["id"] == "bash-drain-orchestrated" {
            break value;
        }
        assert!(
            value.get("type").is_some(),
            "unexpected non-push frame before deferred bash response: {value:?}"
        );
    };
    assert_eq!(bash_response["id"], "bash-drain-orchestrated");
    assert_eq!(
        bash_response["status"], "completed",
        "response: {bash_response:?}"
    );

    assert!(aft.shutdown().success());
}

#[test]
fn abort_inflight_kills_wait_registered_foreground_and_settles_deferred_response() {
    let mut aft = spawn_with_wait("5000");
    let dir = tempfile::tempdir().unwrap();
    configure_bash_background(&mut aft, &dir, 5_000);
    let pid_file = dir.path().join("foreground.pid");
    let command = format!("echo $$ > '{}'; sleep 30", pid_file.display());
    let session_id = "abort-session";

    aft.send_silent(
        &json!({
            "id": "bash-abort-foreground",
            "session_id": session_id,
            "method": "bash",
            "params": {
                "command": command,
                "foreground_orchestrate": true,
                "wait": true,
                "compressed": false,
            },
        })
        .to_string(),
    );

    let started = Instant::now();
    while !pid_file.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "foreground task did not start"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    // The body deliberately names another session. Raw command callers cannot
    // retarget this cancellation because the server uses the bound session.
    aft.send_silent(
        &json!({
            "id": "abort-inflight",
            "session_id": session_id,
            "command": "bash_abort_inflight",
            "params": { "session_id": "wrong-session" },
        })
        .to_string(),
    );

    let mut abort_response = None;
    let mut bash_response = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while (abort_response.is_none() || bash_response.is_none()) && Instant::now() < deadline {
        let Some(frame) = aft.try_read_next_timeout(Duration::from_millis(250)) else {
            continue;
        };
        if frame.get("type").is_some() {
            continue;
        }
        match frame["id"].as_str() {
            Some("abort-inflight") => abort_response = Some(frame),
            Some("bash-abort-foreground") => bash_response = Some(frame),
            _ => {}
        }
    }

    let abort_response = abort_response.expect("abort response");
    assert_eq!(
        abort_response["success"], true,
        "response: {abort_response:?}"
    );
    assert_eq!(abort_response["killed"], 1, "response: {abort_response:?}");
    let bash_response = bash_response.expect("deferred bash response");
    assert_eq!(
        bash_response["success"], true,
        "response: {bash_response:?}"
    );
    assert_eq!(
        bash_response["status"], "killed",
        "response: {bash_response:?}"
    );
    let task_id = bash_response["task_id"].as_str().expect("task id");

    let status = aft.send(
        &json!({
            "id": "abort-status",
            "session_id": session_id,
            "command": "bash_status",
            "params": { "task_id": task_id },
        })
        .to_string(),
    );
    assert_eq!(status["success"], true, "status: {status:?}");
    assert_eq!(status["status"], "killed", "status: {status:?}");
    assert_eq!(
        status["status_reason"], "call_aborted",
        "status: {status:?}"
    );

    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .expect("foreground pid");
    let started = Instant::now();
    loop {
        let ps = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let alive = ps.status.success() && !String::from_utf8_lossy(&ps.stdout).contains('Z');
        if !alive {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "pid {pid} survived abort"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(aft.shutdown().success());
}
