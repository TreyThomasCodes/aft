use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::helpers::{user_config, AftProcess};

const SEARCH_QUERY: &str = "where does standalone deferred cancellation stop remote embedding";

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set embedding read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("read embedding request");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if bytes.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8(bytes).expect("embedding request is utf-8")
}

fn write_embedding_response(stream: &mut TcpStream) {
    let body = r#"{"data":[{"embedding":[0.1,0.2,0.3],"index":0}]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn start_embedding_server() -> (String, mpsc::Receiver<()>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind embedding server");
    let address = listener.local_addr().expect("embedding server address");
    let (query_started_tx, query_started_rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let (mut stream, _) = listener.accept().expect("accept embedding request");
            let request = read_http_request(&mut stream);
            if request.contains(SEARCH_QUERY) {
                query_started_tx
                    .send(())
                    .expect("signal query embedding start");
                thread::sleep(Duration::from_secs(2));
                write_embedding_response(&mut stream);
                return;
            }
            write_embedding_response(&mut stream);
        }
        panic!("semantic query embedding did not reach the mock server");
    });
    (format!("http://{address}"), query_started_rx, handle)
}

#[test]
fn standalone_ndjson_status_and_cancel_proceed_while_search_is_pending() {
    let project = tempfile::tempdir().expect("create standalone project");
    let storage = tempfile::tempdir().expect("create standalone storage");
    let (base_url, query_started, embedding_server) = start_embedding_server();
    let mut aft = AftProcess::spawn();

    let configure = aft.send(
        &serde_json::to_string(&json!({
            "id": "configure-search",
            "command": "configure",
            "harness": "opencode",
            "project_root": project.path().display().to_string(),
            "storage_dir": storage.path().display().to_string(),
            "config": user_config(json!({
                "semantic_search": true,
                "semantic": {
                    "backend": "openai_compatible",
                    "model": "test-embedding",
                    "base_url": base_url,
                    "query_timeout_ms": 3000
                }
            }))
        }))
        .expect("serialize configure request"),
    );
    assert_eq!(
        configure["success"], true,
        "configure failed: {configure:?}"
    );

    aft.send_silent(
        &serde_json::to_string(&json!({
            "id": "slow-search",
            "command": "semantic_search",
            "query": SEARCH_QUERY
        }))
        .expect("serialize search request"),
    );
    query_started
        .recv_timeout(Duration::from_secs(10))
        .expect("standalone query embedding starts");

    let status_started_at = Instant::now();
    let status = aft.send_with_timeout(
        &serde_json::to_string(&json!({"id": "sibling-status", "command": "status"}))
            .expect("serialize status request"),
        Duration::from_millis(500),
    );
    let status_latency = status_started_at.elapsed();
    assert_eq!(
        status["id"], "sibling-status",
        "search blocked status: {status:?}"
    );
    assert_eq!(status["success"], true);
    assert!(
        status_latency < Duration::from_millis(500),
        "sibling status waited behind query embedding for {status_latency:?}"
    );

    let cancel = aft.send_with_timeout(
        &serde_json::to_string(&json!({
            "id": "cancel-search",
            "command": "cancel_request",
            "params": {"id": "slow-search"}
        }))
        .expect("serialize cancel request"),
        Duration::from_millis(500),
    );
    assert_eq!(cancel["success"], true, "cancel command failed: {cancel:?}");
    assert_eq!(cancel["cancelled"], true);

    let deadline = Instant::now() + Duration::from_millis(500);
    let cancelled = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(frame) = aft.try_read_next_timeout(remaining.min(Duration::from_millis(100)))
        else {
            assert!(
                Instant::now() < deadline,
                "cancelled search should resolve before the remote response"
            );
            continue;
        };
        if frame["id"] == "slow-search" {
            break frame;
        }
        assert!(
            Instant::now() < deadline,
            "cancelled search response timed out"
        );
    };
    assert_eq!(cancelled["success"], false);
    assert_eq!(cancelled["code"], "request_cancelled");

    let status = aft.shutdown();
    assert!(status.success());
    embedding_server.join().expect("embedding server joins");
}
