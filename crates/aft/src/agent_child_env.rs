//! AFT-owned environment and files for first-party agent children.
//!
//! The governance controls in this module are attached to spawned bash and PTY
//! children. AFT never edits the user's shell startup files or global Git
//! configuration, so an operator's terminal keeps its existing behavior.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use crate::config::Config;

pub const SHIMS_DIR_NAME: &str = "shims";
pub const GIT_HOOKS_DIR_NAME: &str = "git-hooks";
const PREPARE_COMMIT_MSG: &str = "prepare-commit-msg";
const GH_SHIMS_DIR_ENV: &str = "AFT_GH_SHIMS_DIR";
const GH_SHIM_BINARY_ENV: &str = "AFT_GH_SHIM_BINARY";
const GIT_CO_AUTHOR_ENV: &str = "AFT_GIT_CO_AUTHOR";

/// The generated hook is POSIX `sh`, including on Git for Windows. It appends
/// AFT's attribution before handing control to a repository hook with the same
/// arguments Git supplied.
const PREPARE_COMMIT_MSG_HOOK: &str = r#"#!/bin/sh
# AFT selects this hook through the agent child's environment. It does not alter
# the repository or the user's Git configuration.
# Agent-labeled commits are joint work too, so subjects such as "mason:" do not
# receive an attribution exemption.
msg_file=$1
hook_name=prepare-commit-msg
mode=${AFT_GIT_CO_AUTHOR:-off}
line=

case "$mode" in
  off|'') ;;
  auto)
    if [ -n "${AFT_GH_SHIM_BINARY:-}" ]; then
      line=$("$AFT_GH_SHIM_BINARY" gh-shim --co-author-line 2>/dev/null || :)
    fi
    ;;
  *) line="Co-authored-by: $mode" ;;
esac

if [ -n "$line" ]; then
  trailers=$(git interpret-trailers --parse "$msg_file" 2>/dev/null || :)
  identity=${line#Co-authored-by: }
  email=$(printf '%s\n' "$identity" | sed -n 's/.*<\([^<>]*\)>$/\1/p')
  present=false
  if printf '%s\n' "$trailers" | grep -i -F -x "$line" >/dev/null 2>&1; then
    present=true
  elif [ -n "$email" ] && printf '%s\n' "$trailers" | grep -i '^Co-authored-by:' | grep -i -F "<$email>" >/dev/null 2>&1; then
    present=true
  fi
  if [ "$present" = false ]; then
    printf '\n%s\n' "$line" >> "$msg_file"
  fi
fi

repo_hooks=$(git config --local --get core.hooksPath 2>/dev/null || :)
if [ -n "$repo_hooks" ]; then
  case "$repo_hooks" in
    /*|[A-Za-z]:[\\/]*) candidate="$repo_hooks/$hook_name" ;;
    \~/*) candidate="${HOME:-}${repo_hooks#\~}/$hook_name" ;;
    *) candidate="$PWD/$repo_hooks/$hook_name" ;;
  esac
else
  git_dir=${GIT_DIR:-$(git rev-parse --git-dir 2>/dev/null || :)}
  case "$git_dir" in
    /*|[A-Za-z]:[\\/]*) candidate="$git_dir/hooks/$hook_name" ;;
    *) candidate="$PWD/$git_dir/hooks/$hook_name" ;;
  esac
fi

if [ -x "$candidate" ]; then
  candidate_dir=$(cd "$(dirname "$candidate")" 2>/dev/null && pwd -P)
  self_dir=$(cd "$(dirname "$0")" 2>/dev/null && pwd -P)
  if [ -z "$candidate_dir" ] || [ -z "$self_dir" ] || [ "$candidate_dir/$hook_name" != "$self_dir/$hook_name" ]; then
    exec "$candidate" "$@"
  fi
fi

exit 0
"#;

/// Refresh files selected by the resolved configuration. This runs during
/// configure and is also cheap enough to repair a stale entry immediately
/// before a child spawn.
pub fn maintain(config: &Config, storage_root: &Path) -> Result<(), String> {
    let shims_dir = storage_root.join(SHIMS_DIR_NAME);
    if config.gh_shim.enabled {
        let binary = shim_binary(config)?;
        match probe_gh_shim_binary(&binary) {
            Ok(()) => ensure_gh_entry(&shims_dir, &binary)?,
            Err(reason) => {
                crate::slog_warn!(
                    "[agent_child_env] refusing gh shim candidate {}: {reason}",
                    binary.display()
                );
                if !existing_gh_entry_is_valid(&shims_dir) {
                    remove_gh_entry(&shims_dir)?;
                    crate::slog_warn!(
                        "[agent_child_env] removed unverified gh shim entry after refusing candidate {}",
                        binary.display()
                    );
                }
            }
        }
    } else {
        remove_gh_entry(&shims_dir)?;
    }

    if config.git.co_author != "off" {
        ensure_prepare_commit_msg_hook(&storage_root.join(GIT_HOOKS_DIR_NAME))?;
    }
    Ok(())
}

/// Add governance to one child environment. This is the single seam used
/// before foreground, background, sandboxed, and PTY launch planning.
pub fn inject(
    config: &Config,
    storage_root: &Path,
    environment: &mut HashMap<String, String>,
) -> Result<(), String> {
    let gh_enabled = config.gh_shim.enabled;
    let co_author_enabled = config.git.co_author != "off";
    if !gh_enabled && !co_author_enabled {
        return Ok(());
    }

    maintain(config, storage_root)?;

    if gh_enabled {
        let shims_dir = storage_root.join(SHIMS_DIR_NAME);
        let inherited = environment
            .get("PATH")
            .map(OsString::from)
            .unwrap_or_else(|| crate::effective_path::effective_path().to_os_string());
        let mut entries = vec![shims_dir.clone()];
        entries.extend(std::env::split_paths(&inherited).filter(|entry| entry != &shims_dir));
        let path = std::env::join_paths(entries)
            .map_err(|error| format!("failed to construct governed child PATH: {error}"))?;
        environment.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        environment.insert(
            GH_SHIMS_DIR_ENV.to_string(),
            shims_dir.to_string_lossy().into_owned(),
        );
    }

    if co_author_enabled {
        environment.insert("GIT_CONFIG_COUNT".to_string(), "1".to_string());
        environment.insert("GIT_CONFIG_KEY_0".to_string(), "core.hooksPath".to_string());
        environment.insert(
            "GIT_CONFIG_VALUE_0".to_string(),
            storage_root
                .join(GIT_HOOKS_DIR_NAME)
                .to_string_lossy()
                .into_owned(),
        );
        environment.insert(GIT_CO_AUTHOR_ENV.to_string(), config.git.co_author.clone());
        if config.git.co_author == "auto" {
            environment.insert(
                GH_SHIM_BINARY_ENV.to_string(),
                shim_binary(config)?.to_string_lossy().into_owned(),
            );
        }
    }

    Ok(())
}

pub fn shim_binary(config: &Config) -> Result<PathBuf, String> {
    let binary = match config.gh_shim.binary_path.as_ref() {
        Some(path) => path.clone(),
        None => std::env::current_exe()
            .map_err(|error| format!("failed to resolve the running AFT binary: {error}"))?,
    };
    if !binary.is_absolute() {
        return Err(format!(
            "gh_shim.binary_path must be absolute: {}",
            binary.display()
        ));
    }
    Ok(binary)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ShimProbeCacheKey {
    path: PathBuf,
    modified: Option<Duration>,
    size: u64,
}

#[derive(serde::Deserialize)]
struct ShimSelfReport {
    shim_version: String,
    gh_routing_schema_floor: u64,
}

static SHIM_PROBE_CACHE: OnceLock<Mutex<HashMap<ShimProbeCacheKey, Result<(), String>>>> =
    OnceLock::new();

/// Verify behavior rather than executable names: installation may point at a
/// renamed AFT image, while a process that merely resembles one must not become
/// the agent child's `gh` command.
fn probe_gh_shim_binary(binary: &Path) -> Result<(), String> {
    let metadata =
        fs::metadata(binary).map_err(|error| format!("could not stat candidate: {error}"))?;
    let key = ShimProbeCacheKey {
        path: binary.to_path_buf(),
        modified: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok()),
        size: metadata.len(),
    };
    let cache = SHIM_PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        return cached;
    }

    let result = probe_gh_shim_binary_uncached(binary);
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, result.clone());
    result
}

fn probe_gh_shim_binary_uncached(binary: &Path) -> Result<(), String> {
    // Invoke the image directly, including on Windows where the managed entry is
    // a gh.cmd wrapper. This keeps validation independent of the wrapper's shell.
    let output = Command::new(binary)
        .args(["gh-shim", "--shim-version"])
        .output()
        .map_err(|error| format!("could not execute --shim-version probe: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "--shim-version probe exited with {status}",
            status = output.status
        ));
    }
    let report: ShimSelfReport = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("--shim-version probe emitted invalid JSON: {error}"))?;
    if report.shim_version.is_empty() || report.gh_routing_schema_floor == 0 {
        return Err("--shim-version probe omitted required shim identity fields".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn existing_gh_entry_is_valid(shims_dir: &Path) -> bool {
    let entry = shims_dir.join("gh");
    let binary = match fs::read_link(&entry) {
        Ok(target) if target.is_absolute() => target,
        Ok(target) => shims_dir.join(target),
        Err(_) => entry,
    };
    probe_gh_shim_binary(&binary).is_ok()
}

#[cfg(windows)]
fn existing_gh_entry_is_valid(shims_dir: &Path) -> bool {
    let entry = shims_dir.join("gh.cmd");
    let Ok(wrapper) = fs::read_to_string(entry) else {
        return false;
    };
    let Some(binary) = wrapper
        .strip_prefix("@echo off\r\n\"")
        .and_then(|line| line.strip_suffix("\" gh-shim %*\r\n"))
        .map(|path| PathBuf::from(path.replace("%%", "%")))
    else {
        return false;
    };
    probe_gh_shim_binary(&binary).is_ok()
}

#[cfg(not(any(unix, windows)))]
fn existing_gh_entry_is_valid(_shims_dir: &Path) -> bool {
    false
}

fn ensure_prepare_commit_msg_hook(hooks_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(hooks_dir).map_err(|error| {
        format!(
            "failed to create child Git hooks directory {}: {error}",
            hooks_dir.display()
        )
    })?;
    let hook = hooks_dir.join(PREPARE_COMMIT_MSG);
    write_if_changed(&hook, PREPARE_COMMIT_MSG_HOOK.as_bytes())?;
    #[cfg(unix)]
    set_executable(&hook)?;
    Ok(())
}

#[cfg(unix)]
fn ensure_gh_entry(shims_dir: &Path, binary: &Path) -> Result<(), String> {
    use std::os::unix::fs::symlink;

    fs::create_dir_all(shims_dir).map_err(|error| {
        format!(
            "failed to create gh shim directory {}: {error}",
            shims_dir.display()
        )
    })?;
    let entry = shims_dir.join("gh");
    if fs::read_link(&entry).ok().as_deref() == Some(binary) {
        return Ok(());
    }
    if entry.is_dir() {
        return Err(format!(
            "cannot replace gh shim entry because it is a directory: {}",
            entry.display()
        ));
    }
    let temporary = shims_dir.join(format!(".gh.tmp.{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    symlink(binary, &temporary).map_err(|error| {
        format!(
            "failed to create gh shim link {} -> {}: {error}",
            temporary.display(),
            binary.display()
        )
    })?;
    fs::rename(&temporary, &entry).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "failed to install gh shim link {}: {error}",
            entry.display()
        )
    })
}

#[cfg(windows)]
fn ensure_gh_entry(shims_dir: &Path, binary: &Path) -> Result<(), String> {
    fs::create_dir_all(shims_dir).map_err(|error| {
        format!(
            "failed to create gh shim directory {}: {error}",
            shims_dir.display()
        )
    })?;
    write_if_changed(&shims_dir.join("gh.cmd"), &windows_gh_cmd(binary))
}

#[cfg(not(any(unix, windows)))]
fn ensure_gh_entry(_shims_dir: &Path, _binary: &Path) -> Result<(), String> {
    Err("gh child PATH injection is unsupported on this platform".to_string())
}

fn remove_gh_entry(shims_dir: &Path) -> Result<(), String> {
    for name in ["gh", "gh.cmd"] {
        let entry = shims_dir.join(name);
        match fs::remove_file(&entry) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to remove disabled gh shim entry {}: {error}",
                    entry.display()
                ));
            }
        }
    }
    match fs::remove_dir(shims_dir) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "failed to remove empty gh shim directory {}: {error}",
            shims_dir.display()
        )),
    }
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    if path.is_dir() {
        return Err(format!(
            "cannot replace managed child file because it is a directory: {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("managed child file has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create managed child directory {}: {error}",
            parent.display()
        )
    })?;
    let temporary = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    fs::write(&temporary, bytes).map_err(|error| {
        format!(
            "failed to write managed child file {}: {error}",
            temporary.display()
        )
    })?;
    // Windows rename does not replace an existing destination. Managed files
    // contain no user data, so remove only the exact stale file before install.
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|error| {
            format!(
                "failed to replace stale managed child file {}: {error}",
                path.display()
            )
        })?;
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "failed to install managed child file {}: {error}",
            path.display()
        )
    })
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| {
            format!(
                "failed to read hook permissions {}: {error}",
                path.display()
            )
        })?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to make hook executable {}: {error}", path.display()))
}

/// Render the Windows command wrapper separately so its quoting contract can be
/// checked on every development platform; `cmd.exe` dispatch still requires
/// the native Windows CI oracle.
pub fn windows_gh_cmd(binary: &Path) -> Vec<u8> {
    let rendered = binary.to_string_lossy();
    debug_assert!(!rendered.contains('"'));
    let rendered = rendered.replace('%', "%%");
    format!("@echo off\r\n\"{rendered}\" gh-shim %*\r\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GitConfig};

    #[test]
    fn disabled_features_leave_the_requested_environment_byte_identical() {
        let mut config = Config::default();
        config.gh_shim.enabled = false;
        config.git = GitConfig::default();
        let before = HashMap::from([
            ("PATH".to_string(), "/one:/two".to_string()),
            ("CUSTOM".to_string(), "value".to_string()),
        ]);
        let mut after = before.clone();
        inject(&config, Path::new("/unused"), &mut after).unwrap();
        assert_eq!(after, before);
    }

    #[cfg(unix)]
    #[test]
    fn configure_maintenance_refreshes_stale_gh_links_and_removes_disabled_entries() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("aft-first");
        let second = temp.path().join("aft-second");
        write_self_reporting_shim(&first);
        write_self_reporting_shim(&second);
        let mut config = Config::default();
        config.gh_shim.binary_path = Some(first);
        maintain(&config, temp.path()).unwrap();
        let entry = temp.path().join("shims/gh");
        assert_eq!(
            fs::read_link(&entry).unwrap(),
            config.gh_shim.binary_path.as_deref().unwrap()
        );

        config.gh_shim.binary_path = Some(second);
        maintain(&config, temp.path()).unwrap();
        assert_eq!(
            fs::read_link(&entry).unwrap(),
            config.gh_shim.binary_path.as_deref().unwrap()
        );

        config.gh_shim.enabled = false;
        maintain(&config, temp.path()).unwrap();
        assert!(fs::symlink_metadata(entry).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn configure_maintenance_refuses_harnesses_and_preserves_verified_shims() {
        let temp = tempfile::tempdir().unwrap();
        let verified = temp.path().join("aft-verified");
        let harness = temp.path().join("aft-test-harness");
        write_self_reporting_shim(&verified);
        write_executable(
            &harness,
            "#!/bin/sh\nif [ \"${2:-}\" = \"--shim-version\" ]; then exit 2; fi\nexit 0\n",
        );

        let mut config = Config::default();
        config.gh_shim.binary_path = Some(verified.clone());
        maintain(&config, temp.path()).unwrap();
        let entry = temp.path().join("shims/gh");
        assert_eq!(fs::read_link(&entry).unwrap(), verified);

        config.gh_shim.binary_path = Some(harness);
        maintain(&config, temp.path()).unwrap();
        assert_eq!(
            fs::read_link(&entry).unwrap(),
            verified,
            "a rejected candidate must not replace a verified shim"
        );

        fs::remove_file(&entry).unwrap();
        maintain(&config, temp.path()).unwrap();
        assert!(
            fs::symlink_metadata(entry).is_err(),
            "a rejected candidate must not install a new gh entry"
        );
    }

    #[test]
    fn windows_wrapper_uses_the_explicit_gh_shim_dispatch_form() {
        assert_eq!(
            String::from_utf8(windows_gh_cmd(Path::new(r"C:\AFT Dev\aft.exe"))).unwrap(),
            "@echo off\r\n\"C:\\AFT Dev\\aft.exe\" gh-shim %*\r\n"
        );
    }

    #[test]
    fn generated_hook_stays_posix_and_documents_joint_agent_attribution() {
        assert!(PREPARE_COMMIT_MSG_HOOK.starts_with("#!/bin/sh\n"));
        assert!(!PREPARE_COMMIT_MSG_HOOK.contains("[["));
        assert!(!PREPARE_COMMIT_MSG_HOOK.contains("function "));
        assert!(!PREPARE_COMMIT_MSG_HOOK.contains("mason:*)"));
        assert!(PREPARE_COMMIT_MSG_HOOK.contains("do not\n# receive an attribution exemption"));
    }

    #[cfg(unix)]
    fn run_git(repo: &Path, args: &[&str], environment: &HashMap<String, String>) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .envs(environment)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed: {status}");
    }

    #[cfg(unix)]
    fn initialize_repo(repo: &Path) {
        fs::create_dir_all(repo).unwrap();
        let environment = HashMap::new();
        run_git(repo, &["init", "--quiet"], &environment);
        run_git(repo, &["config", "user.name", "AFT Test"], &environment);
        run_git(
            repo,
            &["config", "user.email", "aft-test@example.test"],
            &environment,
        );
        fs::write(repo.join("tracked.txt"), "one\n").unwrap();
        run_git(repo, &["add", "tracked.txt"], &environment);
    }

    #[cfg(unix)]
    fn commit_message(repo: &Path) -> String {
        let output = std::process::Command::new("git")
            .args(["log", "-1", "--format=%B"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        set_executable(path).unwrap();
    }

    #[cfg(unix)]
    fn write_self_reporting_shim(path: &Path) {
        write_executable(
            path,
            "#!/bin/sh\nif [ \"${1:-}\" = \"gh-shim\" ] && [ \"${2:-}\" = \"--shim-version\" ]; then\n  printf '%s\\n' '{\"shim_version\":\"test\",\"gh_routing_schema_floor\":1}'\n  exit 0\nfi\nexit 1\n",
        );
    }

    #[cfg(unix)]
    #[test]
    fn auto_hook_is_idempotent_and_chains_default_repository_hook() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let storage = temp.path().join("storage");
        initialize_repo(&repo);
        let shim = temp.path().join("fake-aft");
        write_executable(
            &shim,
            "#!/bin/sh\nprintf '%s\\n' 'Co-authored-by: aft-alfonso[bot] <318960130+aft-alfonso[bot]@users.noreply.github.com>'\n",
        );
        let local_hook = repo.join(".git/hooks/prepare-commit-msg");
        write_executable(
            &local_hook,
            "#!/bin/sh\nprintf '%s\\n' 'Local-Hook: default' >> \"$1\"\n",
        );

        let mut config = Config::default();
        config.gh_shim.enabled = false;
        config.gh_shim.binary_path = Some(shim);
        config.git.co_author = "auto".to_string();
        let mut environment = HashMap::new();
        inject(&config, &storage, &mut environment).unwrap();
        run_git(
            &repo,
            &["commit", "--quiet", "-m", "mason: joint work"],
            &environment,
        );
        run_git(
            &repo,
            &["commit", "--quiet", "--amend", "--no-edit"],
            &environment,
        );

        let message = commit_message(&repo);
        assert_eq!(message.matches("Co-authored-by:").count(), 1);
        assert!(message.contains(
            "Co-authored-by: aft-alfonso[bot] <318960130+aft-alfonso[bot]@users.noreply.github.com>"
        ));
        assert_eq!(message.matches("Local-Hook: default").count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_hook_skips_derivation_and_chains_custom_hooks_path() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let storage = temp.path().join("storage");
        initialize_repo(&repo);
        let environment = HashMap::new();
        run_git(
            &repo,
            &["config", "core.hooksPath", ".custom-hooks"],
            &environment,
        );
        let custom_hook = repo.join(".custom-hooks/prepare-commit-msg");
        fs::create_dir_all(custom_hook.parent().unwrap()).unwrap();
        write_executable(
            &custom_hook,
            "#!/bin/sh\nprintf '%s\\n' 'Local-Hook: custom' >> \"$1\"\n",
        );

        let mut config = Config::default();
        config.gh_shim.enabled = false;
        config.git.co_author = "Pair Agent <pair@example.test>".to_string();
        let mut environment = HashMap::new();
        inject(&config, &storage, &mut environment).unwrap();
        assert!(!environment.contains_key(GH_SHIM_BINARY_ENV));
        run_git(
            &repo,
            &["commit", "--quiet", "-m", "explicit pair"],
            &environment,
        );

        let message = commit_message(&repo);
        assert!(message.contains("Co-authored-by: Pair Agent <pair@example.test>"));
        assert!(message.contains("Local-Hook: custom"));
    }
}
