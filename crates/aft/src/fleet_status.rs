//! Publisher for AFT's segment on the fleet status-holder plane.
//!
//! The holder owns retention and composed rendering. While its route is live,
//! AFT publishes its project-scoped segment and leaves status-bar attachment to
//! the holder's host plugin.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

const STATUS_CADENCE: Duration = Duration::from_millis(2_500);
const STATUS_PUBLISH_TTL_MS: u64 = 7_500;
const STATUS_MODULE: &str = "aft";

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct StatusLineSegment {
    pub(crate) module: String,
    pub(crate) scope: String,
    pub(crate) text: String,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct StatusLineSnapshot {
    pub(crate) line: String,
    pub(crate) segments: Vec<StatusLineSegment>,
}

#[cfg(test)]
impl StatusLineSnapshot {
    fn parse(body: &[u8]) -> Option<Self> {
        serde_json::from_slice(body).ok()
    }

    fn has_foreign_segment(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.module != STATUS_MODULE && !segment.text.is_empty())
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

#[derive(Default)]
struct ClientState {
    last_publish_at: HashMap<String, Instant>,
    publish_fence: PublishFence,
}

struct FleetStatusInner {
    wire_tx: Option<mpsc::Sender<StatusWireRequest>>,
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
                    wire_tx: Some(wire_tx),
                    state: parking_lot::Mutex::new(ClientState::default()),
                    next_revision: AtomicU64::new(1),
                }),
            },
            wire_rx,
        )
    }

    pub(crate) fn dormant() -> Self {
        Self {
            inner: Arc::new(FleetStatusInner {
                wire_tx: None,
                state: parking_lot::Mutex::new(ClientState::default()),
                next_revision: AtomicU64::new(1),
            }),
        }
    }

    /// Publish AFT's segment when the holder route is live. The return value is
    /// ownership, not delivery: `true` means the holder's host plugin owns the
    /// response bar even when cadence, contention, or backpressure skips a send.
    pub(crate) fn publish(&self, project_root: &Path, aft_text: &str) -> bool {
        let Some(wire_tx) = self.inner.wire_tx.as_ref() else {
            return false;
        };
        let scope = format!("project:{}", project_root.to_string_lossy());
        let now = Instant::now();
        let revision = {
            let Some(mut state) = self.inner.state.try_lock() else {
                return true;
            };
            if state
                .last_publish_at
                .get(&scope)
                .is_some_and(|last| now.saturating_duration_since(*last) < STATUS_CADENCE)
            {
                return true;
            }
            state.last_publish_at.insert(scope.clone(), now);
            self.inner.next_revision.fetch_add(1, Ordering::Relaxed)
        };

        let request =
            StatusWireRequest::publish(Arc::downgrade(&self.inner), &scope, aft_text, revision);
        if wire_tx.try_send(request).is_err() {
            self.inner.state.lock().last_publish_at.remove(&scope);
        }
        true
    }

    #[cfg(test)]
    fn publish_fence(&self) -> PublishFence {
        self.inner.state.lock().publish_fence
    }
}

pub(crate) struct StatusWireRequest {
    body: Value,
    client: Weak<FleetStatusInner>,
}

impl StatusWireRequest {
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
            client,
        }
    }

    pub(crate) fn body(&self) -> &Value {
        &self.body
    }

    pub(crate) fn complete_response(self, body: &[u8]) {
        let Some(ack) = StatusPublishAck::parse(body) else {
            return;
        };
        let Some(client) = self.client.upgrade() else {
            return;
        };
        client.state.lock().publish_fence.observe(ack);
    }

    pub(crate) fn complete_unavailable(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use std::collections::HashSet;

    const FIXTURES: &str = include_str!("../../../.cortexkit/status-line-fixtures-2026-08-14.json");
    const PUBLISH_ACK_FIXTURES: [&str; 9] = [
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
    const LINE_REPLY_FIXTURES: [&str; 9] = [
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
    fn publish_ack_fixtures_drive_runtime_fencing() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), 18, "fixture probe entry count changed");
        let mut fence = PublishFence::default();
        for name in PUBLISH_ACK_FIXTURES {
            fence.observe(parse_ack(&fixtures[name]));
        }
        assert_eq!(
            fence,
            PublishFence {
                epoch: 4,
                accepted_revision: 1
            }
        );
    }

    #[test]
    fn line_reply_fixtures_remain_wire_shape_documentation() {
        let fixtures = fixtures();
        let documented = PUBLISH_ACK_FIXTURES
            .into_iter()
            .chain(LINE_REPLY_FIXTURES)
            .collect::<HashSet<_>>();
        assert_eq!(documented.len(), fixtures.len());
        assert!(fixtures
            .keys()
            .all(|name| documented.contains(name.as_str())));

        let lines = LINE_REPLY_FIXTURES
            .into_iter()
            .map(|name| (name, parse_line(&fixtures[name])))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            lines["line_foreign_present"].line,
            "aft E0 W0 idx fresh | pfc 1567 events (777 gaps)"
        );
        assert!(lines["line_foreign_present"].has_foreign_segment());
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
            lines["quiet_line"]
                .segments
                .last()
                .map(|segment| segment.text.as_str()),
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
            lines["epoch_bump_line_after"].segments[3].text,
            "fresh epoch revision one"
        );
        assert!(!lines["line_aft_solo_scope"].has_foreign_segment());
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
    fn dormant_client_falls_back_without_publishing() {
        let client = FleetStatusClient::dormant();

        assert!(!client.publish(Path::new("/tmp/project"), "E0 W0 | D0 U0 C0 | T0"));
        let state = client.inner.state.lock();
        assert!(state.last_publish_at.is_empty());
        assert_eq!(client.inner.next_revision.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn empty_local_status_publishes_alive_quiet() {
        let (client, mut wire_rx) = FleetStatusClient::channel(4);

        assert!(client.publish(Path::new("/tmp/project"), ""));
        let publish = wire_rx.try_recv().expect("publish request");
        assert_eq!(publish.body()["op"], "status.publish");
        assert_eq!(publish.body()["text"], "");
        assert_eq!(publish.body()["ttl_ms"], STATUS_PUBLISH_TTL_MS);
        assert!(
            wire_rx.try_recv().is_err(),
            "publisher emitted a pull request"
        );
    }

    #[test]
    fn cadence_suppresses_duplicate_publishes_and_ack_updates_fence() {
        let (client, mut wire_rx) = FleetStatusClient::channel(4);
        let fixtures = fixtures();
        let publish_fixture =
            serde_json::to_vec(&fixtures["publish_aft"]).expect("publish fixture bytes");

        assert!(client.publish(Path::new("/tmp/project"), "local"));
        let publish = wire_rx.try_recv().expect("first publish request");
        assert_eq!(publish.body()["revision"], 1);
        publish.complete_response(&publish_fixture);

        assert!(client.publish(Path::new("/tmp/project"), "local"));
        assert!(
            wire_rx.try_recv().is_err(),
            "publish cadence issued another request"
        );
        assert_eq!(
            client.publish_fence(),
            PublishFence {
                epoch: 3,
                accepted_revision: 1
            }
        );
    }
}
