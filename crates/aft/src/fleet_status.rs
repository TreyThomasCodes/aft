//! Pull-on-render consumer for the fleet status-holder plane.
//!
//! The daemon owns status retention and composition. AFT only publishes its
//! project-scoped segment, asks for the three scopes relevant to the current
//! render, and caches the producer's composed response briefly.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Weak};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

const STATUS_CADENCE: Duration = Duration::from_millis(2_500);
const STATUS_PULL_BUDGET: Duration = Duration::from_millis(50);
const STATUS_PUBLISH_TTL_MS: u64 = 7_500;
const STATUS_MODULE: &str = "aft";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct StatusLineSegment {
    pub(crate) module: String,
    pub(crate) scope: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct StatusLineSnapshot {
    pub(crate) line: String,
    pub(crate) segments: Vec<StatusLineSegment>,
}

impl StatusLineSnapshot {
    fn parse(body: &[u8]) -> Option<Self> {
        serde_json::from_slice(body).ok()
    }

    fn has_foreign_segment(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.module != STATUS_MODULE && !segment.text.is_empty())
    }

    pub(crate) fn compose_fleet_bar(&self) -> Option<String> {
        self.has_foreign_segment()
            .then(|| format!("[CK: {}]", self.line))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct StatusPublishAck {
    pub(crate) epoch: u64,
    pub(crate) accepted_revision: u64,
}

impl StatusPublishAck {
    fn parse(body: &[u8]) -> Option<Self> {
        serde_json::from_slice(body).ok()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PublishFence {
    epoch: u64,
    accepted_revision: u64,
}

impl PublishFence {
    fn observe(&mut self, ack: StatusPublishAck) {
        if ack.epoch > self.epoch
            || (ack.epoch == self.epoch && ack.accepted_revision >= self.accepted_revision)
        {
            self.epoch = ack.epoch;
            self.accepted_revision = ack.accepted_revision;
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct RenderKey {
    project_scope: String,
    session_scope: String,
}

impl RenderKey {
    fn new(project_root: &Path, session_id: &str) -> Self {
        Self {
            project_scope: format!("project:{}", project_root.to_string_lossy()),
            session_scope: format!("session:{session_id}"),
        }
    }

    fn scopes(&self) -> [&str; 3] {
        ["global", &self.project_scope, &self.session_scope]
    }
}

#[derive(Clone, Debug)]
struct CachedLine {
    observed_at: Instant,
    snapshot: StatusLineSnapshot,
}

#[derive(Default)]
struct ClientState {
    cache: HashMap<RenderKey, CachedLine>,
    pulls_in_flight: HashSet<RenderKey>,
    last_publish_at: HashMap<String, Instant>,
    publish_fence: PublishFence,
}

struct FleetStatusInner {
    wire_tx: mpsc::Sender<StatusWireRequest>,
    state: parking_lot::Mutex<ClientState>,
    next_revision: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct FleetStatusClient {
    inner: Arc<FleetStatusInner>,
}

impl FleetStatusClient {
    pub(crate) fn channel(capacity: usize) -> (Self, mpsc::Receiver<StatusWireRequest>) {
        let (wire_tx, wire_rx) = mpsc::channel(capacity);
        (
            Self {
                inner: Arc::new(FleetStatusInner {
                    wire_tx,
                    state: parking_lot::Mutex::new(ClientState::default()),
                    next_revision: AtomicU64::new(1),
                }),
            },
            wire_rx,
        )
    }

    /// Return the producer-composed fleet bar when a fresh pull observes a
    /// non-empty foreign segment. Contention, transport errors, and the bounded
    /// wait all return `None`, leaving the existing solo renderer untouched.
    pub(crate) fn render(
        &self,
        project_root: &Path,
        session_id: &str,
        aft_text: &str,
    ) -> Option<String> {
        let key = RenderKey::new(project_root, session_id);
        let now = Instant::now();
        let (publish, revision) = {
            let mut state = self.inner.state.try_lock()?;
            if let Some(cached) = state.cache.get(&key) {
                if now.saturating_duration_since(cached.observed_at) <= STATUS_CADENCE {
                    return cached.snapshot.compose_fleet_bar();
                }
            }
            if !state.pulls_in_flight.insert(key.clone()) {
                return None;
            }

            let publish = state
                .last_publish_at
                .get(&key.project_scope)
                .is_none_or(|last| now.saturating_duration_since(*last) >= STATUS_CADENCE);
            let revision = publish
                .then(|| self.inner.next_revision.fetch_add(1, Ordering::Relaxed))
                .unwrap_or(0);
            if publish {
                state.last_publish_at.insert(key.project_scope.clone(), now);
            }
            (publish, revision)
        };

        if publish {
            let request = StatusWireRequest::publish(
                Arc::downgrade(&self.inner),
                &key.project_scope,
                aft_text,
                revision,
            );
            if self.inner.wire_tx.try_send(request).is_err() {
                self.inner
                    .state
                    .lock()
                    .last_publish_at
                    .remove(&key.project_scope);
            }
        }

        let (reply_tx, reply_rx) = std_mpsc::sync_channel(1);
        let request = StatusWireRequest::line(key.scopes(), reply_tx);
        if self.inner.wire_tx.try_send(request).is_err() {
            self.clear_in_flight(&key);
            return None;
        }

        let snapshot = match reply_rx.recv_timeout(STATUS_PULL_BUDGET) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) | Err(_) => {
                self.clear_in_flight(&key);
                return None;
            }
        };

        let rendered = snapshot.compose_fleet_bar();
        let mut state = self.inner.state.lock();
        state.pulls_in_flight.remove(&key);
        state.cache.insert(
            key,
            CachedLine {
                observed_at: Instant::now(),
                snapshot,
            },
        );
        rendered
    }

    fn clear_in_flight(&self, key: &RenderKey) {
        self.inner.state.lock().pulls_in_flight.remove(key);
    }

    #[cfg(test)]
    fn publish_fence(&self) -> PublishFence {
        self.inner.state.lock().publish_fence
    }
}

enum StatusWireCompletion {
    Line(std_mpsc::SyncSender<Option<StatusLineSnapshot>>),
    Publish(Weak<FleetStatusInner>),
}

pub(crate) struct StatusWireRequest {
    body: Value,
    completion: StatusWireCompletion,
}

impl StatusWireRequest {
    fn line(scopes: [&str; 3], reply: std_mpsc::SyncSender<Option<StatusLineSnapshot>>) -> Self {
        Self {
            body: json!({
                "op": "status.line",
                "scopes": scopes,
            }),
            completion: StatusWireCompletion::Line(reply),
        }
    }

    fn publish(client: Weak<FleetStatusInner>, scope: &str, text: &str, revision: u64) -> Self {
        Self {
            body: json!({
                "op": "status.publish",
                "module": STATUS_MODULE,
                "scope": scope,
                "text": text,
                "ttl_ms": STATUS_PUBLISH_TTL_MS,
                "revision": revision,
            }),
            completion: StatusWireCompletion::Publish(client),
        }
    }

    pub(crate) fn body(&self) -> &Value {
        &self.body
    }

    pub(crate) fn complete_response(self, body: &[u8]) {
        match self.completion {
            StatusWireCompletion::Line(reply) => {
                let _ = reply.send(StatusLineSnapshot::parse(body));
            }
            StatusWireCompletion::Publish(client) => {
                let Some(ack) = StatusPublishAck::parse(body) else {
                    return;
                };
                let Some(client) = client.upgrade() else {
                    return;
                };
                client.state.lock().publish_fence.observe(ack);
            }
        }
    }

    pub(crate) fn complete_unavailable(self) {
        if let StatusWireCompletion::Line(reply) = self.completion {
            let _ = reply.send(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use std::thread;

    const FIXTURES: &str = include_str!("../../../.cortexkit/status-line-fixtures-2026-08-14.json");

    fn fixtures() -> Map<String, Value> {
        serde_json::from_str::<Value>(FIXTURES)
            .expect("producer fixture JSON")
            .as_object()
            .expect("producer fixture object")
            .clone()
    }

    fn parse_line(value: &Value) -> StatusLineSnapshot {
        StatusLineSnapshot::parse(&serde_json::to_vec(value).expect("fixture bytes"))
            .expect("status.line fixture")
    }

    fn parse_ack(value: &Value) -> StatusPublishAck {
        StatusPublishAck::parse(&serde_json::to_vec(value).expect("fixture bytes"))
            .expect("status.publish fixture")
    }

    #[test]
    fn all_producer_fixtures_exercise_parse_compose_and_fencing_paths() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), 18, "fixture probe entry count changed");
        let mut exercised = HashSet::new();

        let ack_names = [
            "publish_aft",
            "supersede_r3",
            "supersede_r2_late",
            "ttl_publish",
            "quiet_publish_empty_text",
            "fat1",
            "fat2",
            "epoch_bump_republish_r1_new_conn",
            "publish_aft_project",
        ];
        let mut fence = PublishFence::default();
        for name in ack_names {
            fence.observe(parse_ack(&fixtures[name]));
            exercised.insert(name);
        }
        assert_eq!(
            fence,
            PublishFence {
                epoch: 4,
                accepted_revision: 1
            }
        );

        let line_names = [
            "line_foreign_present",
            "supersede_line_after_r3",
            "supersede_line_after_late_r2",
            "ttl_line_before",
            "ttl_line_after_expiry",
            "quiet_line",
            "line_cap_overflow",
            "epoch_bump_line_after",
            "line_aft_solo_scope",
        ];
        let mut lines = HashMap::new();
        for name in line_names {
            let line = parse_line(&fixtures[name]);
            let _ = line.compose_fleet_bar();
            lines.insert(name, line);
            exercised.insert(name);
        }

        assert_eq!(
            lines["line_foreign_present"].compose_fleet_bar().as_deref(),
            Some("[CK: aft E0 W0 idx fresh | pfc 1567 events (777 gaps)]")
        );
        assert_eq!(
            lines["supersede_line_after_r3"],
            lines["supersede_line_after_late_r2"]
        );
        assert!(lines["supersede_line_after_late_r2"]
            .line
            .contains("revision three"));
        assert_eq!(lines["ttl_line_before"].segments.len(), 4);
        assert_eq!(lines["ttl_line_after_expiry"].segments.len(), 3);
        assert_eq!(
            lines["quiet_line"].segments.last().map(|s| s.text.as_str()),
            Some("")
        );
        assert!(!lines["quiet_line"].line.contains("ttlfx"));

        let capped = &lines["line_cap_overflow"];
        assert!(capped.line.chars().count() <= 200);
        assert_eq!(
            capped.segments.len(),
            6,
            "the cap must not truncate segments[]"
        );
        assert!(
            !capped.line.contains("fat2"),
            "whole tail segments are dropped"
        );
        assert_eq!(
            capped.compose_fleet_bar(),
            Some(format!("[CK: {}]", capped.line))
        );
        assert_eq!(
            lines["epoch_bump_line_after"].segments[3].text,
            "fresh epoch revision one"
        );
        assert_eq!(lines["line_aft_solo_scope"].compose_fleet_bar(), None);

        assert_eq!(exercised.len(), fixtures.len());
        assert!(fixtures
            .keys()
            .all(|name| exercised.contains(name.as_str())));
    }

    #[test]
    fn publication_fence_drops_regressions_but_new_epoch_wins() {
        let fixtures = fixtures();
        let mut fence = PublishFence::default();
        fence.observe(parse_ack(&fixtures["supersede_r3"]));
        fence.observe(StatusPublishAck {
            epoch: 3,
            accepted_revision: 2,
        });
        assert_eq!(
            fence,
            PublishFence {
                epoch: 3,
                accepted_revision: 3
            }
        );

        fence.observe(parse_ack(&fixtures["epoch_bump_republish_r1_new_conn"]));
        assert_eq!(
            fence,
            PublishFence {
                epoch: 4,
                accepted_revision: 1
            }
        );
    }

    #[test]
    fn timeout_falls_back_within_the_render_budget() {
        let (client, mut wire_rx) = FleetStatusClient::channel(4);
        let receiver = thread::spawn(move || {
            let publish = wire_rx.blocking_recv().expect("publish request");
            assert_eq!(publish.body()["op"], "status.publish");
            let line = wire_rx.blocking_recv().expect("line request");
            assert_eq!(line.body()["op"], "status.line");
            thread::sleep(Duration::from_millis(300));
            line.complete_unavailable();
        });

        let started = Instant::now();
        let rendered = client.render(
            Path::new("/tmp/project"),
            "session-1",
            "E0 W0 | D0 U0 C0 | T0",
        );
        let elapsed = started.elapsed();

        assert_eq!(rendered, None);
        assert!(elapsed >= STATUS_PULL_BUDGET);
        assert!(
            elapsed < Duration::from_millis(200),
            "render blocked for {elapsed:?}"
        );
        receiver.join().expect("receiver thread");
    }

    #[test]
    fn empty_local_status_publishes_alive_quiet() {
        let (client, mut wire_rx) = FleetStatusClient::channel(4);
        let solo_fixture = serde_json::to_vec(&fixtures()["line_aft_solo_scope"])
            .expect("solo status fixture bytes");
        let receiver = thread::spawn(move || {
            let publish = wire_rx.blocking_recv().expect("publish request");
            assert_eq!(publish.body()["op"], "status.publish");
            assert_eq!(publish.body()["text"], "");
            assert_eq!(publish.body()["ttl_ms"], STATUS_PUBLISH_TTL_MS);

            let line = wire_rx.blocking_recv().expect("line request");
            line.complete_response(&solo_fixture);
        });

        assert_eq!(
            client.render(Path::new("/tmp/project"), "session-1", ""),
            None
        );
        receiver.join().expect("receiver thread");
    }

    #[test]
    fn cache_serves_bursts_without_multiplying_pulls() {
        let (client, mut wire_rx) = FleetStatusClient::channel(4);
        let fixtures = fixtures();
        let publish_fixture =
            serde_json::to_vec(&fixtures["publish_aft"]).expect("publish fixture bytes");
        let line_fixture = serde_json::to_vec(&fixtures["line_foreign_present"])
            .expect("status.line fixture bytes");
        let receiver = thread::spawn(move || {
            let publish = wire_rx.blocking_recv().expect("publish request");
            assert_eq!(publish.body()["revision"], 1);
            publish.complete_response(&publish_fixture);

            let line = wire_rx.blocking_recv().expect("line request");
            line.complete_response(&line_fixture);
            thread::sleep(Duration::from_millis(20));
            assert!(
                wire_rx.try_recv().is_err(),
                "fresh cache issued another request"
            );
        });

        let first = client.render(Path::new("/tmp/project"), "session-1", "local");
        let second = client.render(Path::new("/tmp/project"), "session-1", "local");
        assert_eq!(
            first.as_deref(),
            Some("[CK: aft E0 W0 idx fresh | pfc 1567 events (777 gaps)]")
        );
        assert_eq!(second, first);
        receiver.join().expect("receiver thread");
        assert_eq!(
            client.publish_fence(),
            PublishFence {
                epoch: 3,
                accepted_revision: 1
            }
        );
    }
}
