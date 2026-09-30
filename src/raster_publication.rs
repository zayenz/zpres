use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::native_fs::{self, RenameError, unlock_file};

use crate::output_ownership::{
    OutputNamespaceGuard, OutputOwnershipError, RASTER_OWNERSHIP_MARKER_FILE,
};

const OWNERSHIP_MARKER_FILE: &str = RASTER_OWNERSHIP_MARKER_FILE;
const GENERATOR: &str = "zpres";
const MARKER_KIND: &str = "raster-page-set";
const SCHEMA: u32 = 1;
const LOCK_PREFIX: &str = ".zpres-raster-publish-";
const LOCK_STAGE_PREFIX: &str = ".zpres-raster-lock-stage-";
const STAGE_PREFIX: &str = ".zpres-raster-stage-";
const UNIQUE_ATTEMPTS: usize = 128;
const MAX_MARKER_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LOCK_BYTES: u64 = 4 * 1024;

static UNIQUE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RasterFormat {
    Png,
    Jpeg,
}

impl RasterFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    fn marker_value(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RasterPublication {
    pub(crate) files: Vec<PathBuf>,
    pub(crate) generation: String,
    pub(crate) warnings: Vec<String>,
}

pub(crate) struct RasterReadGuard {
    output_dir: PathBuf,
    files: Vec<PathBuf>,
    _writer_lock: WriterLock,
}

impl RasterReadGuard {
    pub(crate) fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub(crate) fn files(&self) -> &[PathBuf] {
        &self.files
    }
}

#[derive(Debug, Error)]
pub(crate) enum RasterPublicationError {
    #[error(transparent)]
    OutputOwnership(#[from] OutputOwnershipError),

    #[error("cannot publish raster page set at '{}': {reason}", path.display())]
    InvalidDestination { path: PathBuf, reason: String },

    #[error(
        "refusing to replace unowned raster destination '{}': {reason}. Move or remove that directory, then export again",
        path.display()
    )]
    UnownedDestination { path: PathBuf, reason: String },

    #[error(
        "another zpres raster publication already holds the writer lock for '{}'",
        path.display()
    )]
    ConcurrentPublication { path: PathBuf },

    #[error(
        "raster destination '{}' changed while the complete page set was staged; the newer destination was preserved",
        path.display()
    )]
    DestinationChanged { path: PathBuf },

    #[error(
        "raster publication at '{}' failed before commit ({publication_error}) and restoring the previously empty directory also failed ({rollback_error})",
        path.display()
    )]
    AdoptionRollbackFailed {
        path: PathBuf,
        publication_error: Box<RasterPublicationError>,
        rollback_error: Box<RasterPublicationError>,
    },

    #[error(
        "atomic raster-directory publication is unsupported for '{}': {source}; the previous page set was preserved",
        path.display()
    )]
    AtomicPublicationUnsupported {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[cfg(test)]
    #[error(
        "injected process crash left raster publication state at '{}'; no rollback was attempted",
        path.display()
    )]
    InjectedCrash { path: PathBuf },

    #[error("injected returned raster publication failure at '{}'", path.display())]
    InjectedReturnedFailure { path: PathBuf },
}

/// Atomically publish one complete PNG or JPEG page directory.
///
/// The requested directory becomes exclusively zpres-owned. A missing or
/// truly empty regular directory can be claimed; every non-empty unmarked
/// directory is preserved and refused. On macOS and Linux the namespace commit
/// is one native no-replace rename or directory exchange.
#[cfg(test)]
pub(crate) fn publish_raster_page_set(
    output_dir: &Path,
    format: RasterFormat,
    pages: Vec<Vec<u8>>,
) -> Result<RasterPublication, RasterPublicationError> {
    let namespace = OutputNamespaceGuard::acquire(output_dir)?;
    publish_raster_page_set_under_namespace(&namespace, output_dir, format, pages)
}

pub(crate) fn publish_raster_page_set_under_namespace(
    namespace: &OutputNamespaceGuard,
    output_dir: &Path,
    format: RasterFormat,
    pages: Vec<Vec<u8>>,
) -> Result<RasterPublication, RasterPublicationError> {
    ensure_supported_platform(output_dir)?;
    namespace.ensure_raster_destination_allowed(output_dir)?;
    if pages.is_empty() {
        return Err(RasterPublicationError::InvalidDestination {
            path: output_dir.to_path_buf(),
            reason: "a raster page set must contain at least one page".to_string(),
        });
    }

    let destination = Destination::prepare(output_dir)?;
    let mut writer_lock = WriterLock::acquire(&destination)?;
    let mut warnings = writer_lock.take_warnings();
    recover_interrupted_empty_adoption(&destination, format)?;
    warnings.extend(recover_owned_stages(&destination, format)?);
    let initial = inspect_destination(&destination, format)?;
    let generation = new_generation_id();
    let marker = OwnershipMarker::new(format, &destination.key, generation.clone(), &pages);

    let mut stage = RasterStage::allocate(&destination, marker.clone())?;
    write_staged_pages(stage.path(), format, &pages)?;
    validate_complete_owned_set(
        stage.path(),
        format,
        &destination.key,
        Some(&generation),
        Some(pages.len()),
    )?;
    sync_directory(stage.path()).map_err(|source| RasterPublicationError::Io {
        operation: "sync staged raster page directory",
        path: stage.path().to_path_buf(),
        source,
    })?;

    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::LeakPopulatedStage) {
        let path = stage.disarm();
        return Err(RasterPublicationError::InjectedCrash { path });
    }

    let current = inspect_destination(&destination, format)?;
    if current != initial {
        return Err(RasterPublicationError::DestinationChanged {
            path: destination.path.clone(),
        });
    }

    let mut adoption = match &initial {
        DestinationState::Empty(empty) => Some(AdoptionGuard::claim(&destination, format, empty)?),
        DestinationState::Missing | DestinationState::Owned(_) => None,
    };

    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::AfterEmptyAdoptionBeforeExchange) {
        if let Some(adoption) = adoption.as_mut() {
            adoption.disarm();
        }
        let path = stage.disarm();
        return Err(RasterPublicationError::InjectedCrash { path });
    }

    #[cfg(test)]
    let injected_commit_error = matches!(
        TEST_FAILPOINT.with(|active| active.get()),
        Some(TestFailpoint::BeforeExchangeReturnedError)
            | Some(TestFailpoint::BeforeExchangeAndRollbackFailure)
    );
    #[cfg(not(test))]
    let injected_commit_error = false;
    let commit = if injected_commit_error {
        Err(RasterPublicationError::InjectedReturnedFailure {
            path: destination.path.clone(),
        })
    } else {
        match initial {
            DestinationState::Missing => rename_noreplace(stage.path(), &destination.path),
            DestinationState::Empty(_) | DestinationState::Owned(_) => {
                exchange_paths(stage.path(), &destination.path).inspect(|()| {
                    stage.mark_displaced_owned_directory();
                })
            }
        }
    };
    if let Err(error) = commit {
        if let Some(adoption) = adoption.as_mut() {
            return Err(adoption.rollback_or_combine(error));
        }
        return Err(error);
    }
    if let Some(adoption) = adoption.as_mut() {
        adoption.disarm();
    }

    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::AfterExchangeBeforeParentSync) {
        let path = stage.disarm();
        return Err(RasterPublicationError::InjectedCrash { path });
    }

    let commit_sync = sync_parent_after_commit(&destination.parent);
    if let Err(source) = &commit_sync {
        warnings.push(format!(
            "published '{}' but could not sync its parent directory for power-loss durability: {source}",
            destination.requested.display()
        ));
    }

    match initial {
        DestinationState::Missing => {
            stage.disarm();
        }
        DestinationState::Empty(_) | DestinationState::Owned(_) => {
            if commit_sync.is_err() {
                let path = stage.disarm();
                warnings.push(format!(
                    "retained the displaced owned page directory at '{}' because the raster-directory exchange was not durably synced",
                    path.display()
                ));
            } else if test_post_commit_cleanup_fails() {
                let path = stage.disarm();
                warnings.push(format!(
                    "published '{}' but retained the previous owned page directory at '{}' after an injected cleanup failure",
                    destination.requested.display(),
                    path.display()
                ));
            } else if let Err(error) = remove_owned_directory(
                stage.path(),
                format,
                &destination.key,
                CleanupInventory::Exact,
            ) {
                let path = stage.disarm();
                warnings.push(format!(
                    "published '{}' but could not remove the previous owned page directory '{}': {error}",
                    destination.requested.display(),
                    path.display()
                ));
            } else {
                stage.disarm();
                if let Err(source) = sync_directory(&destination.parent) {
                    warnings.push(format!(
                        "published '{}' but could not sync cleanup of its previous page directory: {source}",
                        destination.requested.display()
                    ));
                }
            }
        }
    }

    let files = (1..=pages.len())
        .map(|page| output_dir.join(format!("page-{page:03}.{}", format.extension())))
        .collect();
    Ok(RasterPublication {
        files,
        generation,
        warnings,
    })
}

/// Validate an existing raster destination without creating its parent, lock,
/// marker, or any capture scratch. Publication repeats the check under the
/// stable writer lock before it mutates the requested directory.
#[cfg(test)]
pub(crate) fn preflight_raster_page_set_destination(
    output_dir: &Path,
    format: RasterFormat,
) -> Result<(), RasterPublicationError> {
    let namespace = OutputNamespaceGuard::acquire(output_dir)?;
    preflight_raster_page_set_destination_under_namespace(&namespace, output_dir, format)
}

pub(crate) fn preflight_raster_page_set_destination_under_namespace(
    namespace: &OutputNamespaceGuard,
    output_dir: &Path,
    format: RasterFormat,
) -> Result<(), RasterPublicationError> {
    ensure_supported_platform(output_dir)?;
    namespace.ensure_raster_destination_allowed(output_dir)?;
    let (name, requested_parent) = validate_requested_path(output_dir)?;
    reject_owned_raster_ancestor(requested_parent, output_dir)?;
    let parent = match fs::canonicalize(requested_parent) {
        Ok(parent) => parent,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(RasterPublicationError::Io {
                operation: "resolve existing raster destination parent",
                path: requested_parent.to_path_buf(),
                source,
            });
        }
    };
    let metadata = fs::symlink_metadata(&parent).map_err(|source| RasterPublicationError::Io {
        operation: "inspect existing raster destination parent",
        path: parent.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(RasterPublicationError::InvalidDestination {
            path: output_dir.to_path_buf(),
            reason: "the existing output parent is not a directory".to_string(),
        });
    }
    let path = parent.join(name);
    let destination = Destination {
        requested: output_dir.to_path_buf(),
        parent,
        key: destination_key(&path),
        path,
    };
    if interrupted_empty_adoption_marker(&destination, format)?.is_some() {
        return Ok(());
    }
    inspect_destination(&destination, format).map(|_| ())
}

/// Pin one specific published page set while a cooperating consumer reads it.
///
/// The same stable sibling lock serializes writers and readers. If a rebuild
/// won the gap after the caller received its publication report, generation
/// validation fails rather than silently reading a different page set.
pub(crate) fn lock_raster_page_set_for_read_under_namespace(
    namespace: &OutputNamespaceGuard,
    output_dir: &Path,
    format: RasterFormat,
    generation: &str,
    expected_files: &[PathBuf],
) -> Result<RasterReadGuard, RasterPublicationError> {
    ensure_supported_platform(output_dir)?;
    namespace.ensure_raster_destination_allowed(output_dir)?;
    let destination = Destination::prepare(output_dir)?;
    let writer_lock = WriterLock::acquire(&destination)?;
    let state = inspect_destination(&destination, format)?;
    let DestinationState::Owned(evidence) = state else {
        return Err(RasterPublicationError::DestinationChanged {
            path: output_dir.to_path_buf(),
        });
    };
    if evidence.marker.generation != generation {
        return Err(RasterPublicationError::DestinationChanged {
            path: output_dir.to_path_buf(),
        });
    }
    let requested_files = evidence
        .marker
        .pages
        .iter()
        .map(|page| output_dir.join(&page.file))
        .collect::<Vec<_>>();
    if requested_files != expected_files {
        return Err(RasterPublicationError::InvalidDestination {
            path: output_dir.to_path_buf(),
            reason: "the requested reader inventory does not exactly match the published marker"
                .to_string(),
        });
    }
    let files = evidence
        .marker
        .pages
        .iter()
        .map(|page| destination.path.join(&page.file))
        .collect();
    Ok(RasterReadGuard {
        output_dir: destination.path,
        files,
        _writer_lock: writer_lock,
    })
}

#[derive(Debug, Clone)]
struct Destination {
    requested: PathBuf,
    parent: PathBuf,
    path: PathBuf,
    key: String,
}

impl Destination {
    fn prepare(requested: &Path) -> Result<Self, RasterPublicationError> {
        let (name, requested_parent) = validate_requested_path(requested)?;
        reject_owned_raster_ancestor(requested_parent, requested)?;
        fs::create_dir_all(requested_parent).map_err(|source| RasterPublicationError::Io {
            operation: "create raster destination parent",
            path: requested_parent.to_path_buf(),
            source,
        })?;
        let parent =
            fs::canonicalize(requested_parent).map_err(|source| RasterPublicationError::Io {
                operation: "resolve raster destination parent",
                path: requested_parent.to_path_buf(),
                source,
            })?;
        let path = parent.join(name);
        let key = destination_key(&path);
        Ok(Self {
            requested: requested.to_path_buf(),
            parent,
            path,
            key,
        })
    }

    fn stage_prefix(&self) -> String {
        format!("{STAGE_PREFIX}{}-", &self.key[..24])
    }

    fn lock_path(&self) -> PathBuf {
        self.parent
            .join(format!("{LOCK_PREFIX}{}.lock", &self.key[..24]))
    }
}

fn validate_requested_path(
    requested: &Path,
) -> Result<(&std::ffi::OsStr, &Path), RasterPublicationError> {
    let contains_parent = requested
        .components()
        .any(|component| component == std::path::Component::ParentDir);
    if contains_parent
        || raw_path_contains_current_directory(requested)
        || !matches!(
            requested.components().next_back(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(RasterPublicationError::InvalidDestination {
            path: requested.to_path_buf(),
            reason: "the output path must contain only normal directory components after its optional root; '.', '..', and filesystem-root targets are unsafe"
                .to_string(),
        });
    }
    let Some(name) = requested.file_name() else {
        return Err(RasterPublicationError::InvalidDestination {
            path: requested.to_path_buf(),
            reason: "the output path has no directory name".to_string(),
        });
    };
    let requested_parent = requested
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok((name, requested_parent))
}

fn reject_owned_raster_ancestor(
    requested_parent: &Path,
    requested: &Path,
) -> Result<(), RasterPublicationError> {
    let mut candidate = if requested_parent.is_absolute() {
        requested_parent.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| RasterPublicationError::Io {
                operation: "resolve the current directory for raster ancestry validation",
                path: requested_parent.to_path_buf(),
                source,
            })?
            .join(requested_parent)
    };
    let mut ancestor = loop {
        match fs::canonicalize(&candidate) {
            Ok(path) => break path,
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                if !candidate.pop() {
                    return Ok(());
                }
            }
            Err(source) => {
                return Err(RasterPublicationError::Io {
                    operation: "resolve raster destination ancestry",
                    path: candidate,
                    source,
                });
            }
        }
    };

    loop {
        let marker = ancestor.join(OWNERSHIP_MARKER_FILE);
        match fs::symlink_metadata(&marker) {
            Ok(_) => {
                return Err(RasterPublicationError::InvalidDestination {
                    path: requested.to_path_buf(),
                    reason: format!(
                        "the output would be nested inside exclusively owned raster directory '{}'",
                        ancestor.display()
                    ),
                });
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(RasterPublicationError::Io {
                    operation: "inspect raster destination ancestry",
                    path: marker,
                    source,
                });
            }
        }
        if !ancestor.pop() {
            return Ok(());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DestinationState {
    Missing,
    Empty(EmptyDirectoryEvidence),
    Owned(OwnedDirectoryEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EmptyDirectoryEvidence {
    identity: FileIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedDirectoryEvidence {
    identity: FileIdentity,
    marker: OwnershipMarker,
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity;

fn inspect_destination(
    destination: &Destination,
    format: RasterFormat,
) -> Result<DestinationState, RasterPublicationError> {
    let metadata = match fs::symlink_metadata(&destination.path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(DestinationState::Missing);
        }
        Err(source) => {
            return Err(RasterPublicationError::Io {
                operation: "inspect raster destination",
                path: destination.path.clone(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(RasterPublicationError::UnownedDestination {
            path: destination.requested.clone(),
            reason: "the requested path is a symbolic link".to_string(),
        });
    }
    if !metadata.is_dir() {
        return Err(RasterPublicationError::UnownedDestination {
            path: destination.requested.clone(),
            reason: "the requested path is not a directory".to_string(),
        });
    }

    let entries = read_directory_entries(&destination.path)?;
    if entries.is_empty() {
        return Ok(DestinationState::Empty(EmptyDirectoryEvidence {
            identity: file_identity(&metadata),
        }));
    }

    let marker_path = destination.path.join(OWNERSHIP_MARKER_FILE);
    if !entries.iter().any(|path| path == &marker_path) {
        return Err(RasterPublicationError::UnownedDestination {
            path: destination.requested.clone(),
            reason: format!(
                "the directory is non-empty and has no canonical {OWNERSHIP_MARKER_FILE} ownership marker"
            ),
        });
    }
    let marker = read_and_validate_marker(&marker_path, format, &destination.key, None)?;
    validate_complete_owned_set(
        &destination.path,
        format,
        &destination.key,
        Some(&marker.generation),
        None,
    )?;
    Ok(DestinationState::Owned(OwnedDirectoryEvidence {
        identity: file_identity(&metadata),
        marker,
    }))
}

fn read_directory_entries(path: &Path) -> Result<Vec<PathBuf>, RasterPublicationError> {
    let entries = fs::read_dir(path).map_err(|source| RasterPublicationError::Io {
        operation: "read raster destination",
        path: path.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RasterPublicationError::Io {
            operation: "read raster destination entry",
            path: path.to_path_buf(),
            source,
        })?;
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct OwnershipMarker {
    schema: u32,
    generator: String,
    kind: String,
    format: String,
    target: String,
    generation: String,
    page_count: usize,
    pages: Vec<PageInventoryEntry>,
}

impl OwnershipMarker {
    fn new(format: RasterFormat, target: &str, generation: String, pages: &[Vec<u8>]) -> Self {
        Self {
            schema: SCHEMA,
            generator: GENERATOR.to_string(),
            kind: MARKER_KIND.to_string(),
            format: format.marker_value().to_string(),
            target: target.to_string(),
            generation,
            page_count: pages.len(),
            pages: pages
                .iter()
                .enumerate()
                .map(|(index, bytes)| PageInventoryEntry {
                    file: format!("page-{:03}.{}", index + 1, format.extension()),
                    sha256: hex_bytes(&Sha256::digest(bytes)),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PageInventoryEntry {
    file: String,
    sha256: String,
}

fn render_marker(marker: &OwnershipMarker) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(marker).expect("ownership marker serializes");
    bytes.push(b'\n');
    bytes
}

fn read_and_validate_marker(
    path: &Path,
    format: RasterFormat,
    target: &str,
    generation: Option<&str>,
) -> Result<OwnershipMarker, RasterPublicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: format!("the ownership marker cannot be inspected: {source}"),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_MARKER_BYTES
    {
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: "the ownership marker is not a bounded regular file".to_string(),
        });
    }
    let bytes = fs::read(path).map_err(|source| RasterPublicationError::Io {
        operation: "read raster ownership marker",
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: "the ownership marker grew beyond the bounded size while it was read"
                .to_string(),
        });
    }
    let marker: OwnershipMarker = serde_json::from_slice(&bytes).map_err(|source| {
        RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: format!("the ownership marker is invalid JSON: {source}"),
        }
    })?;
    let valid_generation = !marker.generation.is_empty()
        && marker
            .generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    let valid_inventory = marker.page_count == marker.pages.len()
        && marker.pages.iter().enumerate().all(|(index, page)| {
            page.file == format!("page-{:03}.{}", index + 1, format.extension())
                && page.sha256.len() == 64
                && page
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        });
    if marker.schema != SCHEMA
        || marker.generator != GENERATOR
        || marker.kind != MARKER_KIND
        || marker.format != format.marker_value()
        || marker.target != target
        || render_marker(&marker) != bytes
        || !valid_generation
        || !valid_inventory
        || generation.is_some_and(|expected| marker.generation != expected)
    {
        let format_hint = if marker.format != format.marker_value() {
            format!(
                "; it belongs to the '{}' raster format, not '{}'",
                marker.format,
                format.marker_value()
            )
        } else {
            String::new()
        };
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: format!(
                "the ownership marker is not canonical for this destination{format_hint}"
            ),
        });
    }
    Ok(marker)
}

struct RasterStage {
    path: Option<PathBuf>,
    format: RasterFormat,
    target: String,
    cleanup: StageCleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageCleanup {
    PrivateStage,
    DisplacedOwnedDirectory,
}

impl RasterStage {
    fn allocate(
        destination: &Destination,
        marker: OwnershipMarker,
    ) -> Result<Self, RasterPublicationError> {
        for _ in 0..UNIQUE_ATTEMPTS {
            let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = destination.parent.join(format!(
                "{}{}-{sequence:016x}",
                destination.stage_prefix(),
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let mut stage = Self {
                        path: Some(path),
                        format: match marker.format.as_str() {
                            "png" => RasterFormat::Png,
                            "jpeg" => RasterFormat::Jpeg,
                            _ => unreachable!("marker format is constructed internally"),
                        },
                        target: marker.target.clone(),
                        cleanup: StageCleanup::PrivateStage,
                    };
                    if let Err(error) = write_new_synced_file(
                        &stage.path().join(OWNERSHIP_MARKER_FILE),
                        &render_marker(&marker),
                    ) {
                        let _ = fs::remove_dir_all(stage.path());
                        stage.disarm();
                        return Err(error);
                    }
                    return Ok(stage);
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(RasterPublicationError::Io {
                        operation: "create unique staged raster directory",
                        path,
                        source,
                    });
                }
            }
        }
        Err(RasterPublicationError::Io {
            operation: "allocate unique staged raster directory",
            path: destination.parent.clone(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "all unique raster stage names were occupied",
            ),
        })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("raster stage is armed")
    }

    fn disarm(&mut self) -> PathBuf {
        self.path.take().expect("raster stage is armed")
    }

    fn mark_displaced_owned_directory(&mut self) {
        self.cleanup = StageCleanup::DisplacedOwnedDirectory;
    }
}

impl Drop for RasterStage {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            match self.cleanup {
                StageCleanup::PrivateStage => {
                    // This process allocated the unique sibling under the
                    // stable writer lock, so it can safely remove even a
                    // partially written stage after an ordinary returned
                    // error.
                    let _ = fs::remove_dir_all(&path);
                }
                StageCleanup::DisplacedOwnedDirectory => {
                    let _ = remove_owned_directory(
                        &path,
                        self.format,
                        &self.target,
                        CleanupInventory::Exact,
                    );
                }
            }
        }
    }
}

fn write_staged_pages(
    stage: &Path,
    format: RasterFormat,
    pages: &[Vec<u8>],
) -> Result<(), RasterPublicationError> {
    for (index, bytes) in pages.iter().enumerate() {
        let path = stage.join(format!("page-{:03}.{}", index + 1, format.extension()));
        write_new_synced_file(&path, bytes)?;
    }
    Ok(())
}

fn write_new_synced_file(path: &Path, bytes: &[u8]) -> Result<(), RasterPublicationError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| RasterPublicationError::Io {
            operation: "create staged raster file",
            path: path.to_path_buf(),
            source,
        })?;
    if let Err(source) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(RasterPublicationError::Io {
            operation: "write and sync staged raster file",
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

fn validate_complete_owned_set(
    path: &Path,
    format: RasterFormat,
    target: &str,
    generation: Option<&str>,
    expected_pages: Option<usize>,
) -> Result<OwnershipMarker, RasterPublicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RasterPublicationError::Io {
        operation: "inspect owned raster directory",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: "the owned raster path is not a regular directory".to_string(),
        });
    }
    let marker = read_and_validate_marker(
        &path.join(OWNERSHIP_MARKER_FILE),
        format,
        target,
        generation,
    )?;
    let mut pages = Vec::new();
    for entry in fs::read_dir(path).map_err(|source| RasterPublicationError::Io {
        operation: "read owned raster directory",
        path: path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| RasterPublicationError::Io {
            operation: "read owned raster directory entry",
            path: path.to_path_buf(),
            source,
        })?;
        if entry.file_name() == OWNERSHIP_MARKER_FILE {
            continue;
        }
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| RasterPublicationError::Io {
                operation: "inspect owned raster page",
                path: entry_path.clone(),
                source,
            })?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason: "an owned raster page is not a regular file".to_string(),
            });
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason: "an owned raster page has a non-UTF-8 name".to_string(),
            });
        };
        let Some(number) = canonical_page_number(&name, format) else {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason: format!(
                    "owned raster directories may contain only canonical page-NNN.{} files",
                    format.extension()
                ),
            });
        };
        let bytes = fs::read(&entry_path).map_err(|source| RasterPublicationError::Io {
            operation: "read owned raster page for inventory validation",
            path: entry_path,
            source,
        })?;
        pages.push((number, name, hex_bytes(&Sha256::digest(&bytes))));
    }
    pages.sort_by_key(|(number, _, _)| *number);
    if pages.is_empty()
        || pages
            .iter()
            .map(|(number, _, _)| *number)
            .ne(1..=pages.len())
        || expected_pages.is_some_and(|expected| pages.len() != expected)
        || marker.page_count != pages.len()
        || marker
            .pages
            .iter()
            .zip(&pages)
            .any(|(expected, (_, file, sha256))| {
                expected.file != *file || expected.sha256 != *sha256
            })
    {
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: format!(
                "owned raster pages do not match the marker's contiguous page-001 through page-N {} inventory and hashes",
                format.marker_value()
            ),
        });
    }
    Ok(marker)
}

fn canonical_page_number(name: &str, format: RasterFormat) -> Option<usize> {
    let suffix = format!(".{}", format.extension());
    let digits = name.strip_prefix("page-")?.strip_suffix(&suffix)?;
    let number = digits.parse::<usize>().ok()?;
    (number > 0 && name == format!("page-{number:03}.{}", format.extension())).then_some(number)
}

struct AdoptionGuard {
    marker_path: Option<PathBuf>,
    directory: PathBuf,
    parent: PathBuf,
}

impl AdoptionGuard {
    fn claim(
        destination: &Destination,
        format: RasterFormat,
        evidence: &EmptyDirectoryEvidence,
    ) -> Result<Self, RasterPublicationError> {
        let current = inspect_destination(destination, format)?;
        if current != DestinationState::Empty(evidence.clone()) {
            return Err(RasterPublicationError::DestinationChanged {
                path: destination.requested.clone(),
            });
        }
        let marker = OwnershipMarker::new(
            format,
            &destination.key,
            format!("adopted-{}", new_generation_id()),
            &[],
        );
        let marker_path = destination.path.join(OWNERSHIP_MARKER_FILE);
        let mut marker_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker_path)
            .map_err(|source| RasterPublicationError::Io {
                operation: "create raster ownership-adoption marker",
                path: marker_path.clone(),
                source,
            })?;
        let mut adoption = Self {
            marker_path: Some(marker_path),
            directory: destination.path.clone(),
            parent: destination.parent.clone(),
        };
        if let Err(source) = marker_file
            .write_all(&render_marker(&marker))
            .and_then(|()| marker_file.sync_all())
        {
            drop(marker_file);
            let error = RasterPublicationError::Io {
                operation: "write and sync raster ownership-adoption marker",
                path: adoption
                    .marker_path
                    .as_ref()
                    .expect("adoption marker is armed")
                    .clone(),
                source,
            };
            return Err(adoption.rollback_or_combine(error));
        }
        drop(marker_file);
        if let Err(source) = sync_directory(&destination.path) {
            let error = RasterPublicationError::Io {
                operation: "sync newly adopted raster directory",
                path: destination.path.clone(),
                source,
            };
            return Err(adoption.rollback_or_combine(error));
        }
        if let Err(source) = sync_directory(&destination.parent) {
            let error = RasterPublicationError::Io {
                operation: "sync raster ownership adoption",
                path: destination.parent.clone(),
                source,
            };
            return Err(adoption.rollback_or_combine(error));
        }
        Ok(adoption)
    }

    fn disarm(&mut self) {
        self.marker_path.take();
    }

    fn rollback(&mut self) -> Result<(), RasterPublicationError> {
        let Some(marker_path) = self.marker_path.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if test_failpoint_active(TestFailpoint::BeforeExchangeAndRollbackFailure) {
            return Err(RasterPublicationError::Io {
                operation: "roll back raster ownership adoption",
                path: marker_path.clone(),
                source: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected raster adoption rollback failure",
                ),
            });
        }
        fs::remove_file(marker_path).map_err(|source| RasterPublicationError::Io {
            operation: "roll back raster ownership adoption",
            path: marker_path.clone(),
            source,
        })?;
        self.marker_path.take();
        sync_directory(&self.directory).map_err(|source| RasterPublicationError::Io {
            operation: "sync rolled-back raster ownership adoption",
            path: self.directory.clone(),
            source,
        })?;
        sync_directory(&self.parent).map_err(|source| RasterPublicationError::Io {
            operation: "sync parent after raster ownership rollback",
            path: self.parent.clone(),
            source,
        })
    }

    fn rollback_or_combine(
        &mut self,
        publication_error: RasterPublicationError,
    ) -> RasterPublicationError {
        match self.rollback() {
            Ok(()) => publication_error,
            Err(rollback_error) => RasterPublicationError::AdoptionRollbackFailed {
                path: self.directory.clone(),
                publication_error: Box::new(publication_error),
                rollback_error: Box::new(rollback_error),
            },
        }
    }
}

impl Drop for AdoptionGuard {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

fn recover_owned_stages(
    destination: &Destination,
    format: RasterFormat,
) -> Result<Vec<String>, RasterPublicationError> {
    let prefix = destination.stage_prefix();
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&destination.parent).map_err(|source| RasterPublicationError::Io {
        operation: "scan raster destination parent for abandoned stages",
        path: destination.parent.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| RasterPublicationError::Io {
            operation: "read raster stage candidate",
            path: destination.parent.clone(),
            source,
        })?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            candidates.push(entry.path());
        }
    }
    candidates.sort();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    sync_directory(&destination.parent).map_err(|source| RasterPublicationError::Io {
        operation: "stabilize raster namespace before crash recovery",
        path: destination.parent.clone(),
        source,
    })?;
    let mut warnings = Vec::new();
    let mut removed = false;
    for path in candidates {
        match remove_owned_directory(
            &path,
            format,
            &destination.key,
            CleanupInventory::RecoverableSubset,
        ) {
            Ok(()) => removed = true,
            Err(error) => warnings.push(format!(
                "preserved unrecognized or unremovable raster stage '{}': {error}",
                path.display()
            )),
        }
    }
    if removed {
        sync_directory(&destination.parent).map_err(|source| RasterPublicationError::Io {
            operation: "sync raster namespace after crash recovery",
            path: destination.parent.clone(),
            source,
        })?;
    }
    Ok(warnings)
}

fn interrupted_empty_adoption_marker(
    destination: &Destination,
    format: RasterFormat,
) -> Result<Option<PathBuf>, RasterPublicationError> {
    let metadata = match fs::symlink_metadata(&destination.path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RasterPublicationError::Io {
                operation: "inspect raster destination for interrupted adoption",
                path: destination.path.clone(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(None);
    }
    let entries = read_directory_entries(&destination.path)?;
    if entries.len() != 1
        || entries[0].file_name().and_then(|name| name.to_str()) != Some(OWNERSHIP_MARKER_FILE)
    {
        return Ok(None);
    }
    let marker_path = destination.path.join(OWNERSHIP_MARKER_FILE);
    let marker = read_and_validate_marker(&marker_path, format, &destination.key, None)?;
    if !marker.generation.starts_with("adopted-")
        || marker.page_count != 0
        || !marker.pages.is_empty()
    {
        return Ok(None);
    }
    Ok(Some(marker_path))
}

fn recover_interrupted_empty_adoption(
    destination: &Destination,
    format: RasterFormat,
) -> Result<(), RasterPublicationError> {
    let Some(marker_path) = interrupted_empty_adoption_marker(destination, format)? else {
        return Ok(());
    };

    sync_directory(&destination.parent).map_err(|source| RasterPublicationError::Io {
        operation: "stabilize raster namespace before interrupted-adoption recovery",
        path: destination.parent.clone(),
        source,
    })?;
    fs::remove_file(&marker_path).map_err(|source| RasterPublicationError::Io {
        operation: "restore empty raster directory after interrupted adoption",
        path: marker_path,
        source,
    })?;
    sync_directory(&destination.path).map_err(|source| RasterPublicationError::Io {
        operation: "sync restored empty raster directory",
        path: destination.path.clone(),
        source,
    })?;
    sync_directory(&destination.parent).map_err(|source| RasterPublicationError::Io {
        operation: "sync parent after interrupted-adoption recovery",
        path: destination.parent.clone(),
        source,
    })
}

fn remove_owned_directory(
    path: &Path,
    format: RasterFormat,
    target: &str,
    inventory: CleanupInventory,
) -> Result<(), RasterPublicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RasterPublicationError::Io {
        operation: "inspect owned raster directory for cleanup",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason: "the cleanup candidate is not a real directory".to_string(),
        });
    }
    let marker = read_and_validate_marker(&path.join(OWNERSHIP_MARKER_FILE), format, target, None)?;
    let entries = fs::read_dir(path).map_err(|source| RasterPublicationError::Io {
        operation: "read owned raster directory for cleanup",
        path: path.to_path_buf(),
        source,
    })?;
    let mut validated_entries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RasterPublicationError::Io {
            operation: "read owned raster cleanup entry",
            path: path.to_path_buf(),
            source,
        })?;
        if entry.file_name() == OWNERSHIP_MARKER_FILE {
            continue;
        }
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| RasterPublicationError::Io {
                operation: "inspect owned raster cleanup entry",
                path: entry_path.clone(),
                source,
            })?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason: "the owned cleanup candidate contains a symlink or non-regular entry"
                    .to_string(),
            });
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason: "the owned cleanup candidate contains a non-UTF-8 entry".to_string(),
            });
        };
        let Some(expected) = marker.pages.iter().find(|page| page.file == name) else {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason:
                    "the owned cleanup candidate contains a file absent from its marker inventory"
                        .to_string(),
            });
        };
        let bytes = fs::read(&entry_path).map_err(|source| RasterPublicationError::Io {
            operation: "read owned raster cleanup entry for hash validation",
            path: entry_path.clone(),
            source,
        })?;
        if hex_bytes(&Sha256::digest(&bytes)) != expected.sha256 {
            return Err(RasterPublicationError::UnownedDestination {
                path: entry_path,
                reason: "the owned cleanup candidate contains a page whose bytes do not match its marker inventory"
                    .to_string(),
            });
        }
        validated_entries.push(entry_path);
    }

    // Do not mutate the candidate until every entry has been proven to be a
    // regular, marker-listed file with the expected bytes. A stage interrupted
    // during population may contain only a subset of the marker inventory; it
    // is still safe to clean because no unlisted entry is touched.
    validated_entries.sort();
    if inventory == CleanupInventory::Exact && validated_entries.len() != marker.page_count {
        return Err(RasterPublicationError::UnownedDestination {
            path: path.to_path_buf(),
            reason:
                "the completed owned cleanup candidate does not contain its full marker inventory"
                    .to_string(),
        });
    }
    for entry_path in validated_entries {
        fs::remove_file(&entry_path).map_err(|source| RasterPublicationError::Io {
            operation: "remove owned raster cleanup entry",
            path: entry_path,
            source,
        })?;
    }
    sync_directory(path).map_err(|source| RasterPublicationError::Io {
        operation: "sync emptied owned raster directory",
        path: path.to_path_buf(),
        source,
    })?;
    let marker_path = path.join(OWNERSHIP_MARKER_FILE);
    fs::remove_file(&marker_path).map_err(|source| RasterPublicationError::Io {
        operation: "remove raster ownership marker during cleanup",
        path: marker_path,
        source,
    })?;
    sync_directory(path).map_err(|source| RasterPublicationError::Io {
        operation: "sync owned raster marker removal",
        path: path.to_path_buf(),
        source,
    })?;
    fs::remove_dir(path).map_err(|source| RasterPublicationError::Io {
        operation: "remove empty owned raster directory",
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupInventory {
    Exact,
    RecoverableSubset,
}

struct WriterLock {
    file: File,
    warnings: Vec<String>,
}

impl WriterLock {
    fn acquire(destination: &Destination) -> Result<Self, RasterPublicationError> {
        let path = destination.lock_path();
        let expected = format!(
            "zpres-raster-lock schema=1 generator=zpres target={}\n",
            destination.key
        );
        if let Some(file) = open_existing_writer_lock(&path, &expected, &destination.requested)? {
            let warnings = recover_abandoned_writer_lock_stages(destination, &expected);
            return Ok(Self { file, warnings });
        }

        let mut stage = WriterLockStage::allocate(destination, &expected)?;
        match stage.publish(&path, &destination.parent) {
            Ok(file) => {
                let warnings = recover_abandoned_writer_lock_stages(destination, &expected);
                Ok(Self { file, warnings })
            }
            Err(RasterPublicationError::DestinationChanged { .. }) => {
                drop(stage);
                let file = open_existing_writer_lock(&path, &expected, &destination.requested)?
                    .ok_or_else(|| RasterPublicationError::DestinationChanged {
                        path: path.clone(),
                    })?;
                let warnings = recover_abandoned_writer_lock_stages(destination, &expected);
                Ok(Self { file, warnings })
            }
            Err(error) => Err(error),
        }
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

fn open_existing_writer_lock(
    path: &Path,
    expected: &str,
    requested: &Path,
) -> Result<Option<File>, RasterPublicationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RasterPublicationError::Io {
                operation: "inspect raster writer lock",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_LOCK_BYTES {
        return Err(RasterPublicationError::InvalidDestination {
            path: path.to_path_buf(),
            reason: "the reserved raster writer-lock path is not a bounded regular file"
                .to_string(),
        });
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| RasterPublicationError::Io {
            operation: "open raster writer lock",
            path: path.to_path_buf(),
            source,
        })?;
    lock_file_nonblocking(&file, requested)?;
    let mut actual = String::new();
    file.read_to_string(&mut actual)
        .map_err(|source| RasterPublicationError::Io {
            operation: "read raster writer lock",
            path: path.to_path_buf(),
            source,
        })?;
    if actual != expected {
        return Err(RasterPublicationError::InvalidDestination {
            path: path.to_path_buf(),
            reason: "the reserved raster writer-lock file is not owned by this destination"
                .to_string(),
        });
    }
    Ok(Some(file))
}

struct WriterLockStage {
    path: Option<PathBuf>,
    file: Option<File>,
}

impl WriterLockStage {
    fn allocate(destination: &Destination, expected: &str) -> Result<Self, RasterPublicationError> {
        for _ in 0..UNIQUE_ATTEMPTS {
            let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = destination.parent.join(format!(
                "{LOCK_STAGE_PREFIX}{}-{}-{sequence:016x}",
                &destination.key[..24],
                std::process::id()
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    if let Err(source) = file
                        .write_all(expected.as_bytes())
                        .and_then(|()| file.sync_all())
                    {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(RasterPublicationError::Io {
                            operation: "initialize staged raster writer lock",
                            path,
                            source,
                        });
                    }
                    if let Err(error) = lock_file_nonblocking(&file, &destination.requested) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(Self {
                        path: Some(path),
                        file: Some(file),
                    });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(RasterPublicationError::Io {
                        operation: "create staged raster writer lock",
                        path,
                        source,
                    });
                }
            }
        }
        Err(RasterPublicationError::Io {
            operation: "allocate staged raster writer lock",
            path: destination.parent.clone(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "all unique raster writer-lock stage names were occupied",
            ),
        })
    }

    fn publish(
        &mut self,
        destination: &Path,
        parent: &Path,
    ) -> Result<File, RasterPublicationError> {
        let path = self.path.as_deref().expect("writer-lock stage is armed");
        rename_noreplace(path, destination)?;
        self.path.take();
        sync_directory(parent).map_err(|source| RasterPublicationError::Io {
            operation: "sync atomic raster writer-lock publication",
            path: parent.to_path_buf(),
            source,
        })?;
        Ok(self.file.take().expect("writer-lock stage owns its file"))
    }
}

impl Drop for WriterLockStage {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn recover_abandoned_writer_lock_stages(destination: &Destination, expected: &str) -> Vec<String> {
    let prefix = format!("{LOCK_STAGE_PREFIX}{}-", &destination.key[..24]);
    let entries = match fs::read_dir(&destination.parent) {
        Ok(entries) => entries,
        Err(source) => {
            return vec![format!(
                "could not scan for abandoned raster writer-lock stages: {source}"
            )];
        }
    };
    let mut warnings = Vec::new();
    let mut removed = false;
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(file_type) if file_type.is_file() && !file_type.is_symlink() => {}
            Ok(_) => {
                warnings.push(format!(
                    "preserved unrecognized raster writer-lock stage '{}'",
                    path.display()
                ));
                continue;
            }
            Err(source) => {
                warnings.push(format!(
                    "could not inspect raster writer-lock stage '{}': {source}",
                    path.display()
                ));
                continue;
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.len() <= MAX_LOCK_BYTES => {}
            Ok(_) => {
                warnings.push(format!(
                    "preserved oversized raster writer-lock stage '{}'",
                    path.display()
                ));
                continue;
            }
            Err(source) => {
                warnings.push(format!(
                    "could not inspect raster writer-lock stage '{}': {source}",
                    path.display()
                ));
                continue;
            }
        }
        let mut file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(source) => {
                warnings.push(format!(
                    "could not open raster writer-lock stage '{}': {source}",
                    path.display()
                ));
                continue;
            }
        };
        match lock_file_nonblocking(&file, &path) {
            Err(RasterPublicationError::ConcurrentPublication { .. }) => continue,
            Err(error) => {
                warnings.push(format!(
                    "could not lock raster writer-lock stage '{}': {error}",
                    path.display()
                ));
                continue;
            }
            Ok(()) => {}
        }
        let mut actual = String::new();
        if file.read_to_string(&mut actual).is_err() || actual != expected {
            warnings.push(format!(
                "preserved unrecognized raster writer-lock stage '{}'",
                path.display()
            ));
            continue;
        }
        if let Err(source) = fs::remove_file(&path) {
            warnings.push(format!(
                "could not remove abandoned raster writer-lock stage '{}': {source}",
                path.display()
            ));
        } else {
            removed = true;
        }
    }
    if removed && let Err(source) = sync_directory(&destination.parent) {
        warnings.push(format!(
            "removed abandoned raster writer-lock stages but could not sync their parent: {source}"
        ));
    }
    warnings
}

fn destination_key(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"zpres-raster-destination\0");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(not(unix))]
    hasher.update(path.as_os_str().to_string_lossy().as_bytes());
    hex_bytes(&hasher.finalize())
}

#[cfg(unix)]
fn raw_path_contains_current_directory(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str()
        .as_bytes()
        .split(|byte| *byte == b'/')
        .any(|component| component == b".")
}

#[cfg(not(unix))]
fn raw_path_contains_current_directory(path: &Path) -> bool {
    let path = path.as_os_str().to_string_lossy();
    path.split(['/', '\\']).any(|component| component == ".")
}

fn new_generation_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("r-{nanos:032x}-{:08x}-{sequence:016x}", std::process::id())
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> FileIdentity {
    FileIdentity
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn sync_parent_after_commit(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::ParentSyncAfterCommit) {
        return Err(io::Error::other(
            "injected parent sync failure after raster commit",
        ));
    }
    sync_directory(path)
}

fn test_post_commit_cleanup_fails() -> bool {
    #[cfg(test)]
    {
        test_failpoint_active(TestFailpoint::PostCommitCleanup)
    }
    #[cfg(not(test))]
    {
        false
    }
}

// Atomic raster publication currently depends on the same native directory
// primitives as HTML publication. Other platforms fail before the requested
// destination is inspected or mutated.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ensure_supported_platform(_path: &Path) -> Result<(), RasterPublicationError> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn ensure_supported_platform(path: &Path) -> Result<(), RasterPublicationError> {
    Err(RasterPublicationError::AtomicPublicationUnsupported {
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::Unsupported,
            "native no-replace rename, directory exchange, and OS-released writer locks are implemented only for macOS and Linux",
        ),
    })
}

fn rename_noreplace(source: &Path, destination: &Path) -> Result<(), RasterPublicationError> {
    native_fs::rename_noreplace(source, destination).map_err(|error| match error {
        RenameError::Io(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            RasterPublicationError::DestinationChanged {
                path: destination.to_path_buf(),
            }
        }
        error => map_native_rename_error(
            error,
            destination,
            "atomically publish no-replace raster directory",
        ),
    })
}

fn exchange_paths(source: &Path, destination: &Path) -> Result<(), RasterPublicationError> {
    native_fs::exchange_paths(source, destination).map_err(|error| {
        map_native_rename_error(
            error,
            destination,
            "atomically exchange complete raster directories",
        )
    })
}

fn map_native_rename_error(
    error: RenameError,
    destination: &Path,
    operation: &'static str,
) -> RasterPublicationError {
    match error {
        RenameError::InvalidPath(path) => RasterPublicationError::InvalidDestination {
            path,
            reason: "the path contains an interior NUL byte".to_string(),
        },
        RenameError::Io(source) if native_fs::atomic_operation_unsupported(&source) => {
            RasterPublicationError::AtomicPublicationUnsupported {
                path: destination.to_path_buf(),
                source,
            }
        }
        RenameError::Io(source) => RasterPublicationError::Io {
            operation,
            path: destination.to_path_buf(),
            source,
        },
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn lock_file_nonblocking(file: &File, output: &Path) -> Result<(), RasterPublicationError> {
    native_fs::lock_file_nonblocking(file).map_err(|source| {
        if source.kind() == io::ErrorKind::WouldBlock {
            RasterPublicationError::ConcurrentPublication {
                path: output.to_path_buf(),
            }
        } else {
            RasterPublicationError::Io {
                operation: "acquire raster publication writer lock",
                path: output.to_path_buf(),
                source,
            }
        }
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn lock_file_nonblocking(_file: &File, output: &Path) -> Result<(), RasterPublicationError> {
    ensure_supported_platform(output)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestFailpoint {
    LeakPopulatedStage,
    BeforeExchangeReturnedError,
    BeforeExchangeAndRollbackFailure,
    AfterEmptyAdoptionBeforeExchange,
    AfterExchangeBeforeParentSync,
    ParentSyncAfterCommit,
    PostCommitCleanup,
}

#[cfg(test)]
thread_local! {
    static TEST_FAILPOINT: std::cell::Cell<Option<TestFailpoint>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn test_failpoint_active(failpoint: TestFailpoint) -> bool {
    TEST_FAILPOINT.with(|active| active.get() == Some(failpoint))
}

#[cfg(test)]
struct TestFailpointGuard {
    previous: Option<TestFailpoint>,
}

#[cfg(test)]
impl TestFailpointGuard {
    fn set(failpoint: TestFailpoint) -> Self {
        let previous = TEST_FAILPOINT.with(|active| active.replace(Some(failpoint)));
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for TestFailpointGuard {
    fn drop(&mut self) {
        TEST_FAILPOINT.with(|active| active.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    fn stage_paths(output: &Path) -> Vec<PathBuf> {
        let destination = Destination::prepare(output).unwrap();
        let mut paths = fs::read_dir(&destination.parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with(&destination.stage_prefix())
                })
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    #[test]
    fn publishes_a_complete_marked_png_directory() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");

        let publication = publish_raster_page_set(
            &output,
            RasterFormat::Png,
            vec![b"first".to_vec(), b"second".to_vec()],
        )
        .unwrap();

        assert_eq!(
            publication.files,
            vec![output.join("page-001.png"), output.join("page-002.png")]
        );
        assert!(publication.warnings.is_empty());
        assert!(publication.generation.starts_with("r-"));
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"first");
        assert_eq!(fs::read(output.join("page-002.png")).unwrap(), b"second");
        let destination = Destination::prepare(&output).unwrap();
        validate_complete_owned_set(
            &destination.path,
            RasterFormat::Png,
            &destination.key,
            None,
            Some(2),
        )
        .unwrap();
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn replaces_the_whole_owned_directory_and_removes_stale_pages() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(
            &output,
            RasterFormat::Png,
            vec![b"old one".to_vec(), b"old two".to_vec()],
        )
        .unwrap();
        let mut already_open_page = File::open(output.join("page-002.png")).unwrap();

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"new one".to_vec()]).unwrap();

        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"new one");
        assert!(!output.join("page-002.png").exists());
        let mut old_bytes = Vec::new();
        already_open_page.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, b"old two");
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn claims_a_truly_empty_directory() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        fs::create_dir(&output).unwrap();

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"page".to_vec()]).unwrap();

        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"page");
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn returned_exchange_error_restores_a_previously_empty_directory() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        fs::create_dir(&output).unwrap();

        let error = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::BeforeExchangeReturnedError);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"page".to_vec()]).unwrap_err()
        };

        assert!(matches!(
            error,
            RasterPublicationError::InjectedReturnedFailure { .. }
        ));
        assert!(output.read_dir().unwrap().next().is_none());
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn adoption_rollback_failure_is_surfaced_and_recoverable() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        fs::create_dir(&output).unwrap();

        let error = {
            let _failpoint =
                TestFailpointGuard::set(TestFailpoint::BeforeExchangeAndRollbackFailure);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"page".to_vec()]).unwrap_err()
        };

        assert!(matches!(
            error,
            RasterPublicationError::AdoptionRollbackFailed { .. }
        ));
        assert!(output.join(OWNERSHIP_MARKER_FILE).is_file());
        assert!(stage_paths(&output).is_empty());
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"final".to_vec()]).unwrap();
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"final");
    }

    #[test]
    fn refuses_every_unmarked_nonempty_directory_including_legacy_pages() {
        let temp = tempdir().unwrap();
        for (name, entries) in [
            ("legacy", vec![("page-001.png", b"legacy".as_slice())]),
            ("mixed", vec![("notes.txt", b"notes".as_slice())]),
        ] {
            let output = temp.path().join(name);
            fs::create_dir(&output).unwrap();
            for (path, bytes) in &entries {
                fs::write(output.join(path), bytes).unwrap();
            }

            let error =
                publish_raster_page_set(&output, RasterFormat::Png, vec![b"replacement".to_vec()])
                    .unwrap_err();

            assert!(matches!(
                error,
                RasterPublicationError::UnownedDestination { .. }
            ));
            assert!(error.to_string().contains("Move or remove that directory"));
            for (path, bytes) in &entries {
                assert_eq!(fs::read(output.join(path)).unwrap(), *bytes);
            }
            assert!(!output.join(OWNERSHIP_MARKER_FILE).exists());
            assert!(stage_paths(&output).is_empty());
        }
    }

    #[test]
    fn one_owned_directory_cannot_change_raster_format() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"png".to_vec()]).unwrap();
        let before = fs::read(output.join("page-001.png")).unwrap();

        let error = publish_raster_page_set(&output, RasterFormat::Jpeg, vec![b"jpeg".to_vec()])
            .unwrap_err();

        assert!(matches!(
            error,
            RasterPublicationError::UnownedDestination { .. }
        ));
        assert!(
            error
                .to_string()
                .contains("'png' raster format, not 'jpeg'")
        );
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), before);
        assert!(!output.join("page-001.jpg").exists());
    }

    #[test]
    fn refuses_a_raster_destination_nested_inside_an_owned_page_set() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"png".to_vec()]).unwrap();
        let nested = output.join("jpeg");

        let error = publish_raster_page_set(&nested, RasterFormat::Jpeg, vec![b"jpeg".to_vec()])
            .unwrap_err();

        assert!(matches!(
            error,
            RasterPublicationError::OutputOwnership(
                OutputOwnershipError::OwnedRasterAncestor { .. }
            )
        ));
        assert!(
            error
                .to_string()
                .contains("exclusively owned raster page directory")
        );
        assert!(!nested.exists());
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"png");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn refuses_apfs_aliases_nested_inside_an_owned_page_set() {
        let temp = tempdir().unwrap();
        for (index, (owned_name, alias_name)) in [
            ("Pages", "pages"),
            ("caf\u{e9}", "cafe\u{301}"),
            ("stra\u{df}e", "STRASSE"),
            ("\u{fb03}", "FFI"),
        ]
        .into_iter()
        .enumerate()
        {
            let root = temp.path().join(format!("case-{index}"));
            fs::create_dir(&root).unwrap();
            let output = root.join(owned_name);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"png".to_vec()]).unwrap();
            let nested = root.join(alias_name).join("jpeg");

            let error =
                publish_raster_page_set(&nested, RasterFormat::Jpeg, vec![b"jpeg".to_vec()])
                    .unwrap_err();

            assert!(matches!(
                error,
                RasterPublicationError::OutputOwnership(
                    OutputOwnershipError::OwnedRasterAncestor { .. }
                )
            ));
            assert!(!nested.exists());
            assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"png");
        }
    }

    #[test]
    fn refuses_a_marked_directory_when_files_or_hashes_do_not_match_inventory() {
        let temp = tempdir().unwrap();
        let tampered = temp.path().join("tampered");
        publish_raster_page_set(&tampered, RasterFormat::Png, vec![b"original".to_vec()]).unwrap();
        fs::write(tampered.join("page-001.png"), b"user edit").unwrap();

        let error =
            publish_raster_page_set(&tampered, RasterFormat::Png, vec![b"replacement".to_vec()])
                .unwrap_err();
        assert!(matches!(
            error,
            RasterPublicationError::UnownedDestination { .. }
        ));
        assert_eq!(
            fs::read(tampered.join("page-001.png")).unwrap(),
            b"user edit"
        );

        let extra = temp.path().join("extra");
        publish_raster_page_set(&extra, RasterFormat::Png, vec![b"original".to_vec()]).unwrap();
        fs::write(extra.join("notes.txt"), b"keep me").unwrap();
        let error =
            publish_raster_page_set(&extra, RasterFormat::Png, vec![b"replacement".to_vec()])
                .unwrap_err();
        assert!(matches!(
            error,
            RasterPublicationError::UnownedDestination { .. }
        ));
        assert_eq!(fs::read(extra.join("notes.txt")).unwrap(), b"keep me");
        assert_eq!(fs::read(extra.join("page-001.png")).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn corrupt_owned_inventory_is_refused_and_preserved_byte_for_byte() {
        use std::os::unix::fs::symlink;

        #[derive(Debug, Clone, Copy)]
        enum Corruption {
            InvalidMarker,
            PageHash,
            MissingPage,
            ExtraFile,
            MarkerSymlink,
            NestedDirectory,
        }

        fn fingerprint(path: &Path) -> Vec<(String, String, Vec<u8>)> {
            let mut entries = fs::read_dir(path)
                .unwrap()
                .map(Result::unwrap)
                .map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let metadata = fs::symlink_metadata(entry.path()).unwrap();
                    if metadata.file_type().is_symlink() {
                        (
                            name,
                            "symlink".to_string(),
                            fs::read_link(entry.path())
                                .unwrap()
                                .as_os_str()
                                .to_string_lossy()
                                .as_bytes()
                                .to_vec(),
                        )
                    } else if metadata.is_dir() {
                        (name, "directory".to_string(), Vec::new())
                    } else {
                        (name, "file".to_string(), fs::read(entry.path()).unwrap())
                    }
                })
                .collect::<Vec<_>>();
            entries.sort();
            entries
        }

        let temp = tempdir().unwrap();
        for corruption in [
            Corruption::InvalidMarker,
            Corruption::PageHash,
            Corruption::MissingPage,
            Corruption::ExtraFile,
            Corruption::MarkerSymlink,
            Corruption::NestedDirectory,
        ] {
            let output = temp.path().join(format!("case-{corruption:?}"));
            publish_raster_page_set(
                &output,
                RasterFormat::Png,
                vec![b"one".to_vec(), b"two".to_vec()],
            )
            .unwrap();
            match corruption {
                Corruption::InvalidMarker => {
                    fs::write(output.join(OWNERSHIP_MARKER_FILE), b"{}\n").unwrap();
                }
                Corruption::PageHash => {
                    fs::write(output.join("page-001.png"), b"tampered").unwrap();
                }
                Corruption::MissingPage => {
                    fs::remove_file(output.join("page-002.png")).unwrap();
                }
                Corruption::ExtraFile => {
                    fs::write(output.join("notes.txt"), b"notes").unwrap();
                }
                Corruption::MarkerSymlink => {
                    let marker = fs::read(output.join(OWNERSHIP_MARKER_FILE)).unwrap();
                    let outside = temp.path().join(format!("marker-{corruption:?}.json"));
                    fs::write(&outside, marker).unwrap();
                    fs::remove_file(output.join(OWNERSHIP_MARKER_FILE)).unwrap();
                    symlink(outside, output.join(OWNERSHIP_MARKER_FILE)).unwrap();
                }
                Corruption::NestedDirectory => {
                    let nested = output.join("nested");
                    fs::create_dir(&nested).unwrap();
                    fs::write(nested.join("sentinel"), b"keep").unwrap();
                }
            }
            let before = fingerprint(&output);

            let error =
                publish_raster_page_set(&output, RasterFormat::Png, vec![b"replacement".to_vec()])
                    .unwrap_err();

            assert!(
                matches!(error, RasterPublicationError::UnownedDestination { .. }),
                "unexpected error for {corruption:?}: {error}"
            );
            assert_eq!(fingerprint(&output), before, "changed {corruption:?}");
            assert!(stage_paths(&output).is_empty());
        }
    }

    #[test]
    fn private_stage_drop_removes_a_partial_inventory_after_returned_error() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        let destination = Destination::prepare(&output).unwrap();
        let pages = vec![b"one".to_vec(), b"two".to_vec()];
        let marker = OwnershipMarker::new(
            RasterFormat::Png,
            &destination.key,
            new_generation_id(),
            &pages,
        );
        {
            let stage = RasterStage::allocate(&destination, marker).unwrap();
            write_new_synced_file(&stage.path().join("page-001.png"), &pages[0]).unwrap();
        }
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn generation_read_guard_rejects_a_rebuild_that_won_the_gap_and_blocks_writers() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        let first =
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"first".to_vec()]).unwrap();
        let second =
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"second".to_vec()]).unwrap();

        let stale_namespace = OutputNamespaceGuard::acquire(&output).unwrap();
        let stale = match lock_raster_page_set_for_read_under_namespace(
            &stale_namespace,
            &output,
            RasterFormat::Png,
            &first.generation,
            &first.files,
        ) {
            Ok(_) => panic!("stale generation unexpectedly acquired a read guard"),
            Err(error) => error,
        };
        assert!(matches!(
            stale,
            RasterPublicationError::DestinationChanged { .. }
        ));
        drop(stale_namespace);

        let namespace = OutputNamespaceGuard::acquire(&output).unwrap();
        let guard = lock_raster_page_set_for_read_under_namespace(
            &namespace,
            &output,
            RasterFormat::Png,
            &second.generation,
            &second.files,
        )
        .unwrap();
        assert_eq!(guard.files().len(), 1);
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let competing_output = output.clone();
        let competing = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = publish_raster_page_set(
                &competing_output,
                RasterFormat::Png,
                vec![b"third".to_vec()],
            );
            finished_tx.send(result).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(
            finished_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );
        drop(guard);
        drop(namespace);
        finished_rx
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap();
        competing.join().unwrap();
    }

    #[test]
    fn namespace_lock_closes_the_parent_child_adoption_race() {
        let temp = tempdir().unwrap();
        let parent = temp.path().join("pages");
        let child = parent.join("jpeg");
        let namespace = OutputNamespaceGuard::acquire(&parent).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let competing_child = child.clone();
        let contender = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = publish_raster_page_set(
                &competing_child,
                RasterFormat::Jpeg,
                vec![b"child".to_vec()],
            );
            finished_tx.send(result).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(
            finished_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );
        publish_raster_page_set_under_namespace(
            &namespace,
            &parent,
            RasterFormat::Png,
            vec![b"parent".to_vec()],
        )
        .unwrap();
        drop(namespace);

        let child_error = finished_rx
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap_err();
        contender.join().unwrap();
        assert!(matches!(
            child_error,
            RasterPublicationError::OutputOwnership(
                OutputOwnershipError::OwnedRasterAncestor { .. }
            )
        ));
        assert!(!child.exists());
        let destination = Destination::prepare(&parent).unwrap();
        validate_complete_owned_set(
            &destination.path,
            RasterFormat::Png,
            &destination.key,
            None,
            Some(1),
        )
        .unwrap();
    }

    #[test]
    fn publishes_jpeg_to_its_own_destination() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("jpeg-pages");

        let publication = publish_raster_page_set(
            &output,
            RasterFormat::Jpeg,
            vec![b"one".to_vec(), b"two".to_vec()],
        )
        .unwrap();

        assert_eq!(
            publication.files,
            vec![output.join("page-001.jpg"), output.join("page-002.jpg")]
        );
        assert!(!output.join("page-001.png").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_target_and_internal_symlinks_without_following_them() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        let target_link = temp.path().join("target-link");
        symlink(&outside, &target_link).unwrap();

        let error = publish_raster_page_set(
            &target_link,
            RasterFormat::Png,
            vec![b"replacement".to_vec()],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RasterPublicationError::UnownedDestination { .. }
        ));
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");

        let output = temp.path().join("owned");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"page".to_vec()]).unwrap();
        fs::remove_file(output.join("page-001.png")).unwrap();
        symlink(&outside, output.join("page-001.png")).unwrap();
        let error =
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"replacement".to_vec()])
                .unwrap_err();
        assert!(matches!(
            error,
            RasterPublicationError::UnownedDestination { .. }
        ));
        assert!(
            fs::symlink_metadata(output.join("page-001.png"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
    }

    #[test]
    fn rejects_root_dot_and_parent_directory_targets_before_mutation() {
        let temp = tempdir().unwrap();
        let unsafe_targets = [
            PathBuf::from("."),
            temp.path().join("child").join(".."),
            temp.path().join("child").join("..").join("pages"),
            PathBuf::from(format!("{}/.", temp.path().join("pages").display())),
            PathBuf::from("./pages"),
        ];

        for output in unsafe_targets {
            let error = publish_raster_page_set(&output, RasterFormat::Png, vec![b"page".to_vec()])
                .unwrap_err();
            assert!(matches!(
                error,
                RasterPublicationError::InvalidDestination { .. }
            ));
            assert!(
                error
                    .to_string()
                    .contains("only normal directory components")
            );
        }
    }

    #[test]
    fn preflight_is_read_only_for_unsafe_missing_and_unowned_destinations() {
        let temp = tempdir().unwrap();
        let unsafe_parent = temp.path().join("must-not-exist");
        let unsafe_output = unsafe_parent.join("..").join("pages");
        let error =
            preflight_raster_page_set_destination(&unsafe_output, RasterFormat::Png).unwrap_err();
        assert!(matches!(
            error,
            RasterPublicationError::InvalidDestination { .. }
        ));
        assert!(!unsafe_parent.exists());

        let missing = temp.path().join("missing-parent").join("pages");
        preflight_raster_page_set_destination(&missing, RasterFormat::Png).unwrap();
        assert!(!missing.parent().unwrap().exists());

        let unowned = temp.path().join("unowned");
        fs::create_dir(&unowned).unwrap();
        fs::write(unowned.join("sentinel"), b"user").unwrap();
        let before = fs::read(unowned.join("sentinel")).unwrap();
        let error = preflight_raster_page_set_destination(&unowned, RasterFormat::Png).unwrap_err();
        assert!(matches!(
            error,
            RasterPublicationError::UnownedDestination { .. }
        ));
        assert_eq!(fs::read(unowned.join("sentinel")).unwrap(), before);
    }

    #[test]
    fn a_post_exchange_parent_sync_error_is_success_and_retains_the_displaced_set() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(
            &output,
            RasterFormat::Png,
            vec![b"old one".to_vec(), b"old two".to_vec()],
        )
        .unwrap();
        let publication = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::ParentSyncAfterCommit);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"new".to_vec()]).unwrap()
        };

        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"new");
        assert!(!output.join("page-002.png").exists());
        assert_eq!(publication.warnings.len(), 2);
        assert_eq!(stage_paths(&output).len(), 1);

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"final".to_vec()]).unwrap();
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn recovers_a_populated_stage_left_before_commit() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"old".to_vec()]).unwrap();

        let error = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::LeakPopulatedStage);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"crashed".to_vec()])
                .unwrap_err()
        };
        assert!(matches!(
            error,
            RasterPublicationError::InjectedCrash { .. }
        ));
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"old");
        assert_eq!(stage_paths(&output).len(), 1);

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"final".to_vec()]).unwrap();
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"final");
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn recovers_a_post_exchange_counterpart_without_rolling_back_the_new_set() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"old".to_vec()]).unwrap();

        let error = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::AfterExchangeBeforeParentSync);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"new".to_vec()]).unwrap_err()
        };
        assert!(matches!(
            error,
            RasterPublicationError::InjectedCrash { .. }
        ));
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"new");
        assert_eq!(stage_paths(&output).len(), 1);

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"final".to_vec()]).unwrap();
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"final");
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn recovers_an_interrupted_empty_directory_adoption() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        fs::create_dir(&output).unwrap();

        let error = {
            let _failpoint =
                TestFailpointGuard::set(TestFailpoint::AfterEmptyAdoptionBeforeExchange);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"crashed".to_vec()])
                .unwrap_err()
        };
        assert!(matches!(
            error,
            RasterPublicationError::InjectedCrash { .. }
        ));
        assert!(output.join(OWNERSHIP_MARKER_FILE).is_file());
        assert_eq!(stage_paths(&output).len(), 1);

        // Normal exports preflight before reaching the publisher's recovery.
        let before = fs::read(output.join(OWNERSHIP_MARKER_FILE)).unwrap();
        preflight_raster_page_set_destination(&output, RasterFormat::Png).unwrap();
        assert_eq!(
            fs::read(output.join(OWNERSHIP_MARKER_FILE)).unwrap(),
            before
        );
        assert!(preflight_raster_page_set_destination(&output, RasterFormat::Jpeg).is_err());

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"final".to_vec()]).unwrap();
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"final");
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn post_commit_cleanup_failure_is_success_with_a_recoverable_warning() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"old".to_vec()]).unwrap();

        let publication = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::PostCommitCleanup);
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"new".to_vec()]).unwrap()
        };
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"new");
        assert_eq!(publication.warnings.len(), 1);
        assert_eq!(stage_paths(&output).len(), 1);

        publish_raster_page_set(&output, RasterFormat::Png, vec![b"final".to_vec()]).unwrap();
        assert!(stage_paths(&output).is_empty());
    }

    #[test]
    fn preserves_an_unmarked_stage_lookalike_during_recovery() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        let destination = Destination::prepare(&output).unwrap();
        let lookalike = destination
            .parent
            .join(format!("{}user-data", destination.stage_prefix()));
        fs::create_dir(&lookalike).unwrap();
        fs::write(lookalike.join("sentinel"), b"user").unwrap();

        let publication =
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"page".to_vec()]).unwrap();

        assert_eq!(publication.warnings.len(), 1);
        assert_eq!(fs::read(lookalike.join("sentinel")).unwrap(), b"user");
    }

    #[test]
    fn a_live_writer_lock_prevents_stage_recovery_and_competing_publication() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        let destination = Destination::prepare(&output).unwrap();
        let _lock = WriterLock::acquire(&destination).unwrap();

        let error =
            publish_raster_page_set(&output, RasterFormat::Png, vec![b"competing".to_vec()])
                .unwrap_err();

        assert!(matches!(
            error,
            RasterPublicationError::ConcurrentPublication { .. }
        ));
        assert!(!output.exists());
    }

    #[test]
    fn writer_lock_initialization_is_atomic_and_recovers_complete_crash_stages() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        let destination = Destination::prepare(&output).unwrap();
        let expected = format!(
            "zpres-raster-lock schema=1 generator=zpres target={}\n",
            destination.key
        );
        let mut stage = WriterLockStage::allocate(&destination, &expected).unwrap();
        let abandoned = stage.path.take().unwrap();
        let staged_file = stage.file.take().unwrap();
        drop(staged_file);
        drop(stage);
        assert!(abandoned.is_file());
        assert!(!destination.lock_path().exists());

        let mut lock = WriterLock::acquire(&destination).unwrap();

        assert!(destination.lock_path().is_file());
        assert!(!abandoned.exists());
        assert!(lock.take_warnings().is_empty());
    }

    #[test]
    fn noncanonical_persistent_writer_lock_is_refused_without_replacement() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        let destination = Destination::prepare(&output).unwrap();
        fs::write(destination.lock_path(), b"").unwrap();

        let error = match WriterLock::acquire(&destination) {
            Ok(_) => panic!("noncanonical persistent lock was accepted"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            RasterPublicationError::InvalidDestination { .. }
        ));
        assert_eq!(fs::read(destination.lock_path()).unwrap(), b"");
    }

    #[test]
    fn peer_file_output_cannot_replace_the_persistent_raster_lock() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"first".to_vec()]).unwrap();
        let destination = Destination::prepare(&output).unwrap();
        let lock_path = destination.lock_path();
        let before = fs::read(&lock_path).unwrap();
        let namespace = OutputNamespaceGuard::acquire(&lock_path).unwrap();

        assert!(matches!(
            namespace.publish_file(&lock_path, b"corrupt"),
            Err(OutputOwnershipError::InvalidOutputTarget { .. })
        ));
        drop(namespace);
        assert_eq!(fs::read(&lock_path).unwrap(), before);
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"second".to_vec()]).unwrap();
        assert_eq!(fs::read(output.join("page-001.png")).unwrap(), b"second");
    }

    #[test]
    fn concurrent_observers_never_see_the_canonical_directory_missing() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("pages");
        publish_raster_page_set(&output, RasterFormat::Png, vec![b"old".to_vec()]).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let missing = Arc::new(AtomicBool::new(false));
        let observer_output = output.clone();
        let observer_stop = Arc::clone(&stop);
        let observer_missing = Arc::clone(&missing);
        let observer = thread::spawn(move || {
            while !observer_stop.load(AtomicOrdering::Acquire) {
                match fs::symlink_metadata(&observer_output) {
                    Ok(metadata) if metadata.is_dir() => {}
                    _ => observer_missing.store(true, AtomicOrdering::Release),
                }
            }
        });

        for generation in 0..32 {
            publish_raster_page_set(
                &output,
                RasterFormat::Png,
                vec![format!("generation {generation}").into_bytes()],
            )
            .unwrap();
        }
        stop.store(true, AtomicOrdering::Release);
        observer.join().unwrap();

        assert!(!missing.load(AtomicOrdering::Acquire));
    }
}
