//! Session-keyed hashline bindings: registration, effective mode, and lifetime.
//!
//! Registration computes edit-slot eligibility independently of schema selection,
//! derives `effective = configured_enabled AND edit_slot_survives`, and installs
//! the binding for `(canonical project root, session id)`. Request handlers capture
//! a binding guard for the duration of the call so concurrent sessions under one
//! root never share tags, stores, or schemas. Effective-value changes drain
//! in-flight guards before clearing stores.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};

use crate::hashline::apply::RegisterStore;
use crate::hashline::snapshot::SnapshotStore;

/// Stable identity for one session under one project root.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionKey {
    pub root: PathBuf,
    pub session_id: String,
}

impl SessionKey {
    pub fn new(root: impl Into<PathBuf>, session_id: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            session_id: session_id.into(),
        }
    }
}

/// Configure-channel warning emitted when hashline is configured on but `edit`
/// did not survive final surface selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DowngradeWarning {
    pub code: &'static str,
    pub reason: &'static str,
}

impl DowngradeWarning {
    pub const EDIT_NOT_REGISTERED: Self = Self {
        code: "hashline_downgraded",
        reason: "edit_not_registered",
    };

    /// JSON object for the configure-warnings channel.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "reason": self.reason,
        })
    }
}

/// Inputs the host supplies when registering (or re-registering) a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistrationRequest {
    /// Resolved `hashline.enabled` from Rust config_resolve.
    pub configured_enabled: bool,
    /// Host-computed flag: `edit` survived final surface selection, pruning,
    /// hoisting, and `disabled_tools`. Sessions without a host pruning layer
    /// (MCP, daemon-direct) default this to `true`.
    pub edit_slot_survives: bool,
}

impl RegistrationRequest {
    pub const fn effective(self) -> bool {
        self.configured_enabled && self.edit_slot_survives
    }

    pub const fn should_downgrade(self) -> bool {
        self.configured_enabled && !self.edit_slot_survives
    }
}

/// Outcome of a completed registration attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrationOutcome {
    pub configured_enabled: bool,
    pub edit_slot_survives: bool,
    pub effective: bool,
    /// Present when configured on but edit was not registered.
    pub downgrade: Option<DowngradeWarning>,
    /// True when stores were cleared because the effective value changed.
    pub stores_cleared: bool,
    /// True when same-effective re-registration preserved snapshot/register state.
    pub stores_preserved: bool,
}

/// Session-owned hashline state installed atomically at registration.
#[derive(Debug)]
pub struct HashlineBinding {
    key: SessionKey,
    configured_enabled: bool,
    edit_slot_survives: bool,
    effective: bool,
    snapshots: SnapshotStore,
    registers: RegisterStore,
    /// In-flight request guards holding this binding.
    in_flight: usize,
}

impl HashlineBinding {
    fn new(key: SessionKey, request: RegistrationRequest) -> Self {
        Self {
            key,
            configured_enabled: request.configured_enabled,
            edit_slot_survives: request.edit_slot_survives,
            effective: request.effective(),
            snapshots: SnapshotStore::new(),
            registers: RegisterStore::new(),
            in_flight: 0,
        }
    }

    pub fn key(&self) -> &SessionKey {
        &self.key
    }

    pub fn configured_enabled(&self) -> bool {
        self.configured_enabled
    }

    pub fn edit_slot_survives(&self) -> bool {
        self.edit_slot_survives
    }

    pub fn effective(&self) -> bool {
        self.effective
    }

    pub fn snapshots(&self) -> &SnapshotStore {
        &self.snapshots
    }

    pub fn snapshots_mut(&mut self) -> &mut SnapshotStore {
        &mut self.snapshots
    }

    pub fn registers(&self) -> &RegisterStore {
        &self.registers
    }

    pub fn registers_mut(&mut self) -> &mut RegisterStore {
        &mut self.registers
    }

    /// Borrow both session stores for one atomic request pipeline.
    pub fn stores_mut(&mut self) -> (&mut SnapshotStore, &mut RegisterStore) {
        (&mut self.snapshots, &mut self.registers)
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    fn clear_stores(&mut self) {
        self.snapshots.clear();
        *self.registers_mut() = RegisterStore::new();
    }
}

/// Shared handle to an installed binding. Capture one per request.
#[derive(Clone, Debug)]
pub struct BindingHandle {
    inner: Arc<Mutex<HashlineBinding>>,
}

impl BindingHandle {
    pub fn with_binding<R>(&self, f: impl FnOnce(&HashlineBinding) -> R) -> R {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        f(&guard)
    }

    pub fn with_binding_mut<R>(&self, f: impl FnOnce(&mut HashlineBinding) -> R) -> R {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut guard)
    }

    pub fn effective(&self) -> bool {
        self.with_binding(|b| b.effective())
    }

    pub fn session_key(&self) -> SessionKey {
        self.with_binding(|b| b.key().clone())
    }
}

/// RAII guard that keeps a binding alive for one request and participates in
/// the rebind drain refcount.
pub struct BindingGuard {
    handle: BindingHandle,
    registry: Weak<BindingRegistryInner>,
}

impl BindingGuard {
    pub fn handle(&self) -> &BindingHandle {
        &self.handle
    }

    pub fn effective(&self) -> bool {
        self.handle.effective()
    }

    pub fn with_binding<R>(&self, f: impl FnOnce(&HashlineBinding) -> R) -> R {
        self.handle.with_binding(f)
    }

    pub fn with_binding_mut<R>(&self, f: impl FnOnce(&mut HashlineBinding) -> R) -> R {
        self.handle.with_binding_mut(f)
    }
}

impl Drop for BindingGuard {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.release_guard(&self.handle);
        } else {
            // Registry gone: still decrement local refcount so tests that drop
            // the registry after guards do not leave a poisoned counter.
            let mut binding = self.handle.inner.lock().unwrap_or_else(|p| p.into_inner());
            binding.in_flight = binding.in_flight.saturating_sub(1);
        }
    }
}

struct BindingRegistryInner {
    state: Mutex<RegistryState>,
    drain: Condvar,
}

#[derive(Default)]
struct RegistryState {
    bindings: HashMap<SessionKey, Arc<Mutex<HashlineBinding>>>,
}

/// Process-wide (or test-local) registry of session hashline bindings.
pub struct BindingRegistry {
    inner: Arc<BindingRegistryInner>,
}

impl Default for BindingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BindingRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BindingRegistryInner {
                state: Mutex::new(RegistryState::default()),
                drain: Condvar::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RegistryState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Atomically install or replace the binding for one session.
    ///
    /// Same-effective re-registration preserves snapshot and register stores.
    /// Effective-value changes drain in-flight guards, clear stores, then install.
    /// Failed callers must not call this with a partial request — the install is
    /// all-or-nothing once invoked.
    pub fn register(
        &self,
        root: impl AsRef<Path>,
        session_id: impl Into<String>,
        request: RegistrationRequest,
    ) -> RegistrationOutcome {
        let key = SessionKey::new(root.as_ref().to_path_buf(), session_id.into());
        self.register_key(key, request, || {})
    }

    fn register_key(
        &self,
        key: SessionKey,
        request: RegistrationRequest,
        after_existing_read: impl FnOnce(),
    ) -> RegistrationOutcome {
        let effective = request.effective();
        let downgrade = request
            .should_downgrade()
            .then_some(DowngradeWarning::EDIT_NOT_REGISTERED);

        // Serialize the existing-value read, comparison, and binding update. A
        // guard may finish while this lock is held because guard release only
        // takes the binding lock and signals the drain condition variable.
        let mut state = self.lock();
        let existing = state.bindings.get(&key).cloned();
        let previous_effective = existing.as_ref().map(|binding| {
            binding
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .effective()
        });
        after_existing_read();

        let (stores_cleared, stores_preserved) = if let Some(existing) = existing {
            if previous_effective != Some(effective) {
                self.drain_in_flight(&existing);
                {
                    let mut binding = existing.lock().unwrap_or_else(|p| p.into_inner());
                    binding.configured_enabled = request.configured_enabled;
                    binding.edit_slot_survives = request.edit_slot_survives;
                    binding.effective = effective;
                    binding.clear_stores();
                }
                state.bindings.insert(key, existing);
                (true, false)
            } else {
                {
                    let mut binding = existing.lock().unwrap_or_else(|p| p.into_inner());
                    binding.configured_enabled = request.configured_enabled;
                    binding.edit_slot_survives = request.edit_slot_survives;
                    // effective unchanged; stores preserved.
                }
                state.bindings.insert(key, existing);
                (false, true)
            }
        } else {
            let binding = Arc::new(Mutex::new(HashlineBinding::new(key.clone(), request)));
            state.bindings.insert(key, binding);
            (false, false)
        };

        RegistrationOutcome {
            configured_enabled: request.configured_enabled,
            edit_slot_survives: request.edit_slot_survives,
            effective,
            downgrade,
            stores_cleared,
            stores_preserved,
        }
    }

    /// Capture the installed binding for one request. Unregistered sessions
    /// yield `None` and must behave as effective-off.
    pub fn capture(
        &self,
        root: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> Option<BindingGuard> {
        let key = SessionKey::new(root.as_ref().to_path_buf(), session_id.into());
        let handle = {
            let state = self.lock();
            let arc = state.bindings.get(&key)?.clone();
            {
                let mut binding = arc.lock().unwrap_or_else(|p| p.into_inner());
                binding.in_flight = binding.in_flight.saturating_add(1);
            }
            BindingHandle { inner: arc }
        };
        Some(BindingGuard {
            handle,
            registry: Arc::downgrade(&self.inner),
        })
    }

    /// Look up without incrementing the in-flight refcount (diagnostics only).
    pub fn peek(
        &self,
        root: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> Option<BindingHandle> {
        let key = SessionKey::new(root.as_ref().to_path_buf(), session_id.into());
        let state = self.lock();
        state
            .bindings
            .get(&key)
            .map(|arc| BindingHandle { inner: arc.clone() })
    }

    /// Remove one session binding (teardown / restart). In-flight guards drain first.
    pub fn teardown(&self, root: impl AsRef<Path>, session_id: impl Into<String>) -> bool {
        let key = SessionKey::new(root.as_ref().to_path_buf(), session_id.into());
        let existing = {
            let state = self.lock();
            state.bindings.get(&key).cloned()
        };
        if let Some(existing) = existing {
            self.drain_in_flight(&existing);
        }
        let mut state = self.lock();
        state.bindings.remove(&key).is_some()
    }

    /// Number of installed bindings (test/diagnostics).
    pub fn len(&self) -> usize {
        self.lock().bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn drain_in_flight(&self, binding: &Arc<Mutex<HashlineBinding>>) {
        let mut guard = binding.lock().unwrap_or_else(|p| p.into_inner());
        while guard.in_flight > 0 {
            guard = self
                .inner
                .drain
                .wait(guard)
                .unwrap_or_else(|p| p.into_inner());
        }
    }
}

impl BindingRegistryInner {
    fn release_guard(&self, handle: &BindingHandle) {
        {
            let mut binding = handle.inner.lock().unwrap_or_else(|p| p.into_inner());
            binding.in_flight = binding.in_flight.saturating_sub(1);
        }
        self.drain.notify_all();
    }
}

/// Effective mode for a request: unregistered sessions are always off.
pub fn effective_for_capture(guard: Option<&BindingGuard>) -> bool {
    guard.map(|g| g.effective()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashline::scan::scan_bytes;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn concurrent_same_session_registration_serializes_read_compare_write() {
        let registry = Arc::new(BindingRegistry::new());
        let key = SessionKey::new("/tmp/hashline-register-race", "shared-session");
        registry.register(
            &key.root,
            key.session_id.clone(),
            RegistrationRequest {
                configured_enabled: true,
                edit_slot_survives: true,
            },
        );
        registry
            .peek(&key.root, key.session_id.clone())
            .expect("initial binding")
            .with_binding_mut(|binding| {
                binding
                    .snapshots_mut()
                    .publish("race.rs", scan_bytes(b"before race\n"));
            });

        let (first_read_tx, first_read_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first_registry = Arc::clone(&registry);
        let first_key = key.clone();
        let first = thread::spawn(move || {
            first_registry.register_key(
                first_key,
                RegistrationRequest {
                    configured_enabled: false,
                    edit_slot_survives: true,
                },
                || {
                    first_read_tx.send(()).expect("signal first read");
                    release_first_rx.recv().expect("release first registration");
                },
            )
        });

        first_read_rx
            .recv()
            .expect("first registration read existing binding");
        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (second_read_tx, second_read_rx) = mpsc::channel();
        let (second_done_tx, second_done_rx) = mpsc::channel();
        let second_registry = Arc::clone(&registry);
        let second_key = key.clone();
        let second = thread::spawn(move || {
            second_started_tx.send(()).expect("signal second start");
            let outcome = second_registry.register_key(
                second_key,
                RegistrationRequest {
                    configured_enabled: true,
                    edit_slot_survives: false,
                },
                || second_read_tx.send(()).expect("signal second read"),
            );
            second_done_tx.send(outcome).expect("send second outcome");
        });

        second_started_rx
            .recv()
            .expect("second registration started");
        assert!(matches!(
            second_read_rx.recv_timeout(Duration::from_secs(1)),
            Err(RecvTimeoutError::Timeout)
        ));
        release_first_tx
            .send(())
            .expect("release first registration");

        let first_outcome = first.join().expect("first registration");
        let second_outcome = second_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second registration completes after first");
        second.join().expect("second registration");

        assert!(first_outcome.stores_cleared);
        assert!(second_outcome.stores_preserved);
        let final_binding = registry
            .peek(&key.root, key.session_id)
            .expect("final binding");
        final_binding.with_binding(|binding| {
            assert!(binding.configured_enabled());
            assert!(!binding.edit_slot_survives());
            assert!(!binding.effective());
            assert!(binding.snapshots().is_empty());
        });
    }
}
