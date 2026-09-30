use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::native_fs::{self, unlock_file};

pub(crate) const RASTER_OWNERSHIP_MARKER_FILE: &str = ".zpres-raster-page-set.json";
const HTML_MANAGED_MARKERS: [&str; 3] = [
    ".zpres-output.json",
    ".zpres-generation.json",
    ".zpres-stage.json",
];
const NAMESPACE_LOCK_FILE: &str = "output-namespace-v1.lock";
const FILE_STAGE_PREFIX: &str = ".zpres-file-stage-";
const UNIQUE_ATTEMPTS: usize = 128;

static UNIQUE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputTargetKind {
    File,
    Directory,
}

#[derive(Debug, Error)]
pub(crate) enum OutputOwnershipError {
    #[error(
        "output publication is supported only on macOS and Linux; refusing to mutate '{}'",
        path.display()
    )]
    UnsupportedPlatform { path: PathBuf },

    #[error(
        "refusing to write Output target '{}' inside exclusively owned raster page directory '{}'",
        path.display(),
        owner.display()
    )]
    OwnedRasterAncestor { path: PathBuf, owner: PathBuf },

    #[error(
        "refusing to write Output target '{}' inside or over zpres-managed HTML publication '{}'",
        path.display(),
        owner.display()
    )]
    OwnedHtmlAncestor { path: PathBuf, owner: PathBuf },

    #[error("invalid zpres output namespace lock '{}': {reason}", path.display())]
    InvalidNamespaceLock { path: PathBuf, reason: String },

    #[error("invalid Output target '{}': {reason}", path.display())]
    InvalidOutputTarget { path: PathBuf, reason: String },

    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Serializes every mutating zpres Output-target boundary on this host for the
/// current user.
///
/// The deliberately broad lock makes parent/child ownership checks stable even
/// when paths use different spellings, mounts, or filesystems. Callers acquire
/// it before creating an output parent or scratch file and hold it through the
/// final commit.
pub(crate) struct OutputNamespaceGuard {
    file: File,
    _lock_path: PathBuf,
}

pub(crate) struct OutputFilePublication {
    pub(crate) warnings: Vec<String>,
}

impl OutputNamespaceGuard {
    pub(crate) fn acquire(requested: &Path) -> Result<Self, OutputOwnershipError> {
        ensure_supported_platform(requested)?;
        let lock_path = namespace_lock_path()?;
        let file = open_namespace_lock(&lock_path)?;
        lock_file_blocking(&file, &lock_path)?;
        Ok(Self {
            file,
            _lock_path: lock_path,
        })
    }

    /// Refuse any peer file or tree output at or below a raster-owned root.
    ///
    /// Both the requested ancestry and an existing target's resolved ancestry
    /// are inspected. The latter catches a file symlink that points into an
    /// owned page directory. File publishers must still stage and rename their
    /// final entry so a hard-link alias cannot mutate the other link in place.
    pub(crate) fn ensure_peer_output_allowed(
        &self,
        requested: &Path,
        kind: OutputTargetKind,
    ) -> Result<(), OutputOwnershipError> {
        self.ensure_output_allowed(requested, kind, false)
    }

    fn ensure_output_allowed(
        &self,
        requested: &Path,
        kind: OutputTargetKind,
        allow_reserved_final_name: bool,
    ) -> Result<(), OutputOwnershipError> {
        let reserved_check_path = if allow_reserved_final_name {
            requested.parent().unwrap_or_else(|| Path::new(""))
        } else {
            requested
        };
        ensure_output_path_components_are_not_reserved(reserved_check_path)?;
        if let Some(root) = managed_html_root_for_entrypoint(requested)? {
            return Err(OutputOwnershipError::OwnedHtmlAncestor {
                path: requested.to_path_buf(),
                owner: root,
            });
        }
        if kind == OutputTargetKind::File
            && output_entry_resolves_to(requested, &self._lock_path, &self.file)?
        {
            return Err(OutputOwnershipError::InvalidOutputTarget {
                path: requested.to_path_buf(),
                reason: "the path is reserved for zpres output namespace serialization".to_string(),
            });
        }
        let starts = resolved_output_ancestries(requested, kind)?;
        for start in starts {
            if let Some(owner) = find_marker_ancestor(&start, RASTER_OWNERSHIP_MARKER_FILE)? {
                return Err(OutputOwnershipError::OwnedRasterAncestor {
                    path: requested.to_path_buf(),
                    owner,
                });
            }
            for marker in HTML_MANAGED_MARKERS {
                if let Some(owner) = find_marker_ancestor(&start, marker)? {
                    return Err(OutputOwnershipError::OwnedHtmlAncestor {
                        path: requested.to_path_buf(),
                        owner,
                    });
                }
            }
        }
        Ok(())
    }

    /// Refuse recursively deleting a tree that contains a raster-owned root.
    /// Symlinked directories are not followed because removing the link cannot
    /// remove the target tree.
    pub(crate) fn ensure_tree_removal_allowed(
        &self,
        requested: &Path,
    ) -> Result<(), OutputOwnershipError> {
        self.ensure_peer_output_allowed(requested, OutputTargetKind::Directory)?;
        let absolute = absolute_path(requested)?;
        let metadata = match fs::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(OutputOwnershipError::Io {
                    operation: "inspect recursively replaced Output tree",
                    path: absolute,
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(());
        }
        let mut pending = vec![absolute];
        while let Some(directory) = pending.pop() {
            if marker_entry_exists(&directory.join(RASTER_OWNERSHIP_MARKER_FILE))? {
                return Err(OutputOwnershipError::OwnedRasterAncestor {
                    path: requested.to_path_buf(),
                    owner: directory,
                });
            }
            for marker in HTML_MANAGED_MARKERS {
                if marker_entry_exists(&directory.join(marker))? {
                    return Err(OutputOwnershipError::OwnedHtmlAncestor {
                        path: requested.to_path_buf(),
                        owner: directory,
                    });
                }
            }
            let entries = fs::read_dir(&directory).map_err(|source| OutputOwnershipError::Io {
                operation: "scan recursively replaced Output tree",
                path: directory.clone(),
                source,
            })?;
            for entry in entries {
                let entry = entry.map_err(|source| OutputOwnershipError::Io {
                    operation: "scan recursively replaced Output tree",
                    path: directory.clone(),
                    source,
                })?;
                let file_type = entry
                    .file_type()
                    .map_err(|source| OutputOwnershipError::Io {
                        operation: "inspect recursively replaced Output-tree entry",
                        path: entry.path(),
                        source,
                    })?;
                if file_type.is_dir() && !file_type.is_symlink() {
                    pending.push(entry.path());
                }
            }
        }
        Ok(())
    }

    /// Refuse nesting a raster page set below another raster page set or
    /// anywhere in the private HTML generation tree. An existing raster marker
    /// at the exact requested directory is allowed because it is the previous
    /// generation being replaced.
    pub(crate) fn ensure_raster_destination_allowed(
        &self,
        requested: &Path,
    ) -> Result<(), OutputOwnershipError> {
        ensure_output_path_components_are_not_reserved(requested)?;
        if let Some(root) = managed_html_root_for_entrypoint(requested)? {
            return Err(OutputOwnershipError::OwnedHtmlAncestor {
                path: requested.to_path_buf(),
                owner: root,
            });
        }
        let absolute = absolute_path(requested)?;
        let exact = match fs::symlink_metadata(&absolute) {
            Ok(_) => fs::canonicalize(&absolute).ok(),
            Err(source) if source.kind() == io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(OutputOwnershipError::Io {
                    operation: "inspect raster destination ancestry",
                    path: absolute,
                    source,
                });
            }
        };
        let start = match exact.as_ref() {
            Some(path) => path.clone(),
            None => resolve_deepest_existing(&absolute)?,
        };

        let mut ancestor = start;
        loop {
            let is_exact = exact.as_ref().is_some_and(|path| path == &ancestor);
            if !is_exact && marker_entry_exists(&ancestor.join(RASTER_OWNERSHIP_MARKER_FILE))? {
                return Err(OutputOwnershipError::OwnedRasterAncestor {
                    path: requested.to_path_buf(),
                    owner: ancestor,
                });
            }
            for marker in HTML_MANAGED_MARKERS {
                if marker_entry_exists(&ancestor.join(marker))? {
                    return Err(OutputOwnershipError::OwnedHtmlAncestor {
                        path: requested.to_path_buf(),
                        owner: ancestor,
                    });
                }
            }
            if !ancestor.pop() {
                return Ok(());
            }
        }
    }

    /// Atomically replace one peer file while the namespace is held.
    pub(crate) fn publish_file(
        &self,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<OutputFilePublication, OutputOwnershipError> {
        self.publish_file_impl(destination, bytes, false)
    }

    /// Publish a renderer-owned file whose final `.zpres-*` basename is
    /// reserved from caller-selected targets. Reserved parent components remain
    /// forbidden.
    pub(crate) fn publish_reserved_file(
        &self,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<OutputFilePublication, OutputOwnershipError> {
        self.publish_file_impl(destination, bytes, true)
    }

    fn publish_file_impl(
        &self,
        destination: &Path,
        bytes: &[u8],
        allow_reserved_final_name: bool,
    ) -> Result<OutputFilePublication, OutputOwnershipError> {
        self.ensure_output_allowed(
            destination,
            OutputTargetKind::File,
            allow_reserved_final_name,
        )?;
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| OutputOwnershipError::Io {
            operation: "create Output target parent",
            path: parent.to_path_buf(),
            source,
        })?;
        self.ensure_output_allowed(
            destination,
            OutputTargetKind::File,
            allow_reserved_final_name,
        )?;
        let canonical_parent =
            fs::canonicalize(parent).map_err(|source| OutputOwnershipError::Io {
                operation: "resolve Output target parent",
                path: parent.to_path_buf(),
                source,
            })?;
        let name =
            destination
                .file_name()
                .ok_or_else(|| OutputOwnershipError::InvalidOutputTarget {
                    path: destination.to_path_buf(),
                    reason: "the Output target has no file name".to_string(),
                })?;
        let canonical_destination = canonical_parent.join(name);
        let resolves_to_namespace_lock = fs::canonicalize(&canonical_destination)
            .ok()
            .is_some_and(|path| path == self._lock_path);
        if canonical_destination == self._lock_path || resolves_to_namespace_lock {
            return Err(OutputOwnershipError::InvalidOutputTarget {
                path: destination.to_path_buf(),
                reason: "the path is reserved for zpres output namespace serialization".to_string(),
            });
        }
        let mut stage = FileStage::allocate(&canonical_parent)?;
        stage.write(bytes)?;
        stage.publish(&canonical_destination)?;
        let warnings = sync_directory(&canonical_parent)
            .err()
            .map(|source| {
                format!(
                    "published '{}' but could not sync its parent directory for power-loss durability: {source}",
                    destination.display()
                )
            })
            .into_iter()
            .collect();
        Ok(OutputFilePublication { warnings })
    }

    #[cfg(test)]
    pub(crate) fn lock_path(&self) -> &Path {
        &self._lock_path
    }
}

impl Drop for OutputNamespaceGuard {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

struct FileStage {
    path: Option<PathBuf>,
    file: Option<File>,
}

impl FileStage {
    fn allocate(parent: &Path) -> Result<Self, OutputOwnershipError> {
        for _ in 0..UNIQUE_ATTEMPTS {
            let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                "{FILE_STAGE_PREFIX}{}-{sequence:016x}",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path: Some(path),
                        file: Some(file),
                    });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(OutputOwnershipError::Io {
                        operation: "stage Output target",
                        path,
                        source,
                    });
                }
            }
        }
        Err(OutputOwnershipError::Io {
            operation: "stage Output target",
            path: parent.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "all unique Output-target stage names were occupied",
            ),
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), OutputOwnershipError> {
        let path = self.path.as_ref().expect("file stage is armed").clone();
        let file = self.file.as_mut().expect("file stage owns its file");
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| OutputOwnershipError::Io {
                operation: "write staged Output target",
                path,
                source,
            })
    }

    fn publish(&mut self, destination: &Path) -> Result<(), OutputOwnershipError> {
        self.file.take();
        let path = self.path.as_ref().expect("file stage is armed");
        fs::rename(path, destination).map_err(|source| OutputOwnershipError::Io {
            operation: "publish staged Output target",
            path: destination.to_path_buf(),
            source,
        })?;
        self.path.take();
        Ok(())
    }
}

impl Drop for FileStage {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn namespace_lock_path() -> Result<PathBuf, OutputOwnershipError> {
    let requested_root = PathBuf::from("/tmp");
    let root = fs::canonicalize(&requested_root).map_err(|source| OutputOwnershipError::Io {
        operation: "resolve fixed zpres output namespace lock directory",
        path: requested_root,
        source,
    })?;
    let metadata = fs::symlink_metadata(&root).map_err(|source| OutputOwnershipError::Io {
        operation: "inspect zpres output namespace lock directory",
        path: root.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OutputOwnershipError::InvalidNamespaceLock {
            path: root,
            reason: "the fixed host lock root is not a directory".to_string(),
        });
    }
    #[cfg(unix)]
    let user = unsafe { libc::geteuid() };
    #[cfg(not(unix))]
    let user = 0;
    Ok(root.join(format!(".{NAMESPACE_LOCK_FILE}-{user}")))
}

#[cfg(unix)]
fn open_namespace_lock(path: &Path) -> Result<File, OutputOwnershipError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| OutputOwnershipError::Io {
            operation: "open zpres output namespace lock",
            path: path.to_path_buf(),
            source,
        })?;
    let metadata = file.metadata().map_err(|source| OutputOwnershipError::Io {
        operation: "inspect zpres output namespace lock",
        path: path.to_path_buf(),
        source,
    })?;
    let private_mode = metadata.permissions().mode() & 0o077 == 0;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || !private_mode
    {
        return Err(OutputOwnershipError::InvalidNamespaceLock {
            path: path.to_path_buf(),
            reason:
                "the lock must be a private, single-link regular file owned by the current user"
                    .to_string(),
        });
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_namespace_lock(path: &Path) -> Result<File, OutputOwnershipError> {
    Err(OutputOwnershipError::UnsupportedPlatform {
        path: path.to_path_buf(),
    })
}

fn absolute_path(path: &Path) -> Result<PathBuf, OutputOwnershipError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|source| OutputOwnershipError::Io {
                operation: "resolve current directory for Output target",
                path: path.to_path_buf(),
                source,
            })
    }
}

fn resolved_output_ancestries(
    requested: &Path,
    kind: OutputTargetKind,
) -> Result<Vec<PathBuf>, OutputOwnershipError> {
    let absolute = absolute_path(requested)?;
    let mut starts = Vec::new();
    let lexical_parent = match kind {
        OutputTargetKind::File => absolute.parent().unwrap_or(Path::new("/")),
        OutputTargetKind::Directory => absolute.as_path(),
    };
    starts.push(resolve_deepest_existing(lexical_parent)?);

    if fs::symlink_metadata(&absolute).is_ok() {
        let resolved = fs::canonicalize(&absolute).map_err(|source| OutputOwnershipError::Io {
            operation: "resolve existing Output target",
            path: absolute.clone(),
            source,
        })?;
        let metadata = fs::metadata(&resolved).map_err(|source| OutputOwnershipError::Io {
            operation: "inspect resolved Output target",
            path: resolved.clone(),
            source,
        })?;
        let resolved_start = if kind == OutputTargetKind::File && !metadata.is_dir() {
            resolved.parent().unwrap_or(Path::new("/")).to_path_buf()
        } else {
            resolved
        };
        if !starts.contains(&resolved_start) {
            starts.push(resolved_start);
        }
    }
    Ok(starts)
}

fn output_entry_resolves_to(
    requested: &Path,
    reserved: &Path,
    reserved_file: &File,
) -> Result<bool, OutputOwnershipError> {
    let absolute = absolute_path(requested)?;
    #[cfg(unix)]
    if let (Ok(requested_metadata), Ok(reserved_metadata)) =
        (fs::metadata(&absolute), reserved_file.metadata())
    {
        use std::os::unix::fs::MetadataExt;
        if (requested_metadata.dev(), requested_metadata.ino())
            == (reserved_metadata.dev(), reserved_metadata.ino())
        {
            return Ok(true);
        }
    }
    if fs::canonicalize(&absolute)
        .ok()
        .is_some_and(|path| path == reserved)
    {
        return Ok(true);
    }
    let parent = absolute
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Some(name) = absolute.file_name() else {
        return Ok(false);
    };
    let canonical_parent = resolve_deepest_existing(parent)?;
    Ok(canonical_parent.join(name) == reserved)
}

fn is_reserved_output_metadata_name(name: Option<&std::ffi::OsStr>) -> bool {
    let Some(name) = name.and_then(std::ffi::OsStr::to_str) else {
        return false;
    };
    name.to_ascii_lowercase().starts_with(".zpres-")
}

fn ensure_output_path_components_are_not_reserved(
    requested: &Path,
) -> Result<(), OutputOwnershipError> {
    for component in requested.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        if is_reserved_output_metadata_name(Some(name)) {
            return Err(OutputOwnershipError::InvalidOutputTarget {
                path: requested.to_path_buf(),
                reason: format!(
                    "path component '{}' is reserved for zpres publication metadata",
                    name.to_string_lossy()
                ),
            });
        }
    }
    Ok(())
}

fn managed_html_root_for_entrypoint(
    requested: &Path,
) -> Result<Option<PathBuf>, OutputOwnershipError> {
    let absolute = absolute_path(requested)?;
    for candidate in absolute.ancestors() {
        let Some(name) = candidate.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if !name.eq_ignore_ascii_case("index.html") {
            continue;
        }
        let Some(parent) = candidate.parent() else {
            continue;
        };
        let root = match fs::canonicalize(parent) {
            Ok(root) => root,
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(source) => {
                return Err(OutputOwnershipError::Io {
                    operation: "resolve possible zpres HTML output root",
                    path: parent.to_path_buf(),
                    source,
                });
            }
        };
        let marker = root
            .join("zpres-html-generations")
            .join(".zpres-output.json");
        if marker_entry_exists(&marker)? {
            return Ok(Some(root));
        }
    }
    Ok(None)
}

fn resolve_deepest_existing(path: &Path) -> Result<PathBuf, OutputOwnershipError> {
    let mut candidate = absolute_path(path)?;
    loop {
        match fs::canonicalize(&candidate) {
            Ok(path) => return Ok(path),
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                if !candidate.pop() {
                    return Err(OutputOwnershipError::Io {
                        operation: "resolve existing Output target ancestry",
                        path: path.to_path_buf(),
                        source,
                    });
                }
            }
            Err(source) => {
                return Err(OutputOwnershipError::Io {
                    operation: "resolve existing Output target ancestry",
                    path: candidate,
                    source,
                });
            }
        }
    }
}

fn find_marker_ancestor(
    start: &Path,
    marker: &str,
) -> Result<Option<PathBuf>, OutputOwnershipError> {
    let mut ancestor = start.to_path_buf();
    loop {
        if marker_entry_exists(&ancestor.join(marker))? {
            return Ok(Some(ancestor));
        }
        if !ancestor.pop() {
            return Ok(None);
        }
    }
}

fn marker_entry_exists(path: &Path) -> Result<bool, OutputOwnershipError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source)
            if matches!(
                source.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(source) => Err(OutputOwnershipError::Io {
            operation: "inspect Output ownership marker",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn lock_file_blocking(file: &File, path: &Path) -> Result<(), OutputOwnershipError> {
    native_fs::lock_file_blocking(file).map_err(|source| OutputOwnershipError::Io {
        operation: "lock zpres output namespace",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn lock_file_blocking(_file: &File, path: &Path) -> Result<(), OutputOwnershipError> {
    Err(OutputOwnershipError::UnsupportedPlatform {
        path: path.to_path_buf(),
    })
}

fn ensure_supported_platform(path: &Path) -> Result<(), OutputOwnershipError> {
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        Ok(())
    } else {
        Err(OutputOwnershipError::UnsupportedPlatform {
            path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn every_path_for_the_user_uses_the_same_namespace_lock() {
        let first = tempdir().unwrap();
        let second = tempdir().unwrap();
        let first_guard = OutputNamespaceGuard::acquire(first.path()).unwrap();
        let first_path = first_guard.lock_path().to_path_buf();
        drop(first_guard);
        let second_guard = OutputNamespaceGuard::acquire(second.path()).unwrap();
        assert_eq!(first_path, second_guard.lock_path());
    }

    #[test]
    fn namespace_lock_serializes_actual_contenders() {
        let temp = tempdir().unwrap();
        let first = OutputNamespaceGuard::acquire(temp.path()).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let requested = temp.path().join("other");
        let contender = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let guard = OutputNamespaceGuard::acquire(&requested).unwrap();
            acquired_tx.send(()).unwrap();
            drop(guard);
        });

        started_rx.recv().unwrap();
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        contender.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn peer_guard_resolves_symlinked_raster_ancestry() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let pages = temp.path().join("pages");
        let alias = temp.path().join("alias");
        fs::create_dir(&pages).unwrap();
        fs::write(pages.join(RASTER_OWNERSHIP_MARKER_FILE), b"owned").unwrap();
        symlink(&pages, &alias).unwrap();
        let guard = OutputNamespaceGuard::acquire(&alias).unwrap();

        let error = guard
            .ensure_peer_output_allowed(
                &alias.join("nested").join("notes.txt"),
                OutputTargetKind::File,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            OutputOwnershipError::OwnedRasterAncestor { .. }
        ));
    }

    #[test]
    fn peer_guard_rejects_missing_descendants_of_a_raster_root() {
        let temp = tempdir().unwrap();
        let pages = temp.path().join("pages");
        fs::create_dir(&pages).unwrap();
        fs::write(pages.join(RASTER_OWNERSHIP_MARKER_FILE), b"owned").unwrap();
        let requested = pages.join("missing").join("notes.txt");
        let guard = OutputNamespaceGuard::acquire(&requested).unwrap();

        assert!(matches!(
            guard.ensure_peer_output_allowed(&requested, OutputTargetKind::File),
            Err(OutputOwnershipError::OwnedRasterAncestor { .. })
        ));
        assert!(!pages.join("missing").exists());
    }

    #[test]
    fn an_invalid_marker_entry_still_reserves_the_raster_root() {
        let temp = tempdir().unwrap();
        let pages = temp.path().join("pages");
        fs::create_dir_all(pages.join(RASTER_OWNERSHIP_MARKER_FILE)).unwrap();
        let guard = OutputNamespaceGuard::acquire(&pages).unwrap();

        assert!(matches!(
            guard.ensure_peer_output_allowed(&pages, OutputTargetKind::Directory),
            Err(OutputOwnershipError::OwnedRasterAncestor { .. })
        ));
    }

    #[test]
    fn nonoverlapping_peer_output_is_allowed() {
        let temp = tempdir().unwrap();
        let pages = temp.path().join("pages");
        let output = temp.path().join("review").join("notes.txt");
        fs::create_dir(&pages).unwrap();
        fs::write(pages.join(RASTER_OWNERSHIP_MARKER_FILE), b"owned").unwrap();
        let guard = OutputNamespaceGuard::acquire(&output).unwrap();

        guard
            .ensure_peer_output_allowed(&output, OutputTargetKind::File)
            .unwrap();
    }

    #[test]
    fn atomic_peer_file_publish_does_not_mutate_a_hard_link_peer() {
        let temp = tempdir().unwrap();
        let original = temp.path().join("original.txt");
        let output = temp.path().join("output.txt");
        fs::write(&original, b"original").unwrap();
        fs::hard_link(&original, &output).unwrap();
        let guard = OutputNamespaceGuard::acquire(&output).unwrap();

        let publication = guard.publish_file(&output, b"replacement").unwrap();

        assert!(publication.warnings.is_empty());
        assert_eq!(fs::read(&original).unwrap(), b"original");
        assert_eq!(fs::read(&output).unwrap(), b"replacement");
    }

    #[test]
    fn peer_file_cannot_replace_the_namespace_lock_path() {
        let temp = tempdir().unwrap();
        let guard = OutputNamespaceGuard::acquire(temp.path()).unwrap();
        let lock_path = guard.lock_path().to_path_buf();
        let before = fs::metadata(&lock_path).unwrap();

        assert!(matches!(
            guard.publish_file(&lock_path, b"replacement"),
            Err(OutputOwnershipError::InvalidOutputTarget { .. })
        ));
        let after = fs::metadata(&lock_path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn peer_file_cannot_replace_the_namespace_lock_through_a_firmlink_alias() {
        let temp = tempdir().unwrap();
        let guard = OutputNamespaceGuard::acquire(temp.path()).unwrap();
        let lock_path = guard.lock_path().to_path_buf();
        let alias = Path::new("/System/Volumes/Data").join(
            lock_path
                .strip_prefix(Path::new("/"))
                .expect("the namespace lock path is absolute"),
        );
        let Ok(lock_metadata) = fs::metadata(&lock_path) else {
            return;
        };
        let Ok(alias_metadata) = fs::metadata(&alias) else {
            return;
        };
        use std::os::unix::fs::MetadataExt;
        if (lock_metadata.dev(), lock_metadata.ino())
            != (alias_metadata.dev(), alias_metadata.ino())
        {
            return;
        }

        assert!(matches!(
            guard.publish_file(&alias, b"replacement"),
            Err(OutputOwnershipError::InvalidOutputTarget { .. })
        ));
        let after = fs::metadata(&lock_path).unwrap();
        assert_eq!(
            (lock_metadata.dev(), lock_metadata.ino()),
            (after.dev(), after.ino())
        );
    }

    #[test]
    fn reserved_metadata_in_any_output_path_component_is_refused_before_creation() {
        let temp = tempdir().unwrap();
        let poisoned_parent = temp.path().join(RASTER_OWNERSHIP_MARKER_FILE);
        let peer = poisoned_parent.join("notes.txt");
        let raster_lock = temp
            .path()
            .join(".zpres-raster-publish-0123456789abcdef01234567.lock")
            .join("pages");
        let guard = OutputNamespaceGuard::acquire(&peer).unwrap();

        assert!(matches!(
            guard.ensure_peer_output_allowed(&peer, OutputTargetKind::File),
            Err(OutputOwnershipError::InvalidOutputTarget { .. })
        ));
        assert!(matches!(
            guard.ensure_peer_output_allowed(&poisoned_parent, OutputTargetKind::Directory),
            Err(OutputOwnershipError::InvalidOutputTarget { .. })
        ));
        assert!(matches!(
            guard.ensure_raster_destination_allowed(&raster_lock),
            Err(OutputOwnershipError::InvalidOutputTarget { .. })
        ));
        assert!(!poisoned_parent.exists());
        assert!(!raster_lock.exists());
    }

    #[test]
    fn peer_outputs_preserve_html_generation_metadata_and_root_entrypoint() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("html");
        let generations = root.join("zpres-html-generations");
        let generation = generations.join("g-1");
        fs::create_dir_all(&generation).unwrap();
        fs::write(generations.join(".zpres-output.json"), b"owned root").unwrap();
        fs::write(
            generation.join(".zpres-generation.json"),
            b"owned generation",
        )
        .unwrap();
        fs::write(root.join("index.html"), b"pointer").unwrap();
        let guard = OutputNamespaceGuard::acquire(&root).unwrap();

        for output in [
            root.join("index.html"),
            generations.join("peer.txt"),
            generation.join("assets").join("peer.txt"),
        ] {
            assert!(matches!(
                guard.ensure_peer_output_allowed(&output, OutputTargetKind::File),
                Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
            ));
        }
        assert!(matches!(
            guard.ensure_peer_output_allowed(&root.join("index.html"), OutputTargetKind::Directory),
            Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
        ));
        assert!(matches!(
            guard.ensure_raster_destination_allowed(&root.join("index.html")),
            Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
        ));
        guard
            .ensure_peer_output_allowed(&root.join("deck.pdf"), OutputTargetKind::File)
            .unwrap();
    }

    #[test]
    fn missing_html_entrypoint_is_reserved_with_all_of_its_descendants() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("html");
        let generations = root.join("zpres-html-generations");
        fs::create_dir_all(&generations).unwrap();
        fs::write(generations.join(".zpres-output.json"), b"owned root").unwrap();
        let descendant = root.join("INDEX.HTML").join("notes.txt");
        let guard = OutputNamespaceGuard::acquire(&descendant).unwrap();

        assert!(matches!(
            guard.ensure_peer_output_allowed(&descendant, OutputTargetKind::File),
            Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
        ));
        assert!(matches!(
            guard.ensure_raster_destination_allowed(&root.join("index.html").join("pages")),
            Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
        ));
        assert!(matches!(
            guard.ensure_peer_output_allowed(
                &root.join("index.html").join("..").join("deck.pdf"),
                OutputTargetKind::File,
            ),
            Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
        ));
        assert!(!root.join("index.html").exists());
    }

    #[test]
    fn raster_guard_rejects_html_generation_trees_but_not_normal_html_roots() {
        let temp = tempdir().unwrap();
        let html_root = temp.path().join("html");
        let generations = html_root.join("zpres-html-generations");
        fs::create_dir_all(&generations).unwrap();
        fs::write(generations.join(".zpres-output.json"), b"owned").unwrap();
        let guard = OutputNamespaceGuard::acquire(&html_root).unwrap();

        guard
            .ensure_raster_destination_allowed(&html_root.join("pages"))
            .unwrap();
        assert!(matches!(
            guard.ensure_raster_destination_allowed(&generations.join("pages")),
            Err(OutputOwnershipError::OwnedHtmlAncestor { .. })
        ));
    }

    #[test]
    fn recursive_tree_replacement_refuses_a_nested_raster_root() {
        let temp = tempdir().unwrap();
        let tree = temp.path().join("pages");
        let nested = tree.join("archive");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(RASTER_OWNERSHIP_MARKER_FILE), b"owned").unwrap();
        fs::write(nested.join("page-001.png"), b"preserved").unwrap();
        let guard = OutputNamespaceGuard::acquire(&tree).unwrap();

        assert!(matches!(
            guard.ensure_tree_removal_allowed(&tree),
            Err(OutputOwnershipError::OwnedRasterAncestor { owner, .. }) if owner == nested
        ));
        assert_eq!(fs::read(nested.join("page-001.png")).unwrap(), b"preserved");
    }

    #[test]
    fn recursive_tree_replacement_refuses_a_nested_html_publication() {
        let temp = tempdir().unwrap();
        let tree = temp.path().join("pages");
        let nested = tree.join("archive").join("zpres-html-generations");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(".zpres-output.json"), b"owned").unwrap();
        fs::write(nested.join("generation"), b"preserved").unwrap();
        let guard = OutputNamespaceGuard::acquire(&tree).unwrap();

        assert!(matches!(
            guard.ensure_tree_removal_allowed(&tree),
            Err(OutputOwnershipError::OwnedHtmlAncestor { owner, .. }) if owner == nested
        ));
        assert_eq!(fs::read(nested.join("generation")).unwrap(), b"preserved");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn recursive_tree_replacement_resolves_apfs_case_variant_marker_names() {
        let temp = tempdir().unwrap();
        let tree = temp.path().join("pages");
        let nested = tree.join("archive");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(".ZPRES-RASTER-PAGE-SET.JSON"), b"owned").unwrap();
        if !nested.join(RASTER_OWNERSHIP_MARKER_FILE).exists() {
            return;
        }
        let guard = OutputNamespaceGuard::acquire(&tree).unwrap();

        assert!(matches!(
            guard.ensure_tree_removal_allowed(&tree),
            Err(OutputOwnershipError::OwnedRasterAncestor { owner, .. }) if owner == nested
        ));
    }
}
