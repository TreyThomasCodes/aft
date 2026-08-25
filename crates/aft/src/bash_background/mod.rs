//! Background bash task management: spawning detached tasks, the watchdog that
//! reaps them, output buffering/compression, and on-disk persistence so tasks
//! survive a bridge restart.

pub mod buffer;
pub mod output;
pub mod persistence;
pub mod process;
pub mod pty_process;
pub mod pty_runtime;
pub mod registry;
pub mod watchdog;
pub mod watches;

use crate::bash_permissions::PermissionAsk;
use crate::context::AppContext;
use crate::protocol::Response;
#[cfg(unix)]
use crate::sandbox_spawn::native_sandbox_enforced;
use crate::sandbox_spawn::{
    current_authenticated_principal, resolve_sandbox_spawn, HostEscalationAttempt,
    RequestedSandboxTier, SandboxTaskKind,
};
use persistence::BgMode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

pub use registry::{BgCompletion, BgTaskHealthCounts, BgTaskRegistry};

#[cfg(unix)]
pub(crate) fn resolved_shell_path(pty: bool) -> PathBuf {
    if pty {
        pty_process::resolve_posix_shell()
    } else {
        registry::resolve_posix_shell()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgTaskInfo {
    pub task_id: String,
    pub status: BgTaskStatus,
    pub command: String,
    pub mode: BgMode,
    pub started_at: u64,
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BgTaskStatus {
    Starting,
    Running,
    Killing,
    Completed,
    Failed,
    Killed,
    TimedOut,
    FateUnknown,
}

impl BgTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BgTaskStatus::Completed
                | BgTaskStatus::Failed
                | BgTaskStatus::Killed
                | BgTaskStatus::TimedOut
                | BgTaskStatus::FateUnknown
        )
    }
}

/// Spawn a bash command in the background. Returns a task_id immediately.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    request_id: &str,
    session_id: &str,
    command: &str,
    workdir: Option<PathBuf>,
    env: Option<HashMap<String, String>>,
    timeout_ms: Option<u64>,
    ctx: &AppContext,
    require_background_flag: bool,
    notify_on_completion: bool,
    compressed: bool,
    pty: bool,
    pty_rows: u16,
    pty_cols: u16,
    scanner_report: Vec<PermissionAsk>,
    host_escalation: Option<HostEscalationAttempt>,
) -> Response {
    if require_background_flag && !ctx.config().experimental_bash_background {
        return Response::error(
            request_id,
            "feature_disabled",
            "background bash is disabled; set `bash: { background: true }` (or `bash: true`) in aft.jsonc",
        );
    }

    let workdir = workdir.unwrap_or_else(|| {
        ctx.config().project_root.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
    });
    let storage_dir = task_storage_dir(ctx);
    let max_running = ctx.config().max_background_bash_tasks;
    let timeout = timeout_ms.map(Duration::from_millis);
    let project_root = ctx
        .config()
        .project_root
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .and_then(|path| std::fs::canonicalize(&path).ok().or(Some(path)));

    let mut env = env.unwrap_or_default();
    let config = ctx.config();
    let child_storage_root = self::storage_dir(config.storage_dir.as_deref());
    if let Err(error) =
        crate::agent_child_env::inject(config.as_ref(), &child_storage_root, &mut env)
    {
        return Response::error(request_id, "child_environment_unavailable", error);
    }
    let task_kind = if pty {
        SandboxTaskKind::BashPty
    } else if require_background_flag {
        SandboxTaskKind::BashBackground
    } else {
        SandboxTaskKind::BashForeground
    };
    let principal = current_authenticated_principal();
    let requested_tier = if host_escalation.is_some() {
        RequestedSandboxTier::Host
    } else if ctx.config().sandbox.enabled {
        RequestedSandboxTier::Native
    } else {
        RequestedSandboxTier::Disabled
    };
    let session_dir = persistence::session_tasks_dir(&storage_dir, session_id);
    #[cfg(unix)]
    let (spawn_plan, unregistered_task) = if native_sandbox_enforced(ctx, &principal)
        && host_escalation.is_none()
    {
        let task = match persistence::allocate_task_layout(&storage_dir, session_id) {
            Ok(task) => task,
            Err(error) => {
                return Response::error(
                    request_id,
                    "sandbox_unavailable",
                    format!(
                        "native sandbox failed to create the task artifact directory: {error}; set sandbox.enabled=false to disable native sandboxing"
                    ),
                );
            }
        };
        let plan = resolve_sandbox_spawn(
            ctx,
            &principal,
            requested_tier,
            task_kind,
            &task.paths.io_dir,
            None,
        );
        if plan.refusal_code().is_some() {
            (plan, Some(task))
        } else {
            let shell_path = resolved_shell_path(pty);
            let root = project_root.as_deref().unwrap_or(&workdir);
            let environment = crate::sandbox_spawn::approved_environment_for_plan(&plan, &env);
            match crate::sandbox_spawn::prepare_task_payload(
                &task,
                command.as_bytes(),
                root,
                &workdir,
                &principal,
                &shell_path,
                &environment,
            ) {
                Ok(prepared) => (plan.with_prepared_task(prepared), Some(task)),
                Err(error) => {
                    let _ = persistence::delete_resolved_task(&task);
                    return Response::error(
                        request_id,
                        "sandbox_unavailable",
                        format!("native sandbox failed to materialize task payload: {error}"),
                    );
                }
            }
        }
    } else {
        (
            resolve_sandbox_spawn(
                ctx,
                &principal,
                requested_tier,
                task_kind,
                &session_dir,
                host_escalation.as_ref(),
            ),
            None,
        )
    };
    #[cfg(not(unix))]
    let spawn_plan = resolve_sandbox_spawn(
        ctx,
        &principal,
        requested_tier,
        task_kind,
        &session_dir,
        host_escalation.as_ref(),
    );
    if let Some(code) = spawn_plan.refusal_code() {
        #[cfg(unix)]
        if let Some(task) = unregistered_task.as_ref() {
            let _ = persistence::delete_resolved_task(task);
        }
        let message = spawn_plan
            .refusal_message()
            .unwrap_or("bash process creation refused by sandbox policy");
        return match spawn_plan.refusal_mismatch_class() {
            Some(class) => Response::error_with_data(
                request_id,
                code,
                message,
                json!({ "mismatch_class": class }),
            ),
            None => Response::error(request_id, code, message),
        };
    }

    let cleanup_plan = spawn_plan.clone();
    let spawn_result = if pty {
        ctx.bash_background().spawn_pty(
            spawn_plan,
            command,
            session_id.to_string(),
            workdir,
            env,
            timeout,
            storage_dir,
            max_running,
            notify_on_completion,
            compressed,
            project_root,
            pty_rows,
            pty_cols,
        )
    } else {
        ctx.bash_background().spawn(
            spawn_plan,
            command,
            session_id.to_string(),
            workdir,
            env,
            timeout,
            storage_dir,
            max_running,
            notify_on_completion,
            compressed,
            project_root,
        )
    };

    match spawn_result {
        Ok(task_id) => {
            if let Err(error) =
                ctx.bash_background()
                    .record_scanner_report(&task_id, session_id, scanner_report)
            {
                crate::slog_warn!("{error}");
            }
            Response::success(
                request_id,
                json!({
                    "task_id": task_id,
                    "status": BgTaskStatus::Running,
                    "mode": if pty { "pty" } else { "pipes" },
                }),
            )
        }
        Err(message) if message.contains("limit exceeded") => {
            cleanup_plan.cleanup_unspawned();
            #[cfg(unix)]
            if let Some(task) = unregistered_task.as_ref() {
                let _ = persistence::delete_resolved_task(task);
            }
            Response::error(request_id, "background_task_limit_exceeded", message)
        }
        Err(message) => {
            cleanup_plan.cleanup_unspawned();
            #[cfg(unix)]
            if let Some(task) = unregistered_task.as_ref() {
                let _ = persistence::delete_resolved_task(task);
            }
            if cleanup_plan.is_native_launcher() {
                Response::error(
                    request_id,
                    "sandbox_unavailable",
                    format!(
                        "native sandbox failed before command execution: {message}; set sandbox.enabled=false to disable native sandboxing"
                    ),
                )
            } else {
                Response::error(request_id, "execution_failed", message)
            }
        }
    }
}

pub(crate) fn task_storage_dir(ctx: &AppContext) -> PathBuf {
    let config = ctx.config();
    let root = storage_dir(config.storage_dir.as_deref());
    config
        .harness
        .as_ref()
        .map(|harness| root.join(harness.storage_segment()))
        .unwrap_or(root)
}

/// Resolve the process-state storage root exactly once for every Rust entry point.
/// The environment override is checked here so it wins over a stale plugin wire
/// value, while both plugin-less fallback and plugin-injected paths share one root.
pub fn storage_dir(configured: Option<&std::path::Path>) -> PathBuf {
    if let Some(dir) = non_empty_env_path("AFT_STORAGE_DIR") {
        return resolve_storage_path(&dir);
    }
    if let Some(dir) = configured.filter(|dir| !dir.as_os_str().is_empty()) {
        return resolve_storage_path(dir);
    }
    if let Some(root) = cortexkit_data_root() {
        return root.join("cortexkit").join("aft");
    }
    if let Some(dir) = non_empty_env_path("AFT_CACHE_DIR") {
        return resolve_storage_path(&dir).join("aft");
    }
    std::env::temp_dir().join("cortexkit").join("aft")
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn storage_home_dir() -> Option<PathBuf> {
    let configured = if cfg!(windows) {
        non_empty_env_path("USERPROFILE").or_else(|| non_empty_env_path("HOME"))
    } else {
        non_empty_env_path("HOME").or_else(|| non_empty_env_path("USERPROFILE"))
    };
    configured.or_else(std::env::home_dir)
}

fn cortexkit_data_root() -> Option<PathBuf> {
    if let Some(dir) = non_empty_env_path("XDG_DATA_HOME") {
        return Some(resolve_storage_path(&dir));
    }
    if cfg!(windows) {
        if let Some(dir) = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var_os("APPDATA").filter(|value| !value.is_empty()))
            .map(PathBuf::from)
        {
            return Some(resolve_storage_path(&dir));
        }
    }
    storage_home_dir().map(|home| {
        let root = if cfg!(windows) {
            home.join("AppData").join("Local")
        } else {
            home.join(".local").join("share")
        };
        resolve_storage_path(&root)
    })
}

fn resolve_storage_path(path: &std::path::Path) -> PathBuf {
    let expanded = if path == std::path::Path::new("~") {
        storage_home_dir().unwrap_or_else(std::env::temp_dir)
    } else if let Some(raw) = path.to_str() {
        if raw.starts_with("~/") || raw.starts_with("~\\") {
            storage_home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(&raw[2..])
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(expanded)
    };
    normalize_absolute_path(&absolute)
}

fn normalize_absolute_path(path: &std::path::Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn repair_legacy_root_tasks(storage_root: &std::path::Path, harness: crate::harness::Harness) {
    let root_tasks = storage_root.join("bash-tasks");
    if !dir_has_entries(&root_tasks) {
        return;
    }

    let harness_tasks = storage_root
        .join(harness.storage_segment())
        .join("bash-tasks");
    if dir_has_entries(&harness_tasks) {
        return;
    }
    if let Some(parent) = harness_tasks.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            crate::slog_warn!(
                "failed to create harness bash task dir {}: {}",
                parent.display(),
                error
            );
            return;
        }
    }
    if harness_tasks.exists() {
        let _ = std::fs::remove_dir(&harness_tasks);
    }

    match std::fs::rename(&root_tasks, &harness_tasks) {
        Ok(()) => crate::slog_info!(
            "moved legacy root bash tasks into harness namespace: {}",
            harness_tasks.display()
        ),
        Err(error) => {
            crate::slog_warn!(
                "failed to move legacy root bash tasks into {}: {}; trying child merge",
                harness_tasks.display(),
                error
            );
            if std::fs::create_dir_all(&harness_tasks).is_err() {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(&root_tasks) {
                for entry in entries.flatten() {
                    let source = entry.path();
                    let target = harness_tasks.join(entry.file_name());
                    if !target.exists() {
                        let _ = std::fs::rename(source, target);
                    }
                }
            }
            let _ = std::fs::remove_dir(&root_tasks);
        }
    }
}

fn dir_has_entries(path: &std::path::Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod storage_root_tests {
    use std::ffi::{OsStr, OsString};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::path::Path;

    struct NonPanickingCleanup<F: FnOnce()> {
        cleanup: Option<F>,
    }

    impl<F: FnOnce()> NonPanickingCleanup<F> {
        fn new(cleanup: F) -> Self {
            Self {
                cleanup: Some(cleanup),
            }
        }
    }

    impl<F: FnOnce()> Drop for NonPanickingCleanup<F> {
        fn drop(&mut self) {
            let Some(cleanup) = self.cleanup.take() else {
                return;
            };
            // A cleanup panic while the test is already unwinding aborts the whole
            // libtest process, so cleanup failures must remain contained here.
            let _ = catch_unwind(AssertUnwindSafe(cleanup));
        }
    }

    struct StorageEnvGuard {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl StorageEnvGuard {
        fn capture() -> Self {
            Self {
                previous: [
                    "AFT_STORAGE_DIR",
                    "AFT_CACHE_DIR",
                    "XDG_DATA_HOME",
                    "HOME",
                    "USERPROFILE",
                    "LOCALAPPDATA",
                    "APPDATA",
                ]
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect(),
            }
        }

        fn set(&self, key: &'static str, value: Option<&OsStr>) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    impl Drop for StorageEnvGuard {
        fn drop(&mut self) {
            let previous: Vec<_> = self.previous.drain(..).collect();
            // Env restoration runs while a failing test may already be
            // unwinding; a cleanup panic at that point aborts the whole
            // libtest process (observed on Windows CI), so the restore loop
            // stays contained like NonPanickingCleanup above.
            let _ = catch_unwind(AssertUnwindSafe(move || {
                for (key, value) in previous {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }));
        }
    }


    // The plugin-injected and plugin-less paths must use one resolver so every
    // artifact lane sees the same absolute root under every environment arm.
    #[test]
    fn fallback_and_injected_roots_agree_with_storage_override_arms() {
        let _env_lock = crate::test_env::process_env_lock();
        let env = StorageEnvGuard::capture();
        let base = tempfile::tempdir().expect("storage root test directory");
        let data_home = base.path().join("data");
        let home = base.path().join("home");
        let expected_plugin_root = data_home.join("cortexkit").join("aft");
        let cache_root = base.path().join("legacy-cache");
        env.set("XDG_DATA_HOME", Some(data_home.as_os_str()));
        env.set("HOME", Some(home.as_os_str()));
        env.set("USERPROFILE", Some(home.as_os_str()));
        env.set("AFT_CACHE_DIR", Some(cache_root.as_os_str()));
        env.set("AFT_STORAGE_DIR", None);

        assert_eq!(super::storage_dir(None), expected_plugin_root);
        assert_eq!(
            super::storage_dir(Some(&expected_plugin_root)),
            expected_plugin_root
        );
        assert_eq!(
            crate::search_index::resolve_cache_dir(Path::new("/tmp/project"), None)
                .parent()
                .and_then(Path::parent),
            Some(expected_plugin_root.as_path())
        );

        env.set(
            "AFT_STORAGE_DIR",
            Some(OsStr::new("./relative/../local-aft-storage")),
        );
        let expected_relative = super::resolve_storage_path(Path::new("./local-aft-storage"));
        assert!(expected_relative.is_absolute());
        assert_eq!(super::storage_dir(None), expected_relative);
        assert_eq!(
            super::storage_dir(Some(&expected_plugin_root)),
            expected_relative
        );

        env.set("AFT_STORAGE_DIR", Some(OsStr::new("")));
        assert_eq!(super::storage_dir(None), expected_plugin_root);
        assert_eq!(
            super::storage_dir(Some(&expected_plugin_root)),
            expected_plugin_root
        );

        env.set("AFT_STORAGE_DIR", Some(OsStr::new("~/tilde-aft-storage")));
        let expected_tilde = home.join("tilde-aft-storage");
        assert_eq!(super::storage_dir(None), expected_tilde);
        assert_eq!(
            super::storage_dir(Some(&expected_plugin_root)),
            expected_tilde
        );
    }

    #[test]
    fn cleanup_panic_during_unwind_does_not_abort_libtest() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let cleanup_ran = AtomicBool::new(false);
        let unwind = catch_unwind(AssertUnwindSafe(|| {
            let _cleanup = NonPanickingCleanup::new(|| {
                cleanup_ran.store(true, Ordering::SeqCst);
                panic!("forced cleanup failure");
            });
            panic!("primary test failure");
        }));

        assert!(cleanup_ran.load(Ordering::SeqCst));
        assert_eq!(
            unwind
                .expect_err("primary panic must escape the inner scope")
                .downcast_ref::<&str>(),
            Some(&"primary test failure")
        );
    }
}
