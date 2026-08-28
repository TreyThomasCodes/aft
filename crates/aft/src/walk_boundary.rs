//! Filesystem-boundary guards for recursive directory walks.
//!
//! On macOS, `ReadDir` can panic while it is dropped when a mounted filesystem
//! disappears (`closedir(3)` returns ENXIO). A destructor panic cannot unwind
//! safely, so it aborts the whole daemon. Recursive walks must therefore avoid
//! opening a child directory that belongs to another mounted filesystem.

use std::io;
use std::path::{Path, PathBuf};

/// Filesystem identity captured from the root of one recursive walk.
///
/// Unix exposes the device id directly through `MetadataExt::dev`. Windows does
/// not expose a stable volume serial through `std::fs::Metadata`, so raw
/// `read_dir` walkers keep their historical behavior there; `ignore::WalkBuilder`
/// still uses its cross-platform `same_file_system` implementation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeviceBoundary {
    root_device: Option<u64>,
}

impl DeviceBoundary {
    /// Capture the device that recursive descendants must remain on.
    pub(crate) fn for_root(root: &Path) -> io::Result<Self> {
        Ok(Self {
            root_device: filesystem_device_id(root)?,
        })
    }

    /// Return whether `child` can be entered without crossing the root mount.
    pub(crate) fn should_descend(&self, child: &Path) -> io::Result<bool> {
        self.should_descend_with(child, filesystem_device_id)
    }

    /// Lookup-injectable form used by recursive walkers' regression tests.
    pub(crate) fn should_descend_with<F>(&self, child: &Path, device_lookup: F) -> io::Result<bool>
    where
        F: FnOnce(&Path) -> io::Result<Option<u64>>,
    {
        let Some(root_device) = self.root_device else {
            // See the type-level Windows note above. There is no std-only volume
            // serial to compare on Windows, so this intentionally remains a no-op.
            return Ok(true);
        };
        let Some(child_device) = device_lookup(child)? else {
            return Ok(true);
        };
        Ok(same_filesystem_device(root_device, child_device))
    }

    #[cfg(test)]
    pub(crate) fn from_device_for_test(root_device: u64) -> Self {
        Self {
            root_device: Some(root_device),
        }
    }
}

/// Expand a glob by walking only its literal base on the same filesystem.
///
/// The `glob` crate traverses `**` internally and offers no filesystem-boundary
/// option. Routing recursive glob expansion through `ignore` prevents a vanished
/// mounted child from turning `ReadDir::drop`'s ENXIO into a daemon abort.
pub(crate) fn expand_glob_same_file_system(
    full_pattern: &str,
) -> Result<Vec<PathBuf>, glob::PatternError> {
    let normalized = full_pattern.replace('\\', "/");
    let Some(first_glob) = normalized.find(['*', '?', '[', '{']) else {
        return Ok(Vec::new());
    };
    let (base, relative_pattern) = match normalized[..first_glob].rfind('/') {
        Some(0) => (PathBuf::from("/"), &normalized[1..]),
        Some(base_end) => (
            PathBuf::from(&normalized[..base_end]),
            &normalized[base_end + 1..],
        ),
        None => (PathBuf::from("."), normalized.as_str()),
    };
    let relative = glob::Pattern::new(relative_pattern)?;
    let options = glob::MatchOptions {
        case_sensitive: !cfg!(windows),
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };

    // Do not cross a mount while expanding an agent-provided glob: a vanished
    // child ReadDir can panic in Drop after closedir reports ENXIO and abort AFT.
    Ok(ignore::WalkBuilder::new(&base)
        .same_file_system(true)
        .hidden(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.strip_prefix(&base)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .is_some_and(|path| relative.matches_with(&path, options))
        })
        .collect())
}

/// Direct device-boundary predicate. Keeping it independent of filesystem I/O
/// gives tests a portable mutation target without requiring a fragile loopback
/// mount on macOS.
pub(crate) fn same_filesystem_device(root_device: u64, child_device: u64) -> bool {
    root_device == child_device
}

#[cfg(unix)]
pub(crate) fn filesystem_device_id(path: &Path) -> io::Result<Option<u64>> {
    use std::os::unix::fs::MetadataExt;

    Ok(Some(std::fs::metadata(path)?.dev()))
}

#[cfg(not(unix))]
pub(crate) fn filesystem_device_id(_path: &Path) -> io::Result<Option<u64>> {
    // Standard metadata on non-Unix platforms has file attributes but no
    // reliable volume identifier, so this raw-directory-walk check is a no-op.
    // Recursive walks using `ignore::WalkBuilder` still enforce their own
    // cross-filesystem restriction.
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::{same_filesystem_device, DeviceBoundary};

    #[test]
    fn device_predicate_accepts_root_device_and_rejects_foreign_device() {
        assert!(same_filesystem_device(41, 41));
        assert!(
            !same_filesystem_device(41, 99),
            "a child on another device must not be entered"
        );
    }

    #[test]
    fn injectable_device_lookup_skips_foreign_child() {
        let boundary = DeviceBoundary::from_device_for_test(41);
        let foreign = Path::new("/simulated-foreign-mount");

        assert!(
            !boundary
                .should_descend_with(foreign, |_| Ok(Some(99)))
                .expect("simulated device lookup"),
            "the injected foreign child must be skipped"
        );
        assert!(
            boundary
                .should_descend_with(Path::new("/same-mount"), |_| Ok(Some(41)))
                .expect("simulated device lookup"),
            "the root filesystem remains walkable"
        );
        assert!(
            boundary
                .should_descend_with(Path::new("/lookup-error"), |_| {
                    Err(io::Error::other("simulated stat failure"))
                })
                .is_err(),
            "walkers must disclose stat failures instead of treating them as safe"
        );
    }
}
