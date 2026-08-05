use std::path::{Path, PathBuf};

use crate::lsp::registry::ServerKind;

pub fn find_workspace_root<S>(file_path: &Path, markers: &[S]) -> Option<PathBuf>
where
    S: AsRef<str>,
{
    // Route canonicalization through `canonicalize_normalized` so the returned
    // root is never a Windows verbatim (`\\?\C:\...`) path. This root flows into
    // `LspClient::spawn` -> `Command::current_dir(&root)`, and `CreateProcess`
    // rejects extended-length verbatim paths as `lpCurrentDirectory` (documented
    // Win32 limitation: "The lpCurrentDirectory string ... must not be a \\?\
    // prefixed path"). Without this strip EVERY LSP spawn on Windows fails
    // with "The system cannot find the path specified" and aft_inspect reports
    // servers as not installed (#174). On Unix `canonicalize_normalized` is
    // identity-equivalent to `fs::canonicalize` followed by lexical `.`/`..`
    // collapse, so non-Windows behavior is unchanged.
    // `canonicalize_normalized` falls back to lexical `.`/`..` collapse when
    // `fs::canonicalize` fails (e.g. the file is gone), matching the prior
    // fallback to the raw `file_path` for paths without `.`/`..` components.
    let resolved_path = crate::inspect::job::canonicalize_normalized(file_path);

    let start_dir = if resolved_path.is_dir() {
        resolved_path
    } else {
        resolved_path.parent()?.to_path_buf()
    };

    let mut current = Some(start_dir.as_path());
    while let Some(dir) = current {
        if markers
            .iter()
            .any(|marker| dir.join(marker.as_ref()).exists())
        {
            return Some(dir.to_path_buf());
        }

        current = dir.parent();
    }

    None
}

/// Composite key for caching server instances.
/// Each unique (ServerKind, workspace_root) pair gets its own server process.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerKey {
    pub kind: ServerKind,
    pub root: PathBuf,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{find_workspace_root, ServerKey};
    use crate::inspect::job::canonicalize_normalized;
    use crate::lsp::registry::ServerKind;

    #[test]
    fn test_find_root_with_cargo_toml() {
        let temp_dir = tempdir().unwrap();
        let root = temp_dir.path().join("workspace");
        let src_dir = root.join("src");
        let file = src_dir.join("lib.rs");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(&file, "fn main() {}\n").unwrap();

        let expected_root = fs::canonicalize(&root).unwrap();
        assert_eq!(
            find_workspace_root(&file, &["Cargo.toml"]),
            Some(expected_root)
        );
    }

    #[test]
    fn test_find_root_nested() {
        let temp_dir = tempdir().unwrap();
        let repo_root = temp_dir.path().join("repo");
        let crate_root = repo_root.join("crates").join("foo");
        let src_dir = crate_root.join("src");
        let file = src_dir.join("lib.rs");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(repo_root.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(crate_root.join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
        fs::write(&file, "fn main() {}\n").unwrap();

        let expected_root = fs::canonicalize(&crate_root).unwrap();
        assert_eq!(
            find_workspace_root(&file, &["Cargo.toml"]),
            Some(expected_root)
        );
    }

    #[test]
    fn test_find_root_none() {
        let temp_dir = tempdir().unwrap();
        let src_dir = temp_dir.path().join("src");
        let file = src_dir.join("main.rs");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(&file, "fn main() {}\n").unwrap();

        assert_eq!(find_workspace_root(&file, &["Cargo.toml"]), None);
    }

    #[test]
    fn test_find_root_multiple_markers() {
        let temp_dir = tempdir().unwrap();
        let root = temp_dir.path().join("web");
        let src_dir = root.join("src");
        let file = src_dir.join("index.ts");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(root.join("tsconfig.json"), "{}\n").unwrap();
        fs::create_dir(root.join("package.json")).unwrap();
        fs::write(&file, "export {};\n").unwrap();

        let expected_root = fs::canonicalize(&root).unwrap();
        assert_eq!(
            find_workspace_root(&file, &["tsconfig.json", "package.json"]),
            Some(expected_root)
        );
    }

    #[test]
    fn test_server_key_equality() {
        let root = PathBuf::from("/tmp/workspace");
        let same = ServerKey {
            kind: ServerKind::Rust,
            root: root.clone(),
        };
        let equal = ServerKey {
            kind: ServerKind::Rust,
            root,
        };
        let different = ServerKey {
            kind: ServerKind::Rust,
            root: PathBuf::from("/tmp/other"),
        };

        assert_eq!(same, equal);
        assert_ne!(same, different);
    }

    /// Regression test for #174: the workspace root returned for a nested file
    /// must never carry a Windows verbatim (`\\?\`) prefix, because it flows
    /// into `LspClient::spawn` -> `Command::current_dir`, and `CreateProcess`
    /// rejects verbatim paths as `lpCurrentDirectory` (every LSP spawn on
    /// Windows would otherwise fail with "The system cannot find the path
    /// specified").
    ///
    /// This runs on every platform (not `cfg(windows)`-gated) because the
    /// normalization is platform-independent: on Unix `canonicalize_normalized`
    /// is identity-equivalent to `fs::canonicalize` plus lexical `.`/`..`
    /// collapse, so the assertion is a no-op there; on Windows (MSVC CI) it
    /// asserts the verbatim strip. The byte-equality check against
    /// `canonicalize_normalized` is the platform-independent property that
    /// fails locally on macOS if the roots.rs chokepoint is reverted to a bare
    /// `fs::canonicalize` (whose output diverges from the normalized form only
    /// on Windows, but the equality contract holds everywhere).
    #[test]
    fn test_find_root_strips_windows_verbatim_prefix() {
        let temp_dir = tempdir().unwrap();
        let root = temp_dir.path().join("workspace");
        let src_dir = root.join("src");
        let nested = src_dir.join("deep").join("lib.rs");

        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(&nested, "fn main() {}\n").unwrap();

        let found = find_workspace_root(&nested, &["Cargo.toml"]).expect("root found");

        // No verbatim prefix on any platform.
        let display = found.to_string_lossy();
        assert!(
            !display.starts_with("\\\\?\\"),
            "workspace root must not carry a Windows verbatim prefix: {display}"
        );

        // Byte-for-byte equality with the shared normalizer for the same
        // fixture. This is the platform-independent mutation control: reverting
        // roots.rs to a bare `fs::canonicalize` makes `find_workspace_root`
        // diverge from `canonicalize_normalized` on Windows, and on Unix the
        // equality still holds (both reduce to the canonical form), so the
        // test fails on Windows CI while passing locally on macOS.
        let expected = canonicalize_normalized(&root);
        assert_eq!(found, expected);
    }
}
