use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::native_fs::{self, RenameError, unlock_file};

use crate::output_ownership::{OutputNamespaceGuard, OutputOwnershipError, OutputTargetKind};

pub(crate) const OWNERSHIP_MARKER_FILE: &str = ".zpres-output.json";
pub(crate) const HTML_GENERATIONS_DIRECTORY: &str = "zpres-html-generations";
pub(crate) const HTML_STAGE_MARKER_FILE: &str = ".zpres-stage.json";
pub(crate) const HTML_GENERATION_MARKER_FILE: &str = ".zpres-generation.json";
pub(crate) const HTML_PRESENTATION_FILE: &str = "presentation.html";

const GENERATOR: &str = "zpres";
const SCHEMA: u32 = 1;
const ROOT_MARKER_KIND: &str = "html-generation-root";
const STAGE_MARKER_KIND: &str = "html-generation-stage";
const GENERATION_MARKER_KIND: &str = "html-generation";
const WRITER_LOCK_FILE: &str = ".zpres-html-publish.lock";
const ROOT_STAGE_PREFIX: &str = ".zpres-generation-root-stage-";
const POINTER_STAGE_PREFIX: &str = ".zpres-index-stage-";
const GENERATION_STAGE_PREFIX: &str = ".zpres-stage-";
const UNIQUE_ATTEMPTS: usize = 128;
const MAX_MARKER_BYTES: u64 = 16 * 1024;
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
// Keep the current generation plus fifteen recent generations. This bounds
// disk growth while leaving a generous window for already-loaded browsers to
// finish lazy asset reads after a newer build commits.
const MAX_RETAINED_HTML_GENERATIONS: usize = 16;
// The count limit is intentionally a soft bound: even generations beyond it
// remain for at least an hour so a burst of rapid rebuilds cannot invalidate a
// browser that is still lazily reading assets from a recently loaded page.
// Direct bookmarks to a generation older than both retention windows may
// eventually return 404; the stable root index is the supported bookmark.
const MIN_GENERATION_RETENTION_AGE: Duration = Duration::from_secs(60 * 60);

static UNIQUE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A committed generation-qualified HTML presentation.
///
/// `pointer_index` is the only root-level presentation file replaced by this
/// module. `presentation_index` is the byte-for-byte renderer output inside a
/// retained generation directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlPublication {
    pub(crate) output_root: PathBuf,
    pub(crate) pointer_index: PathBuf,
    pub(crate) generation: String,
    pub(crate) generation_path: PathBuf,
    pub(crate) presentation_index: PathBuf,
    pub(crate) previous_generation: Option<String>,
    pub(crate) retained_replaced_index: Option<PathBuf>,
    pub(crate) warnings: Vec<String>,
}

/// The generation currently selected by a canonical checksummed root pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentHtmlPublication {
    pub(crate) generation: String,
    pub(crate) generation_path: PathBuf,
    pub(crate) presentation_index: PathBuf,
}

/// Resolve the current generation without creating, locking, or modifying the
/// output root.
///
/// A missing root/index returns `None`. A checksummed pointer is
/// returned only after its generation directory, completion marker, and
/// presentation index have all been validated.
pub(crate) fn current_html_publication(
    output_root: &Path,
) -> Result<Option<CurrentHtmlPublication>, PublicationError> {
    let Some(output_root) = resolve_existing_output_root(output_root)? else {
        return Ok(None);
    };
    let pointer_index = output_root.join("index.html");
    let generation = match inspect_index(&pointer_index)? {
        IndexState::Missing => return Ok(None),
        IndexState::Owned(OwnedIndex { generation, .. }) => generation,
    };

    let generations_root = output_root.join(HTML_GENERATIONS_DIRECTORY);
    validate_generation_root(&generations_root)?;
    let generation_path = generations_root.join(&generation);
    validate_permanent_generation(&generation_path, &generation)?;
    let presentation_index = generation_path.join(HTML_PRESENTATION_FILE);
    validate_immutable_presentation(&generation_path)?;
    Ok(Some(CurrentHtmlPublication {
        generation,
        generation_path,
        presentation_index,
    }))
}

/// Populate, retain, and publish one complete HTML presentation generation.
///
/// The callback receives an empty stage below the zpres-owned generation
/// subtree. Once populated, the whole bundle is synced and renamed to its
/// permanent generation path. Only then is the root `index.html` atomically
/// changed to a canonical, checksummed, dependency-free pointer. Other root
/// files remain untouched; older generations are retired and retained under
/// the documented count-and-age policy only after that commit is durable.
pub(crate) fn publish_html_bundle<F, E>(
    output_root: &Path,
    populate: F,
) -> Result<HtmlPublication, PublicationError>
where
    F: FnOnce(&Path) -> Result<(), E>,
    E: StdError + Send + Sync + 'static,
{
    ensure_supported_platform(output_root)?;
    let _namespace = OutputNamespaceGuard::acquire(output_root)?;
    _namespace.ensure_peer_output_allowed(output_root, OutputTargetKind::Directory)?;
    let output_root = prepare_output_root(output_root)?;
    let pointer_index = output_root.join("index.html");

    // Refuse a user-owned root index before creating even the persistent lock
    // file. The index is inspected again after the cooperative lock is held.
    inspect_index(&pointer_index)?;
    let generations_root = ensure_generation_root(&output_root)?;
    let _writer_lock = WriterLock::acquire(&generations_root)?;
    validate_generation_root(&generations_root)?;
    let initial_index = inspect_index(&pointer_index)?;
    let mut warnings = recover_abandoned_stages(&output_root, &generations_root)?;

    let generation = new_generation_id();
    let mut generation_stage = GenerationStage::allocate(&generations_root, &generation)?;
    populate(generation_stage.path()).map_err(|source| PublicationError::Populate {
        path: generation_stage.path().to_path_buf(),
        source: Box::new(source),
    })?;
    validate_stage_marker(generation_stage.path(), &generation)?;
    validate_live_generation_index(generation_stage.path())?;
    snapshot_renderer_index(generation_stage.path())?;
    write_generation_marker(generation_stage.path(), &generation)?;
    sync_tree(generation_stage.path())?;
    remove_stage_marker(generation_stage.path())?;
    sync_directory(generation_stage.path()).map_err(|source| PublicationError::Io {
        operation: "sync finalized HTML generation metadata",
        path: generation_stage.path().to_path_buf(),
        source,
    })?;
    sync_directory(&generations_root).map_err(|source| PublicationError::Io {
        operation: "sync staged HTML generation entry",
        path: generations_root.clone(),
        source,
    })?;

    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::LeakGenerationStage) {
        let path = generation_stage.disarm();
        return Err(PublicationError::InjectedCrash { path });
    }

    let generation_path = generations_root.join(&generation);
    rename_noreplace(generation_stage.path(), &generation_path)?;
    generation_stage.disarm();
    let mut committed_generation =
        CommittedGenerationGuard::new(&generations_root, &generation_path, &generation);
    sync_directory(&generations_root).map_err(|source| PublicationError::Io {
        operation: "sync committed HTML generation entry",
        path: generations_root.clone(),
        source,
    })?;

    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::AfterGenerationCommitBeforePointer) {
        let path = committed_generation.disarm();
        return Err(PublicationError::InjectedCrash { path });
    }

    let pointer_bytes = render_pointer(&generation).into_bytes();
    let mut pointer_stage = StagedPointer::create(&generations_root, &pointer_bytes)?;
    let expected_pointer =
        inspect_owned_index_file(pointer_stage.path())?.ok_or_else(|| PublicationError::Io {
            operation: "validate staged canonical HTML pointer",
            path: pointer_stage.path().to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "staged pointer did not validate as zpres-owned",
            ),
        })?;
    sync_directory(&generations_root).map_err(|source| PublicationError::Io {
        operation: "sync staged HTML pointer entry",
        path: generations_root.clone(),
        source,
    })?;

    let current_index = inspect_index(&pointer_index)?;
    if current_index != initial_index {
        return Err(PublicationError::DestinationChanged {
            path: pointer_index,
        });
    }

    match &initial_index {
        IndexState::Missing => rename_noreplace(pointer_stage.path(), &pointer_index)?,
        IndexState::Owned(_) => exchange_paths(pointer_stage.path(), &pointer_index)?,
    }
    committed_generation.disarm();

    // The root pointer is now committed. Every later failure becomes a warning
    // so callers are never told that the old presentation was restored.
    let replaced_index_path = pointer_stage.disarm();
    let mut publication = HtmlPublication {
        output_root: output_root.clone(),
        pointer_index: pointer_index.clone(),
        generation: generation.clone(),
        generation_path: generation_path.clone(),
        presentation_index: generation_path.join(HTML_PRESENTATION_FILE),
        previous_generation: initial_index.pointer_generation().map(str::to_string),
        retained_replaced_index: None,
        warnings: Vec::new(),
    };

    let output_root_sync = sync_after_pointer_commit(
        &output_root,
        #[cfg(test)]
        TestFailpoint::PostExchangeRootSync,
    );
    if let Err(source) = &output_root_sync {
        publication.warnings.push(format!(
            "published '{}' but could not sync the output root for power-loss durability: {source}",
            publication.pointer_index.display()
        ));
    }
    let generations_root_sync = sync_after_pointer_commit(
        &generations_root,
        #[cfg(test)]
        TestFailpoint::PostExchangeStageSync,
    );
    if let Err(source) = &generations_root_sync {
        publication.warnings.push(format!(
            "published '{}' but could not sync the owned pointer-stage directory for power-loss durability: {source}",
            publication.pointer_index.display()
        ));
    }

    let pointer_commit_durable = output_root_sync.is_ok() && generations_root_sync.is_ok();
    if let IndexState::Owned(previous_index) = &initial_index {
        if pointer_commit_durable {
            cleanup_replaced_index(
                &replaced_index_path,
                previous_index,
                &pointer_index,
                &expected_pointer,
                &mut publication,
            );
        } else {
            publication.retained_replaced_index = Some(replaced_index_path.clone());
            publication.warnings.push(format!(
                "retained replaced root index '{}' because both sides of the atomic exchange were not durably synced",
                replaced_index_path.display()
            ));
        }
    }
    if pointer_commit_durable {
        let mut maintenance = maintain_completed_generations(&generations_root, &generation);
        publication.warnings.append(&mut maintenance);
    } else {
        publication.warnings.push(
            "skipped generation retirement and collection because the root-pointer exchange was not durably synced on both parent directories"
                .to_string(),
        );
    }
    publication.warnings.append(&mut warnings);
    Ok(publication)
}

#[derive(Debug, Error)]
pub(crate) enum PublicationError {
    #[error(transparent)]
    OutputOwnership(#[from] OutputOwnershipError),

    #[error("cannot use output root '{}': {reason}", path.display())]
    InvalidOutputRoot { path: PathBuf, reason: String },

    #[error(
        "refusing to replace root index '{}': {reason}",
        path.display()
    )]
    UnownedIndex { path: PathBuf, reason: String },

    #[error(
        "refusing to use unowned HTML generation subtree '{}': {reason}",
        path.display()
    )]
    UnownedGenerationRoot { path: PathBuf, reason: String },

    #[error(
        "another zpres HTML publication already holds the writer lock for '{}'",
        path.display()
    )]
    ConcurrentPublication { path: PathBuf },

    #[error(
        "destination '{}' changed while the new generation was staged; the newer destination was preserved",
        path.display()
    )]
    DestinationChanged { path: PathBuf },

    #[error(
        "atomic HTML pointer publication is unsupported for '{}': {source}; the previous root index was preserved",
        path.display()
    )]
    AtomicPublicationUnsupported {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to populate staged HTML generation '{}': {source}", path.display())]
    Populate {
        path: PathBuf,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },

    #[error("staged HTML generation '{}' has no regular index.html", path.display())]
    MissingPresentationIndex { path: PathBuf },

    #[error("HTML generation '{}' has no immutable presentation.html", path.display())]
    MissingImmutablePresentation { path: PathBuf },

    #[error("staged output reserved '{}'; that path is owned by zpres", path.display())]
    ReservedPath { path: PathBuf },

    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[cfg(test)]
    #[error("injected process crash left staged generation '{}'; publication did not commit", path.display())]
    InjectedCrash { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IndexState {
    Missing,
    Owned(OwnedIndex),
}

impl IndexState {
    fn pointer_generation(&self) -> Option<&str> {
        match self {
            Self::Owned(OwnedIndex { generation, .. }) => Some(generation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedIndex {
    identity: FileIdentity,
    digest: [u8; 32],
    generation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegularFileEvidence {
    identity: FileIdentity,
    digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletedGeneration {
    generation: String,
    path: PathBuf,
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

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct OwnershipMarker {
    schema: u32,
    generator: String,
    kind: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct StageMarker {
    schema: u32,
    generator: String,
    kind: String,
    generation: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct GenerationMarker {
    schema: u32,
    generator: String,
    kind: String,
    generation: String,
}

fn resolve_existing_output_root(output_root: &Path) -> Result<Option<PathBuf>, PublicationError> {
    let metadata = match fs::symlink_metadata(output_root) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(PublicationError::Io {
                operation: "inspect HTML output root",
                path: output_root.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(PublicationError::InvalidOutputRoot {
            path: output_root.to_path_buf(),
            reason: "the output root is a symbolic link".to_string(),
        });
    }
    if !metadata.is_dir() {
        return Err(PublicationError::InvalidOutputRoot {
            path: output_root.to_path_buf(),
            reason: "the output root exists but is not a directory".to_string(),
        });
    }
    fs::canonicalize(output_root)
        .map(Some)
        .map_err(|source| PublicationError::Io {
            operation: "resolve HTML output root",
            path: output_root.to_path_buf(),
            source,
        })
}

fn prepare_output_root(output_root: &Path) -> Result<PathBuf, PublicationError> {
    match fs::symlink_metadata(output_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(PublicationError::InvalidOutputRoot {
                path: output_root.to_path_buf(),
                reason: "the output root is a symbolic link".to_string(),
            });
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(PublicationError::InvalidOutputRoot {
                path: output_root.to_path_buf(),
                reason: "the output root exists but is not a directory".to_string(),
            });
        }
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(output_root).map_err(|source| PublicationError::Io {
                operation: "create HTML output root",
                path: output_root.to_path_buf(),
                source,
            })?;
        }
        Err(source) => {
            return Err(PublicationError::Io {
                operation: "inspect HTML output root",
                path: output_root.to_path_buf(),
                source,
            });
        }
    }
    fs::canonicalize(output_root).map_err(|source| PublicationError::Io {
        operation: "resolve HTML output root",
        path: output_root.to_path_buf(),
        source,
    })
}

fn inspect_index(path: &Path) -> Result<IndexState, PublicationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(IndexState::Missing);
        }
        Err(source) => {
            return Err(PublicationError::Io {
                operation: "inspect root HTML index",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(PublicationError::UnownedIndex {
            path: path.to_path_buf(),
            reason: "the path is a symbolic link".to_string(),
        });
    }
    if !metadata.is_file() {
        return Err(PublicationError::UnownedIndex {
            path: path.to_path_buf(),
            reason: "the path is not a regular file".to_string(),
        });
    }
    inspect_owned_index_with_metadata(path, metadata)?.map_or_else(
        || {
            Err(PublicationError::UnownedIndex {
                path: path.to_path_buf(),
                reason: "the file is not a canonical checksummed zpres pointer".to_string(),
            })
        },
        |owned| Ok(IndexState::Owned(owned)),
    )
}

fn inspect_owned_index_file(path: &Path) -> Result<Option<OwnedIndex>, PublicationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(PublicationError::Io {
                operation: "inspect staged or replaced HTML index",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    inspect_owned_index_with_metadata(path, metadata)
}

fn inspect_owned_index_with_metadata(
    path: &Path,
    metadata: fs::Metadata,
) -> Result<Option<OwnedIndex>, PublicationError> {
    if metadata.len() > MAX_INDEX_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|source| PublicationError::Io {
        operation: "read HTML index ownership evidence",
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Ok(None);
    }
    let Some(generation) = parse_canonical_pointer(&bytes) else {
        return Ok(None);
    };
    Ok(Some(OwnedIndex {
        identity: file_identity(&metadata),
        digest: Sha256::digest(&bytes).into(),
        generation,
    }))
}

fn regular_file_evidence(path: &Path) -> Result<RegularFileEvidence, PublicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| PublicationError::Io {
        operation: "inspect generation index",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_INDEX_BYTES
    {
        return Err(PublicationError::Io {
            operation: "validate generation index",
            path: path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "generation index is not a bounded regular file",
            ),
        });
    }
    let bytes = fs::read(path).map_err(|source| PublicationError::Io {
        operation: "read generation index",
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(PublicationError::Io {
            operation: "validate generation index size",
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidData, "generation index is too large"),
        });
    }
    Ok(RegularFileEvidence {
        identity: file_identity(&metadata),
        digest: Sha256::digest(&bytes).into(),
    })
}

fn render_pointer(generation: &str) -> String {
    let target = pointer_target(generation);
    let checksum = pointer_checksum(generation);
    format!(
        "<!doctype html>\n<!-- zpres-html-pointer schema=1 generator=zpres generation={generation} checksum={checksum} -->\n<meta charset=\"utf-8\">\n<meta name=\"robots\" content=\"noindex\">\n<meta http-equiv=\"refresh\" content=\"1;url={target}\">\n<title>zpres presentation</title>\n<script>\n(() => {{\n  \"use strict\";\n  const target = \"{target}\";\n  window.location.replace(target + window.location.search + window.location.hash);\n}})();\n</script>\n<a href=\"{target}\">Open presentation</a>\n"
    )
}

fn parse_canonical_pointer(bytes: &[u8]) -> Option<String> {
    let html = std::str::from_utf8(bytes).ok()?;
    let mut lines = html.lines();
    if lines.next()? != "<!doctype html>" {
        return None;
    }
    let marker = lines.next()?;
    let marker = marker.strip_prefix("<!-- zpres-html-pointer ")?;
    let marker = marker.strip_suffix(" -->")?;
    let fields = marker.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 4 || fields[0] != "schema=1" || fields[1] != "generator=zpres" {
        return None;
    }
    let generation = fields[2].strip_prefix("generation=")?;
    let checksum = fields[3].strip_prefix("checksum=")?;
    if !valid_generation_id(generation) || checksum != pointer_checksum(generation) {
        return None;
    }
    if html != render_pointer(generation) {
        return None;
    }
    Some(generation.to_string())
}

fn pointer_target(generation: &str) -> String {
    format!("{HTML_GENERATIONS_DIRECTORY}/{generation}/index.html")
}

fn pointer_checksum(generation: &str) -> String {
    // This unkeyed checksum is canonical ownership evidence for trusted local
    // output, not authentication against a hostile writer.
    let mut hasher = Sha256::new();
    hasher.update(b"zpres-html-pointer\0schema=1\0generator=zpres\0generation=");
    hasher.update(generation.as_bytes());
    hex_bytes(&hasher.finalize())
}

fn render_retired_generation_pointer(generation: &str) -> String {
    let checksum = retired_generation_checksum(generation);
    let target = "../../index.html";
    format!(
        "<!doctype html>\n<!-- zpres-retired-generation schema=1 generator=zpres generation={generation} checksum={checksum} -->\n<meta charset=\"utf-8\">\n<meta name=\"robots\" content=\"noindex\">\n<meta http-equiv=\"refresh\" content=\"1;url={target}\">\n<title>zpres presentation moved</title>\n<script>\n(() => {{\n  \"use strict\";\n  const target = \"{target}\";\n  window.location.replace(target + window.location.search + window.location.hash);\n}})();\n</script>\n<a href=\"{target}\">Open current presentation</a>\n"
    )
}

fn parse_retired_generation_pointer(bytes: &[u8]) -> Option<String> {
    let html = std::str::from_utf8(bytes).ok()?;
    let mut lines = html.lines();
    if lines.next()? != "<!doctype html>" {
        return None;
    }
    let marker = lines.next()?;
    let marker = marker.strip_prefix("<!-- zpres-retired-generation ")?;
    let marker = marker.strip_suffix(" -->")?;
    let fields = marker.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 4 || fields[0] != "schema=1" || fields[1] != "generator=zpres" {
        return None;
    }
    let generation = fields[2].strip_prefix("generation=")?;
    let checksum = fields[3].strip_prefix("checksum=")?;
    if !valid_generation_id(generation)
        || checksum != retired_generation_checksum(generation)
        || html != render_retired_generation_pointer(generation)
    {
        return None;
    }
    Some(generation.to_string())
}

fn retired_generation_checksum(generation: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"zpres-retired-generation\0schema=1\0generator=zpres\0generation=");
    hasher.update(generation.as_bytes());
    hex_bytes(&hasher.finalize())
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

fn ensure_generation_root(output_root: &Path) -> Result<PathBuf, PublicationError> {
    let root = output_root.join(HTML_GENERATIONS_DIRECTORY);
    match fs::symlink_metadata(&root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PublicationError::UnownedGenerationRoot {
                    path: root,
                    reason: "the reserved path is not a regular directory".to_string(),
                });
            }
            if validate_ownership_marker(&root.join(OWNERSHIP_MARKER_FILE), ROOT_MARKER_KIND)
                .is_ok()
            {
                return Ok(root);
            }
            Err(PublicationError::UnownedGenerationRoot {
                path: root,
                reason: "the final generation subtree has missing or invalid ownership metadata; it was preserved unchanged"
                    .to_string(),
            })
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            initialize_generation_root(output_root, &root)
        }
        Err(source) => Err(PublicationError::Io {
            operation: "inspect HTML generation subtree",
            path: root,
            source,
        }),
    }
}

fn initialize_generation_root(
    output_root: &Path,
    root: &Path,
) -> Result<PathBuf, PublicationError> {
    for _ in 0..UNIQUE_ATTEMPTS {
        let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let stage_path = output_root.join(format!(
            "{ROOT_STAGE_PREFIX}{}-{sequence:016x}",
            std::process::id()
        ));
        match fs::create_dir(&stage_path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(PublicationError::Io {
                    operation: "create staged HTML generation subtree",
                    path: stage_path,
                    source,
                });
            }
        }
        let mut stage = GenerationRootStage {
            path: Some(stage_path),
        };
        let marker = OwnershipMarker {
            schema: SCHEMA,
            generator: GENERATOR.to_string(),
            kind: ROOT_MARKER_KIND.to_string(),
        };
        write_json_create_new(&stage.path().join(OWNERSHIP_MARKER_FILE), &marker)?;
        sync_directory(stage.path()).map_err(|source| PublicationError::Io {
            operation: "sync staged HTML generation subtree",
            path: stage.path().to_path_buf(),
            source,
        })?;
        match rename_noreplace(stage.path(), root) {
            Ok(()) => {
                stage.disarm();
                sync_directory(output_root).map_err(|source| PublicationError::Io {
                    operation: "sync HTML output root after generation-subtree commit",
                    path: output_root.to_path_buf(),
                    source,
                })?;
                return Ok(root.to_path_buf());
            }
            Err(PublicationError::DestinationChanged { .. }) => {
                drop(stage);
                validate_generation_root(root)?;
                return Ok(root.to_path_buf());
            }
            Err(error) => return Err(error),
        }
    }
    Err(PublicationError::Io {
        operation: "allocate staged HTML generation subtree",
        path: output_root.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "all unique generation-root stage names were occupied",
        ),
    })
}

struct GenerationRootStage {
    path: Option<PathBuf>,
}

impl GenerationRootStage {
    fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("generation-root stage is armed")
    }

    fn disarm(&mut self) -> PathBuf {
        self.path.take().expect("generation-root stage is armed")
    }
}

impl Drop for GenerationRootStage {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn validate_generation_root(root: &Path) -> Result<(), PublicationError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| PublicationError::Io {
        operation: "reinspect locked HTML generation subtree",
        path: root.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PublicationError::UnownedGenerationRoot {
            path: root.to_path_buf(),
            reason: "the locked reserved path is not a regular directory".to_string(),
        });
    }
    validate_ownership_marker(&root.join(OWNERSHIP_MARKER_FILE), ROOT_MARKER_KIND).map_err(
        |reason| PublicationError::UnownedGenerationRoot {
            path: root.to_path_buf(),
            reason,
        },
    )
}

fn validate_permanent_generation(
    generation_path: &Path,
    expected_generation: &str,
) -> Result<(), PublicationError> {
    let metadata = fs::symlink_metadata(generation_path).map_err(|source| {
        PublicationError::UnownedGenerationRoot {
            path: generation_path.to_path_buf(),
            reason: format!("the checksummed pointer's generation is unavailable: {source}"),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PublicationError::UnownedGenerationRoot {
            path: generation_path.to_path_buf(),
            reason: "the checksummed pointer's generation is not a regular directory".to_string(),
        });
    }
    let marker_path = generation_path.join(HTML_GENERATION_MARKER_FILE);
    let marker = read_json_marker::<GenerationMarker>(&marker_path).map_err(|error| {
        PublicationError::UnownedGenerationRoot {
            path: generation_path.to_path_buf(),
            reason: format!("the generation completion marker is invalid: {error}"),
        }
    })?;
    if marker.schema != SCHEMA
        || marker.generator != GENERATOR
        || marker.kind != GENERATION_MARKER_KIND
        || marker.generation != expected_generation
    {
        return Err(PublicationError::UnownedGenerationRoot {
            path: generation_path.to_path_buf(),
            reason: format!(
                "the completion marker does not identify generation '{expected_generation}'"
            ),
        });
    }
    Ok(())
}

fn validate_ownership_marker(path: &Path, expected_kind: &str) -> Result<(), String> {
    let marker = read_json_marker::<OwnershipMarker>(path).map_err(|error| error.to_string())?;
    if marker.schema != SCHEMA || marker.generator != GENERATOR || marker.kind != expected_kind {
        return Err(format!(
            "ownership marker has schema={}, generator='{}', kind='{}'; expected schema={SCHEMA}, generator='{GENERATOR}', kind='{expected_kind}'",
            marker.schema, marker.generator, marker.kind
        ));
    }
    Ok(())
}

fn new_generation_id() -> String {
    let nanos = current_time_nanos();
    let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("g-{nanos:032x}-{:08x}-{sequence:016x}", std::process::id())
}

fn current_time_nanos() -> u128 {
    #[cfg(test)]
    if let Some(nanos) = TEST_GENERATION_TIME_NANOS.with(|value| value.get()) {
        return nanos;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn generation_age_at(generation: &str, now_nanos: u128) -> Option<Duration> {
    let encoded = generation.strip_prefix("g-")?.split('-').next()?;
    let created_nanos = u128::from_str_radix(encoded, 16).ok()?;
    let age_nanos = now_nanos.saturating_sub(created_nanos);
    let seconds = (age_nanos / 1_000_000_000).min(u64::MAX as u128) as u64;
    let subsecond_nanos = (age_nanos % 1_000_000_000) as u32;
    Some(Duration::new(seconds, subsecond_nanos))
}

fn valid_generation_id(generation: &str) -> bool {
    generation.starts_with("g-")
        && generation.len() <= 96
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

struct GenerationStage {
    path: Option<PathBuf>,
}

impl GenerationStage {
    fn allocate(root: &Path, generation: &str) -> Result<Self, PublicationError> {
        let path = root.join(format!("{GENERATION_STAGE_PREFIX}{generation}"));
        fs::create_dir(&path).map_err(|source| PublicationError::Io {
            operation: "allocate staged HTML generation",
            path: path.clone(),
            source,
        })?;
        let stage = Self { path: Some(path) };
        #[cfg(test)]
        let mut stage = stage;
        #[cfg(test)]
        if test_failpoint_active(TestFailpoint::LeakPartialGenerationStageMarker) {
            let marker_path = stage.path().join(HTML_STAGE_MARKER_FILE);
            let mut marker = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker_path)
                .map_err(|source| PublicationError::Io {
                    operation: "create injected partial generation-stage marker",
                    path: marker_path.clone(),
                    source,
                })?;
            marker
                .write_all(b"{\"schema\":")
                .and_then(|()| marker.sync_all())
                .map_err(|source| PublicationError::Io {
                    operation: "write injected partial generation-stage marker",
                    path: marker_path,
                    source,
                })?;
            let path = stage.disarm();
            return Err(PublicationError::InjectedCrash { path });
        }
        let marker = StageMarker {
            schema: SCHEMA,
            generator: GENERATOR.to_string(),
            kind: STAGE_MARKER_KIND.to_string(),
            generation: generation.to_string(),
        };
        write_json_create_new(&stage.path().join(HTML_STAGE_MARKER_FILE), &marker)?;
        sync_directory(stage.path()).map_err(|source| PublicationError::Io {
            operation: "sync staged HTML generation metadata",
            path: stage.path().to_path_buf(),
            source,
        })?;
        Ok(stage)
    }

    fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("HTML generation stage is armed")
    }

    fn disarm(&mut self) -> PathBuf {
        self.path.take().expect("HTML generation stage is armed")
    }
}

impl Drop for GenerationStage {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

struct CommittedGenerationGuard {
    generations_root: PathBuf,
    path: Option<PathBuf>,
    generation: String,
}

impl CommittedGenerationGuard {
    fn new(generations_root: &Path, path: &Path, generation: &str) -> Self {
        Self {
            generations_root: generations_root.to_path_buf(),
            path: Some(path.to_path_buf()),
            generation: generation.to_string(),
        }
    }

    fn disarm(&mut self) -> PathBuf {
        self.path
            .take()
            .expect("committed HTML generation guard is armed")
    }
}

impl Drop for CommittedGenerationGuard {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if path.parent() != Some(self.generations_root.as_path())
            || validate_permanent_generation(&path, &self.generation).is_err()
        {
            return;
        }
        if fs::remove_dir_all(&path).is_ok() {
            let _ = sync_directory(&self.generations_root);
        }
    }
}

fn validate_stage_marker(stage: &Path, generation: &str) -> Result<(), PublicationError> {
    let path = stage.join(HTML_STAGE_MARKER_FILE);
    let marker = read_json_marker::<StageMarker>(&path)?;
    if marker.schema != SCHEMA
        || marker.generator != GENERATOR
        || marker.kind != STAGE_MARKER_KIND
        || marker.generation != generation
    {
        return Err(PublicationError::ReservedPath { path });
    }
    Ok(())
}

fn validate_live_generation_index(stage: &Path) -> Result<(), PublicationError> {
    let path = stage.join("index.html");
    let metadata = fs::symlink_metadata(&path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PublicationError::MissingPresentationIndex { path: path.clone() }
        } else {
            PublicationError::Io {
                operation: "inspect staged presentation index",
                path: path.clone(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PublicationError::MissingPresentationIndex { path });
    }
    Ok(())
}

fn snapshot_renderer_index(stage: &Path) -> Result<(), PublicationError> {
    let source_path = stage.join("index.html");
    let destination = stage.join(HTML_PRESENTATION_FILE);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(PublicationError::ReservedPath { path: destination }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(PublicationError::Io {
                operation: "inspect reserved immutable presentation path",
                path: destination,
                source,
            });
        }
    }

    let mut source = File::open(&source_path).map_err(|source| PublicationError::Io {
        operation: "open renderer index for immutable snapshot",
        path: source_path,
        source,
    })?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|source| PublicationError::Io {
            operation: "create immutable renderer snapshot",
            path: destination.clone(),
            source,
        })?;
    io::copy(&mut source, &mut destination_file)
        .and_then(|_| destination_file.sync_all())
        .map_err(|source| PublicationError::Io {
            operation: "write immutable renderer snapshot",
            path: destination,
            source,
        })?;
    Ok(())
}

fn validate_immutable_presentation(generation: &Path) -> Result<(), PublicationError> {
    let path = generation.join(HTML_PRESENTATION_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            PublicationError::MissingImmutablePresentation { path: path.clone() }
        } else {
            PublicationError::Io {
                operation: "inspect immutable renderer snapshot",
                path: path.clone(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PublicationError::MissingImmutablePresentation { path });
    }
    Ok(())
}

fn write_generation_marker(stage: &Path, generation: &str) -> Result<(), PublicationError> {
    let path = stage.join(HTML_GENERATION_MARKER_FILE);
    match fs::symlink_metadata(&path) {
        Ok(_) => return Err(PublicationError::ReservedPath { path }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(PublicationError::Io {
                operation: "inspect reserved generation marker path",
                path,
                source,
            });
        }
    }
    let marker = GenerationMarker {
        schema: SCHEMA,
        generator: GENERATOR.to_string(),
        kind: GENERATION_MARKER_KIND.to_string(),
        generation: generation.to_string(),
    };
    write_json_create_new(&path, &marker)
}

fn remove_stage_marker(stage: &Path) -> Result<(), PublicationError> {
    let path = stage.join(HTML_STAGE_MARKER_FILE);
    fs::remove_file(&path).map_err(|source| PublicationError::Io {
        operation: "remove staging-only HTML generation metadata",
        path,
        source,
    })
}

fn write_json_create_new<T: Serialize>(path: &Path, value: &T) -> Result<(), PublicationError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| PublicationError::Io {
        operation: "serialize zpres publication metadata",
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| PublicationError::Io {
            operation: "create zpres publication metadata",
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| PublicationError::Io {
            operation: "write zpres publication metadata",
            path: path.to_path_buf(),
            source,
        })
}

fn read_json_marker<T: DeserializeOwned>(path: &Path) -> Result<T, PublicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| PublicationError::Io {
        operation: "inspect zpres publication metadata",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_MARKER_BYTES
    {
        return Err(PublicationError::Io {
            operation: "validate zpres publication metadata",
            path: path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "metadata is not a bounded regular file",
            ),
        });
    }
    let bytes = fs::read(path).map_err(|source| PublicationError::Io {
        operation: "read zpres publication metadata",
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(PublicationError::Io {
            operation: "validate zpres publication metadata after reading",
            path: path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "metadata grew beyond the bounded marker size while it was read",
            ),
        });
    }
    serde_json::from_slice(&bytes).map_err(|source| PublicationError::Io {
        operation: "parse zpres publication metadata",
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

struct StagedPointer {
    path: Option<PathBuf>,
}

impl StagedPointer {
    fn create(root: &Path, bytes: &[u8]) -> Result<Self, PublicationError> {
        for _ in 0..UNIQUE_ATTEMPTS {
            let sequence = UNIQUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "{POINTER_STAGE_PREFIX}{}-{sequence:016x}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    #[cfg(test)]
                    if test_failpoint_active(TestFailpoint::LeakPartialPointerStage) {
                        file.write_all(b"<!doctype")
                            .and_then(|()| file.sync_all())
                            .map_err(|source| PublicationError::Io {
                                operation: "write injected partial HTML pointer stage",
                                path: path.clone(),
                                source,
                            })?;
                        return Err(PublicationError::InjectedCrash { path });
                    }
                    if let Err(source) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                        let _ = fs::remove_file(&path);
                        return Err(PublicationError::Io {
                            operation: "write staged canonical HTML pointer",
                            path,
                            source,
                        });
                    }
                    return Ok(Self { path: Some(path) });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(PublicationError::Io {
                        operation: "allocate staged canonical HTML pointer",
                        path,
                        source,
                    });
                }
            }
        }
        Err(PublicationError::Io {
            operation: "allocate staged canonical HTML pointer",
            path: root.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "all unique pointer-stage names were occupied",
            ),
        })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("HTML pointer stage is armed")
    }

    fn disarm(&mut self) -> PathBuf {
        self.path.take().expect("HTML pointer stage is armed")
    }
}

impl Drop for StagedPointer {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn cleanup_replaced_index(
    replaced_path: &Path,
    expected_previous: &OwnedIndex,
    pointer_index: &Path,
    expected_pointer: &OwnedIndex,
    publication: &mut HtmlPublication,
) {
    let new_pointer = inspect_owned_index_file(pointer_index).ok().flatten();
    let replaced_index = inspect_owned_index_file(replaced_path).ok().flatten();
    if new_pointer.as_ref() != Some(expected_pointer)
        || replaced_index.as_ref() != Some(expected_previous)
    {
        publication.retained_replaced_index = Some(replaced_path.to_path_buf());
        publication.warnings.push(format!(
            "published '{}' but retained replaced index '{}' because its identity, content digest, or ownership evidence changed after the atomic swap",
            pointer_index.display(),
            replaced_path.display()
        ));
        return;
    }

    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::PostCommitPointerCleanup) {
        publication.retained_replaced_index = Some(replaced_path.to_path_buf());
        publication.warnings.push(format!(
            "published '{}' but retained replaced index '{}' after an injected cleanup failure",
            pointer_index.display(),
            replaced_path.display()
        ));
        return;
    }

    if let Err(source) = fs::remove_file(replaced_path) {
        publication.retained_replaced_index = Some(replaced_path.to_path_buf());
        publication.warnings.push(format!(
            "published '{}' but could not remove replaced index '{}': {source}",
            pointer_index.display(),
            replaced_path.display()
        ));
        return;
    }
    if let Some(stage_parent) = replaced_path.parent()
        && let Err(source) = sync_directory(stage_parent)
    {
        publication.warnings.push(format!(
            "published '{}' and removed its replaced index, but could not sync the owned pointer-stage directory: {source}",
            pointer_index.display()
        ));
    }
}

fn maintain_completed_generations(
    generations_root: &Path,
    current_generation: &str,
) -> Vec<String> {
    let (generations, mut warnings) = collect_completed_generations(generations_root);
    for generation in &generations {
        if generation.generation != current_generation {
            warnings.extend(retire_completed_generation(generations_root, generation));
        }
    }

    let (mut generations, mut rescan_warnings) = collect_completed_generations(generations_root);
    warnings.append(&mut rescan_warnings);
    generations.sort_by(|left, right| right.generation.cmp(&left.generation));

    let mut retained = BTreeSet::new();
    retained.insert(current_generation.to_string());
    for generation in &generations {
        if retained.len() >= MAX_RETAINED_HTML_GENERATIONS {
            break;
        }
        retained.insert(generation.generation.clone());
    }

    for generation in generations {
        if retained.contains(&generation.generation) {
            continue;
        }
        if generation_age_at(&generation.generation, current_time_nanos())
            .is_none_or(|age| age < MIN_GENERATION_RETENTION_AGE)
        {
            continue;
        }
        if let Err(reason) = prune_retired_generation(generations_root, &generation) {
            warnings.push(format!(
                "retained old HTML generation '{}' beyond the configured limit: {reason}",
                generation.path.display()
            ));
        }
    }
    warnings
}

fn collect_completed_generations(
    generations_root: &Path,
) -> (Vec<CompletedGeneration>, Vec<String>) {
    let entries = match fs::read_dir(generations_root) {
        Ok(entries) => entries,
        Err(source) => {
            return (
                Vec::new(),
                vec![format!(
                    "could not scan completed HTML generations in '{}': {source}",
                    generations_root.display()
                )],
            );
        }
    };
    let mut generations = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => {
                warnings.push(format!(
                    "could not read a completed-generation entry in '{}': {source}",
                    generations_root.display()
                ));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(generation) = name.to_str() else {
            continue;
        };
        if !valid_generation_id(generation) {
            continue;
        }
        let path = entry.path();
        if let Err(error) = validate_permanent_generation(&path, generation) {
            warnings.push(format!(
                "retained generation-like path '{}' because its completion marker was invalid: {error}",
                path.display()
            ));
            continue;
        }
        generations.push(CompletedGeneration {
            generation: generation.to_string(),
            path,
        });
    }
    (generations, warnings)
}

fn retire_completed_generation(
    generations_root: &Path,
    generation: &CompletedGeneration,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(error) = validate_permanent_generation(&generation.path, &generation.generation) {
        warnings.push(format!(
            "could not retire HTML generation '{}': {error}",
            generation.path.display()
        ));
        return warnings;
    }

    let index_path = generation.path.join("index.html");
    let existing_bytes = match fs::read(&index_path) {
        Ok(bytes) => bytes,
        Err(source) => {
            warnings.push(format!(
                "could not retire HTML generation index '{}': {source}",
                index_path.display()
            ));
            return warnings;
        }
    };
    if parse_retired_generation_pointer(&existing_bytes).as_deref()
        == Some(generation.generation.as_str())
    {
        if let Err(error) = validate_immutable_presentation(&generation.path) {
            warnings.push(format!(
                "retired generation '{}' lacks its immutable renderer snapshot: {error}",
                generation.path.display()
            ));
        }
        return warnings;
    }

    if let Err(error) = validate_immutable_presentation(&generation.path) {
        if matches!(error, PublicationError::MissingImmutablePresentation { .. }) {
            if let Err(error) = snapshot_renderer_index(&generation.path) {
                warnings.push(format!(
                    "could not snapshot older HTML generation '{}' before retirement: {error}",
                    generation.path.display()
                ));
                return warnings;
            }
            if let Err(source) = sync_directory(&generation.path) {
                warnings.push(format!(
                    "snapshotted older HTML generation '{}' but could not sync it before retirement: {source}",
                    generation.path.display()
                ));
                return warnings;
            }
        } else {
            warnings.push(format!(
                "could not validate older HTML generation '{}' before retirement: {error}",
                generation.path.display()
            ));
            return warnings;
        }
    }

    let previous_evidence = match regular_file_evidence(&index_path) {
        Ok(evidence) => evidence,
        Err(error) => {
            warnings.push(format!(
                "could not capture retirement evidence for '{}': {error}",
                index_path.display()
            ));
            return warnings;
        }
    };
    let redirect = render_retired_generation_pointer(&generation.generation);
    let mut stage = match StagedPointer::create(generations_root, redirect.as_bytes()) {
        Ok(stage) => stage,
        Err(error) => {
            warnings.push(format!(
                "could not stage retirement pointer for '{}': {error}",
                generation.path.display()
            ));
            return warnings;
        }
    };
    let redirect_evidence = match regular_file_evidence(stage.path()) {
        Ok(evidence) => evidence,
        Err(error) => {
            warnings.push(format!(
                "could not validate staged retirement pointer for '{}': {error}",
                generation.path.display()
            ));
            return warnings;
        }
    };
    if let Err(source) = sync_directory(generations_root) {
        warnings.push(format!(
            "could not sync staged retirement pointer for '{}': {source}",
            generation.path.display()
        ));
        return warnings;
    }
    if regular_file_evidence(&index_path).ok().as_ref() != Some(&previous_evidence) {
        warnings.push(format!(
            "generation index '{}' changed before retirement; it was preserved",
            index_path.display()
        ));
        return warnings;
    }
    if let Err(error) = exchange_paths(stage.path(), &index_path) {
        warnings.push(format!(
            "could not atomically retire generation index '{}': {error}",
            index_path.display()
        ));
        return warnings;
    }
    let replaced_index = stage.disarm();

    let generation_synced = sync_directory(&generation.path);
    let stages_synced = sync_directory(generations_root);
    if generation_synced.is_err() || stages_synced.is_err() {
        warnings.push(format!(
            "retired generation '{}' but retained its replaced index at '{}' because a post-exchange directory sync failed (generation: {}; stages: {})",
            generation.generation,
            replaced_index.display(),
            format_sync_result(&generation_synced),
            format_sync_result(&stages_synced)
        ));
        return warnings;
    }

    let redirect_is_current = fs::read(&index_path)
        .ok()
        .and_then(|bytes| parse_retired_generation_pointer(&bytes))
        .as_deref()
        == Some(generation.generation.as_str());
    let redirect_unchanged =
        regular_file_evidence(&index_path).ok().as_ref() == Some(&redirect_evidence);
    let replaced_unchanged =
        regular_file_evidence(&replaced_index).ok().as_ref() == Some(&previous_evidence);
    if !redirect_is_current || !redirect_unchanged || !replaced_unchanged {
        warnings.push(format!(
            "retired generation '{}' but retained replaced index '{}' because post-exchange ownership evidence changed",
            generation.generation,
            replaced_index.display()
        ));
        return warnings;
    }
    if let Err(source) = fs::remove_file(&replaced_index) {
        warnings.push(format!(
            "retired generation '{}' but could not remove replaced index '{}': {source}",
            generation.generation,
            replaced_index.display()
        ));
    } else if let Err(source) = sync_directory(generations_root) {
        warnings.push(format!(
            "retired generation '{}' and removed its replaced index, but could not sync '{}': {source}",
            generation.generation,
            generations_root.display()
        ));
    }
    warnings
}

fn prune_retired_generation(
    generations_root: &Path,
    generation: &CompletedGeneration,
) -> Result<(), String> {
    validate_permanent_generation(&generation.path, &generation.generation)
        .map_err(|error| error.to_string())?;
    validate_immutable_presentation(&generation.path).map_err(|error| error.to_string())?;
    let index_path = generation.path.join("index.html");
    let bytes = fs::read(&index_path).map_err(|error| error.to_string())?;
    if parse_retired_generation_pointer(&bytes).as_deref() != Some(generation.generation.as_str()) {
        return Err("generation index is not a canonical retired-generation pointer".to_string());
    }
    // Make the retirement exchange durable before deleting the generation it
    // guards. If either parent cannot be synced, the generation remains.
    sync_directory(&generation.path).map_err(|error| error.to_string())?;
    sync_directory(generations_root).map_err(|error| error.to_string())?;
    fs::remove_dir_all(&generation.path).map_err(|error| error.to_string())?;
    sync_directory(generations_root).map_err(|error| error.to_string())
}

fn format_sync_result(result: &io::Result<()>) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(error) => error.to_string(),
    }
}

fn recover_abandoned_stages(
    output_root: &Path,
    generations_root: &Path,
) -> Result<Vec<String>, PublicationError> {
    let mut warnings = Vec::new();
    if let Err(reason) = stabilize_abandoned_exchange_recovery(output_root, generations_root) {
        warnings.push(format!(
            "retained abandoned publication stages because prior exchanges could not be stabilized: {reason}"
        ));
        return Ok(warnings);
    }
    let mut changed_generations = false;
    for entry in fs::read_dir(generations_root).map_err(|source| PublicationError::Io {
        operation: "scan HTML generation subtree for abandoned stages",
        path: generations_root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| PublicationError::Io {
            operation: "read abandoned HTML generation stage entry",
            path: generations_root.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(generation) = name.strip_prefix(GENERATION_STAGE_PREFIX) else {
            continue;
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) => {
                warnings.push(format!(
                    "could not inspect possible abandoned generation stage '{}': {source}",
                    path.display()
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            warnings.push(format!(
                "retained unexpected reserved stage path '{}' because it is not a regular directory",
                path.display()
            ));
            continue;
        }

        // The validated parent subtree is wholly zpres-owned and the writer
        // lock excludes cooperating publishers. A valid generated stage name
        // is therefore sufficient to recover even a zero- or partially-written
        // stage marker left by process termination.
        if valid_generation_id(generation) {
            match fs::remove_dir_all(&path) {
                Ok(()) => {
                    changed_generations = true;
                    warnings.push(format!(
                        "removed abandoned zpres HTML generation stage '{}'",
                        path.display()
                    ));
                }
                Err(source) => warnings.push(format!(
                    "could not remove abandoned zpres HTML generation stage '{}': {source}",
                    path.display()
                )),
            }
        } else {
            warnings.push(format!(
                "retained possible abandoned generation stage '{}' because its generated name was invalid",
                path.display()
            ));
        }
    }
    if changed_generations && let Err(source) = sync_directory(generations_root) {
        warnings.push(format!(
            "recovered abandoned generation stages but could not sync '{}': {source}",
            generations_root.display()
        ));
    }

    let mut changed_pointer_stages = false;
    for entry in fs::read_dir(generations_root).map_err(|source| PublicationError::Io {
        operation: "scan owned HTML generation subtree for abandoned pointer stages",
        path: generations_root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| PublicationError::Io {
            operation: "read abandoned owned pointer stage entry",
            path: generations_root.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(POINTER_STAGE_PREFIX) || !name.ends_with(".tmp") {
            continue;
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) => {
                warnings.push(format!(
                    "could not inspect possible abandoned pointer stage '{}': {source}",
                    path.display()
                ));
                continue;
            }
        };
        // Pointer stages live only in the validated zpres-owned subtree. Any
        // regular file with our generated prefix can be recovered, including
        // a partial nonzero write or the renderer index left after a completed
        // exchange whose cleanup was interrupted.
        let safely_owned = metadata.is_file() && !metadata.file_type().is_symlink();
        if safely_owned {
            match fs::remove_file(&path) {
                Ok(()) => {
                    changed_pointer_stages = true;
                    warnings.push(format!(
                        "removed abandoned zpres HTML pointer stage '{}'",
                        path.display()
                    ));
                }
                Err(source) => warnings.push(format!(
                    "could not remove abandoned zpres HTML pointer stage '{}': {source}",
                    path.display()
                )),
            }
        } else {
            warnings.push(format!(
                "retained unexpected pointer-stage path '{}' because it is not a regular file",
                path.display()
            ));
        }
    }
    if changed_pointer_stages && let Err(source) = sync_directory(generations_root) {
        warnings.push(format!(
            "recovered abandoned pointer stages but could not sync '{}': {source}",
            generations_root.display()
        ));
    }
    Ok(warnings)
}

fn stabilize_abandoned_exchange_recovery(
    output_root: &Path,
    generations_root: &Path,
) -> Result<(), String> {
    #[cfg(test)]
    if test_failpoint_active(TestFailpoint::RecoveryStabilizationSync) {
        return Err("injected recovery-stabilization sync failure".to_string());
    }

    sync_directory(output_root).map_err(|error| {
        format!(
            "could not sync output root '{}': {error}",
            output_root.display()
        )
    })?;
    sync_directory(generations_root).map_err(|error| {
        format!(
            "could not sync generation root '{}': {error}",
            generations_root.display()
        )
    })?;
    let entries = fs::read_dir(generations_root).map_err(|error| {
        format!(
            "could not scan completed generations in '{}': {error}",
            generations_root.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(generation) = name.to_str() else {
            continue;
        };
        if !valid_generation_id(generation) {
            continue;
        }
        let path = entry.path();
        if validate_permanent_generation(&path, generation).is_err() {
            continue;
        }
        sync_directory(&path).map_err(|error| {
            format!(
                "could not sync completed generation '{}': {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn sync_tree(path: &Path) -> Result<(), PublicationError> {
    let entries = fs::read_dir(path).map_err(|source| PublicationError::Io {
        operation: "read staged HTML generation for durability sync",
        path: path.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| PublicationError::Io {
            operation: "read staged HTML generation entry",
            path: path.to_path_buf(),
            source,
        })?;
        let entry_path = entry.path();
        let file_type = entry.file_type().map_err(|source| PublicationError::Io {
            operation: "inspect staged HTML generation entry",
            path: entry_path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            sync_tree(&entry_path)?;
        } else if file_type.is_file() {
            File::open(&entry_path)
                .and_then(|file| file.sync_all())
                .map_err(|source| PublicationError::Io {
                    operation: "sync staged HTML generation file",
                    path: entry_path,
                    source,
                })?;
        } else {
            return Err(PublicationError::InvalidOutputRoot {
                path: entry_path,
                reason: "staged HTML generation contains a symlink or non-regular entry"
                    .to_string(),
            });
        }
    }
    sync_directory(path).map_err(|source| PublicationError::Io {
        operation: "sync staged HTML generation directory",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn sync_after_pointer_commit(path: &Path, #[cfg(test)] failpoint: TestFailpoint) -> io::Result<()> {
    #[cfg(test)]
    if test_failpoint_active(failpoint) {
        return Err(io::Error::other(
            "injected post-exchange directory sync failure",
        ));
    }
    sync_directory(path)
}

struct WriterLock {
    file: File,
}

impl WriterLock {
    fn acquire(output_root: &Path) -> Result<Self, PublicationError> {
        let path = output_root.join(WRITER_LOCK_FILE);
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                let metadata =
                    fs::symlink_metadata(&path).map_err(|source| PublicationError::Io {
                        operation: "inspect HTML publication writer lock",
                        path: path.clone(),
                        source,
                    })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(PublicationError::InvalidOutputRoot {
                        path,
                        reason: "the reserved writer-lock path is not a regular file".to_string(),
                    });
                }
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .map_err(|source| PublicationError::Io {
                        operation: "open HTML publication writer lock",
                        path: path.clone(),
                        source,
                    })?
            }
            Err(source) => {
                return Err(PublicationError::Io {
                    operation: "create HTML publication writer lock",
                    path,
                    source,
                });
            }
        };
        lock_file_nonblocking(&file, output_root)?;

        let handle_identity =
            file_identity(&file.metadata().map_err(|source| PublicationError::Io {
                operation: "inspect open HTML publication writer lock",
                path: path.clone(),
                source,
            })?);
        let path_metadata = fs::symlink_metadata(&path).map_err(|source| PublicationError::Io {
            operation: "reinspect HTML publication writer lock",
            path: path.clone(),
            source,
        })?;
        if path_metadata.file_type().is_symlink()
            || !path_metadata.is_file()
            || file_identity(&path_metadata) != handle_identity
        {
            return Err(PublicationError::DestinationChanged { path });
        }
        Ok(Self { file })
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn lock_file_nonblocking(file: &File, output_root: &Path) -> Result<(), PublicationError> {
    native_fs::lock_file_nonblocking(file).map_err(|source| {
        if source.kind() == io::ErrorKind::WouldBlock {
            PublicationError::ConcurrentPublication {
                path: output_root.to_path_buf(),
            }
        } else {
            PublicationError::Io {
                operation: "acquire HTML publication writer lock",
                path: output_root.to_path_buf(),
                source,
            }
        }
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn lock_file_nonblocking(_file: &File, output_root: &Path) -> Result<(), PublicationError> {
    Err(unsupported_error(output_root))
}

// Atomic publication currently requires macOS renameatx_np/flock or Linux
// renameat2/flock. The output root is a trusted local directory; the lock
// serializes cooperating zpres writers and is not a hostile-writer security
// mechanism. Other platforms fail before mutating the output root.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn ensure_supported_platform(_path: &Path) -> Result<(), PublicationError> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn ensure_supported_platform(path: &Path) -> Result<(), PublicationError> {
    Err(unsupported_error(path))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn unsupported_error(path: &Path) -> PublicationError {
    PublicationError::AtomicPublicationUnsupported {
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::Unsupported,
            "native atomic pointer exchange and an OS-released writer lock are implemented only for macOS and Linux",
        ),
    }
}

fn rename_noreplace(source: &Path, destination: &Path) -> Result<(), PublicationError> {
    native_fs::rename_noreplace(source, destination).map_err(|error| match error {
        RenameError::Io(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            PublicationError::DestinationChanged {
                path: destination.to_path_buf(),
            }
        }
        error => map_native_rename_error(
            error,
            destination,
            "atomically publish no-replace filesystem entry",
        ),
    })
}

fn exchange_paths(source: &Path, destination: &Path) -> Result<(), PublicationError> {
    native_fs::exchange_paths(source, destination).map_err(|error| {
        map_native_rename_error(error, destination, "atomically exchange root HTML pointer")
    })
}

fn map_native_rename_error(
    error: RenameError,
    destination: &Path,
    operation: &'static str,
) -> PublicationError {
    match error {
        RenameError::InvalidPath(path) => PublicationError::InvalidOutputRoot {
            path,
            reason: "the path contains an interior NUL byte".to_string(),
        },
        RenameError::Io(source) if native_fs::atomic_operation_unsupported(&source) => {
            PublicationError::AtomicPublicationUnsupported {
                path: destination.to_path_buf(),
                source,
            }
        }
        RenameError::Io(source) => PublicationError::Io {
            operation,
            path: destination.to_path_buf(),
            source,
        },
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestFailpoint {
    LeakGenerationStage,
    LeakPartialGenerationStageMarker,
    LeakPartialPointerStage,
    AfterGenerationCommitBeforePointer,
    PostExchangeRootSync,
    PostExchangeStageSync,
    RecoveryStabilizationSync,
    PostCommitPointerCleanup,
}

#[cfg(test)]
thread_local! {
    static TEST_FAILPOINT: std::cell::Cell<Option<TestFailpoint>> = const {
        std::cell::Cell::new(None)
    };
    static TEST_GENERATION_TIME_NANOS: std::cell::Cell<Option<u128>> = const {
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
struct TestGenerationTimeGuard {
    previous: Option<u128>,
}

#[cfg(test)]
impl TestGenerationTimeGuard {
    fn set(nanos: u128) -> Self {
        let previous = TEST_GENERATION_TIME_NANOS.with(|value| value.replace(Some(nanos)));
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for TestGenerationTimeGuard {
    fn drop(&mut self) {
        TEST_GENERATION_TIME_NANOS.with(|value| value.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn arbitrary_user_index_is_refused_without_creating_owned_paths() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("index.html"), "<!doctype html><h1>User site</h1>").unwrap();

        let error = publish_html_bundle(&root, |stage| {
            fs::write(stage.join("index.html"), "new")?;
            Ok::<(), io::Error>(())
        })
        .unwrap_err();

        assert!(matches!(error, PublicationError::UnownedIndex { .. }));
        assert_eq!(
            fs::read_to_string(root.join("index.html")).unwrap(),
            "<!doctype html><h1>User site</h1>"
        );
        assert!(!root.join(HTML_GENERATIONS_DIRECTORY).exists());
        assert!(!root.join(WRITER_LOCK_FILE).exists());
    }

    #[test]
    fn preexisting_empty_final_generation_path_is_refused_and_preserved() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let generations = root.join(HTML_GENERATIONS_DIRECTORY);
        fs::create_dir_all(&generations).unwrap();
        fs::write(root.join("peer.txt"), "keep").unwrap();

        let error = publish_html_bundle(&root, |stage| {
            fs::write(stage.join("index.html"), "new")?;
            Ok::<(), io::Error>(())
        })
        .unwrap_err();

        assert!(matches!(
            error,
            PublicationError::UnownedGenerationRoot { .. }
        ));
        assert!(generations.is_dir());
        assert!(fs::read_dir(&generations).unwrap().next().is_none());
        assert_eq!(fs::read_to_string(root.join("peer.txt")).unwrap(), "keep");
    }

    #[test]
    fn invalid_final_generation_marker_is_refused_and_preserved_byte_for_byte() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let generations = root.join(HTML_GENERATIONS_DIRECTORY);
        fs::create_dir_all(&generations).unwrap();
        let invalid = b"{\"schema\":";
        fs::write(generations.join(OWNERSHIP_MARKER_FILE), invalid).unwrap();

        let error = publish_html_bundle(&root, |stage| {
            fs::write(stage.join("index.html"), "new")?;
            Ok::<(), io::Error>(())
        })
        .unwrap_err();

        assert!(matches!(
            error,
            PublicationError::UnownedGenerationRoot { .. }
        ));
        assert_eq!(
            fs::read(generations.join(OWNERSHIP_MARKER_FILE)).unwrap(),
            invalid
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_root_index_is_refused_without_following_it() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        fs::create_dir(&root).unwrap();
        let target = root.join("user.html");
        fs::write(&target, "<html>User content</html>").unwrap();
        symlink(&target, root.join("index.html")).unwrap();

        let error = publish_html_bundle(&root, |_stage| Ok::<(), io::Error>(())).unwrap_err();

        assert!(matches!(error, PublicationError::UnownedIndex { .. }));
        assert!(
            fs::symlink_metadata(root.join("index.html"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "<html>User content</html>"
        );
    }

    #[test]
    fn old_generations_are_retained_after_pointer_replacement() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let first = publish_text_bundle(&root, "first");
        let first_pointer = fs::read(root.join("index.html")).unwrap();

        let second = publish_text_bundle(&root, "second");

        assert!(first.generation_path.is_dir());
        assert!(second.generation_path.is_dir());
        assert_eq!(
            fs::read_to_string(&first.presentation_index).unwrap(),
            "first"
        );
        let retired = fs::read(first.generation_path.join("index.html")).unwrap();
        assert_eq!(
            parse_retired_generation_pointer(&retired).as_deref(),
            Some(first.generation.as_str())
        );
        let retired = String::from_utf8(retired).unwrap();
        assert!(retired.contains("../../index.html"));
        assert!(retired.contains("window.location.search + window.location.hash"));
        assert!(retired.contains("http-equiv=\"refresh\" content=\"1;url=../../index.html\""));
        assert_eq!(
            fs::read_to_string(second.generation_path.join("index.html")).unwrap(),
            "second"
        );
        assert_eq!(
            fs::read_to_string(&second.presentation_index).unwrap(),
            "second"
        );
        assert_eq!(
            second.previous_generation.as_deref(),
            Some(first.generation.as_str())
        );
        assert_ne!(fs::read(root.join("index.html")).unwrap(), first_pointer);
        assert_eq!(current_generation(&root), second.generation);
        assert!(!root.join(WRITER_LOCK_FILE).exists());
        assert!(
            root.join(HTML_GENERATIONS_DIRECTORY)
                .join(WRITER_LOCK_FILE)
                .is_file()
        );
    }

    #[test]
    fn rapid_generations_respect_age_floor_then_collect_to_count_bound() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let base = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        {
            let _clock = TestGenerationTimeGuard::set(base);
            for build in 0..(MAX_RETAINED_HTML_GENERATIONS + 4) {
                publish_text_bundle(&root, &format!("rapid-{build}"));
            }
        }
        assert_eq!(
            completed_generation_paths(&root).len(),
            MAX_RETAINED_HTML_GENERATIONS + 4,
            "the minimum-age floor must win over the count limit during a rapid burst"
        );

        let current = {
            let aged = base + MIN_GENERATION_RETENTION_AGE.as_nanos() + 1_000_000_000;
            let _clock = TestGenerationTimeGuard::set(aged);
            publish_text_bundle(&root, "after-age-floor")
        };

        assert_eq!(
            completed_generation_paths(&root).len(),
            MAX_RETAINED_HTML_GENERATIONS
        );
        assert!(current.generation_path.is_dir());
        assert_eq!(current_generation(&root), current.generation);
    }

    #[test]
    fn writer_lock_is_nested_and_same_named_root_sibling_is_preserved() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(WRITER_LOCK_FILE), "belongs to the user").unwrap();

        publish_text_bundle(&root, "presentation");

        assert_eq!(
            fs::read_to_string(root.join(WRITER_LOCK_FILE)).unwrap(),
            "belongs to the user"
        );
        assert!(
            root.join(HTML_GENERATIONS_DIRECTORY)
                .join(WRITER_LOCK_FILE)
                .is_file()
        );
    }

    #[test]
    fn zero_byte_root_pointer_stage_lookalike_survives_rebuild() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        publish_text_bundle(&root, "first");
        let lookalike = root.join(".zpres-index-stage-user.tmp");
        File::create(&lookalike).unwrap();

        publish_text_bundle(&root, "second");

        let metadata = fs::symlink_metadata(&lookalike).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.len(), 0);
    }

    #[test]
    fn permanent_generation_has_complete_marker_but_no_stage_marker() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");

        let publication = publish_text_bundle(&root, "presentation");

        assert!(
            !publication
                .generation_path
                .join(HTML_STAGE_MARKER_FILE)
                .exists()
        );
        let marker: GenerationMarker = read_json_marker(
            &publication
                .generation_path
                .join(HTML_GENERATION_MARKER_FILE),
        )
        .unwrap();
        assert_eq!(marker.schema, SCHEMA);
        assert_eq!(marker.generator, GENERATOR);
        assert_eq!(marker.kind, GENERATION_MARKER_KIND);
        assert_eq!(marker.generation, publication.generation);
    }

    #[test]
    fn failed_population_leaves_old_pointer_and_removes_partial_stage() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let first = publish_text_bundle(&root, "first");
        let old_pointer = fs::read(root.join("index.html")).unwrap();

        let error = publish_html_bundle(&root, |stage| {
            fs::write(stage.join("index.html"), "partial")?;
            Err(io::Error::other("populate failed"))
        })
        .unwrap_err();

        assert!(matches!(error, PublicationError::Populate { .. }));
        assert_eq!(fs::read(root.join("index.html")).unwrap(), old_pointer);
        assert_eq!(current_generation(&root), first.generation);
        assert!(generation_stage_paths(&root).is_empty());
    }

    #[test]
    fn canonical_crash_stage_is_recovered_by_next_writer() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        publish_text_bundle(&root, "first");
        let leaked_path = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::LeakGenerationStage);
            let error = publish_html_bundle(&root, |stage| {
                fs::write(stage.join("index.html"), "leaked")?;
                Ok::<(), io::Error>(())
            })
            .unwrap_err();
            match error {
                PublicationError::InjectedCrash { path } => path,
                error => panic!("unexpected error: {error}"),
            }
        };
        assert!(leaked_path.is_dir());

        let publication = publish_text_bundle(&root, "second");

        assert!(!leaked_path.exists());
        assert!(
            publication
                .warnings
                .iter()
                .any(|warning| warning.contains("removed abandoned zpres HTML generation stage"))
        );
        assert!(generation_stage_paths(&root).is_empty());
    }

    #[test]
    fn partial_generation_stage_marker_is_recovered_by_next_writer() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        publish_text_bundle(&root, "first");
        let leaked_path = {
            let _failpoint =
                TestFailpointGuard::set(TestFailpoint::LeakPartialGenerationStageMarker);
            let error = publish_html_bundle(&root, |_stage| Ok::<(), io::Error>(())).unwrap_err();
            match error {
                PublicationError::InjectedCrash { path } => path,
                error => panic!("unexpected error: {error}"),
            }
        };
        assert!(leaked_path.is_dir());
        assert!(
            !fs::read(leaked_path.join(HTML_STAGE_MARKER_FILE))
                .unwrap()
                .is_empty()
        );

        let publication = publish_text_bundle(&root, "second");

        assert!(!leaked_path.exists());
        assert!(
            publication
                .warnings
                .iter()
                .any(|warning| warning.contains("removed abandoned zpres HTML generation stage"))
        );
    }

    #[test]
    fn partial_pointer_stage_is_recovered_by_next_writer() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let first = publish_text_bundle(&root, "first");
        {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::LeakPartialPointerStage);
            let error = publish_html_bundle(&root, |stage| {
                fs::write(stage.join("index.html"), "interrupted")?;
                Ok::<(), io::Error>(())
            })
            .unwrap_err();
            assert!(matches!(error, PublicationError::InjectedCrash { .. }));
        }
        assert_eq!(current_generation(&root), first.generation);
        assert!(!pointer_stage_paths(&root).is_empty());

        let publication = publish_text_bundle(&root, "second");

        assert!(pointer_stage_paths(&root).is_empty());
        assert!(
            publication
                .warnings
                .iter()
                .any(|warning| warning.contains("removed abandoned zpres HTML pointer stage"))
        );
    }

    #[test]
    fn committed_orphan_is_retired_after_next_successful_publication() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        publish_text_bundle(&root, "first");
        let orphan = {
            let _failpoint =
                TestFailpointGuard::set(TestFailpoint::AfterGenerationCommitBeforePointer);
            let error = publish_html_bundle(&root, |stage| {
                fs::write(stage.join("index.html"), "orphan")?;
                Ok::<(), io::Error>(())
            })
            .unwrap_err();
            match error {
                PublicationError::InjectedCrash { path } => path,
                error => panic!("unexpected error: {error}"),
            }
        };
        assert_eq!(
            fs::read_to_string(orphan.join(HTML_PRESENTATION_FILE)).unwrap(),
            "orphan"
        );

        let current = publish_text_bundle(&root, "second");

        let orphan_generation = orphan.file_name().unwrap().to_str().unwrap();
        assert_eq!(
            parse_retired_generation_pointer(&fs::read(orphan.join("index.html")).unwrap())
                .as_deref(),
            Some(orphan_generation)
        );
        assert_eq!(current_generation(&root), current.generation);
    }

    #[test]
    fn concurrent_reader_observes_only_complete_old_or_new_generation() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        publish_text_bundle(&root, "old");

        let done = Arc::new(AtomicBool::new(false));
        let failures = Arc::new(Mutex::new(Vec::<String>::new()));
        let reader_root = root.clone();
        let reader_done = Arc::clone(&done);
        let reader_failures = Arc::clone(&failures);
        let reader = thread::spawn(move || {
            while !reader_done.load(AtomicOrdering::Acquire) {
                check_pointer_snapshot(&reader_root, &reader_failures);
                thread::yield_now();
            }
            check_pointer_snapshot(&reader_root, &reader_failures);
        });

        let publication = publish_html_bundle(&root, |stage| {
            fs::write(stage.join("index.html"), "new")?;
            thread::sleep(Duration::from_millis(20));
            Ok::<(), io::Error>(())
        })
        .unwrap();
        done.store(true, AtomicOrdering::Release);
        reader.join().unwrap();

        assert!(
            failures.lock().unwrap().is_empty(),
            "reader failures: {:?}",
            failures.lock().unwrap()
        );
        assert_eq!(current_generation(&root), publication.generation);
    }

    #[test]
    fn pointer_preserves_query_and_hash_without_external_dependencies() {
        let generation = "g-00000000000000000000000000000001-00000001-0000000000000001";
        let pointer = render_pointer(generation);

        assert!(pointer.contains("window.location.search + window.location.hash"));
        assert!(pointer.contains(&pointer_target(generation)));
        assert!(pointer.contains("http-equiv=\"refresh\" content=\"1;url="));
        assert!(!pointer.contains("<script src="));
        assert_eq!(
            parse_canonical_pointer(pointer.as_bytes()).as_deref(),
            Some(generation)
        );
    }

    #[test]
    fn current_publication_resolver_is_read_only_and_validates_complete_generation() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        assert!(current_html_publication(&root).unwrap().is_none());
        assert!(!root.exists());

        let publication = publish_text_bundle(&root, "presentation");
        let current = current_html_publication(&root).unwrap().unwrap();

        assert_eq!(current.generation, publication.generation);
        assert_eq!(current.generation_path, publication.generation_path);
        assert_eq!(current.presentation_index, publication.presentation_index);
    }

    #[test]
    fn current_publication_resolver_rejects_missing_completion_marker() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let publication = publish_text_bundle(&root, "presentation");
        fs::remove_file(
            publication
                .generation_path
                .join(HTML_GENERATION_MARKER_FILE),
        )
        .unwrap();

        let error = current_html_publication(&root).unwrap_err();

        assert!(matches!(
            error,
            PublicationError::UnownedGenerationRoot { .. }
        ));
    }

    #[test]
    fn cooperative_writer_lock_is_nonblocking_and_os_released() {
        let temp = tempdir().unwrap();
        let root = prepare_output_root(&temp.path().join("dist")).unwrap();
        let generations_root = ensure_generation_root(&root).unwrap();
        let lock = WriterLock::acquire(&generations_root).unwrap();

        let error = publish_html_bundle(&root, |stage| {
            fs::write(stage.join("index.html"), "blocked")?;
            Ok::<(), io::Error>(())
        })
        .unwrap_err();
        assert!(matches!(
            error,
            PublicationError::ConcurrentPublication { .. }
        ));

        drop(lock);
        let publication = publish_text_bundle(&root, "after unlock");
        assert_eq!(
            fs::read_to_string(publication.presentation_index).unwrap(),
            "after unlock"
        );
    }

    #[test]
    fn postcommit_cleanup_failure_keeps_new_pointer_and_owned_old_temp() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        let first = publish_text_bundle(&root, "first");
        let publication = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::PostCommitPointerCleanup);
            publish_text_bundle(&root, "second")
        };

        assert_ne!(publication.generation, first.generation);
        assert_eq!(current_generation(&root), publication.generation);
        let retained = publication.retained_replaced_index.unwrap();
        assert!(retained.is_file());
        assert_eq!(
            parse_canonical_pointer(&fs::read(retained).unwrap()).unwrap(),
            first.generation
        );
    }

    #[test]
    fn postexchange_sync_failure_retains_old_index_and_skips_retirement() {
        for failpoint in [
            TestFailpoint::PostExchangeRootSync,
            TestFailpoint::PostExchangeStageSync,
        ] {
            let temp = tempdir().unwrap();
            let root = temp.path().join("dist");
            let first = publish_text_bundle(&root, "first");
            let publication = {
                let _failpoint = TestFailpointGuard::set(failpoint);
                publish_text_bundle(&root, "second")
            };

            assert_eq!(current_generation(&root), publication.generation);
            assert_eq!(
                fs::read_to_string(first.generation_path.join("index.html")).unwrap(),
                "first"
            );
            assert!(publication.retained_replaced_index.is_some());
            assert!(publication.warnings.iter().any(|warning| {
                warning.contains("skipped generation retirement and collection")
            }));
        }
    }

    #[test]
    fn failed_recovery_stabilization_preserves_prior_exchange_counterpart() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("dist");
        publish_text_bundle(&root, "first");
        let retained = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::PostExchangeRootSync);
            publish_text_bundle(&root, "second")
                .retained_replaced_index
                .unwrap()
        };
        assert!(retained.is_file());

        let third = {
            let _failpoint = TestFailpointGuard::set(TestFailpoint::RecoveryStabilizationSync);
            publish_text_bundle(&root, "third")
        };

        assert!(retained.is_file());
        assert!(
            third
                .warnings
                .iter()
                .any(|warning| { warning.contains("prior exchanges could not be stabilized") })
        );

        publish_text_bundle(&root, "fourth");
        assert!(!retained.exists());
    }

    fn publish_text_bundle(root: &Path, text: &str) -> HtmlPublication {
        publish_html_bundle(root, |stage| {
            fs::write(stage.join("index.html"), text)?;
            Ok::<(), io::Error>(())
        })
        .unwrap()
    }

    fn current_generation(root: &Path) -> String {
        parse_canonical_pointer(&fs::read(root.join("index.html")).unwrap()).unwrap()
    }

    fn generation_stage_paths(root: &Path) -> Vec<PathBuf> {
        fs::read_dir(root.join(HTML_GENERATIONS_DIRECTORY))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(GENERATION_STAGE_PREFIX))
            })
            .collect()
    }

    fn completed_generation_paths(root: &Path) -> Vec<PathBuf> {
        fs::read_dir(root.join(HTML_GENERATIONS_DIRECTORY))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(valid_generation_id)
            })
            .collect()
    }

    fn pointer_stage_paths(root: &Path) -> Vec<PathBuf> {
        fs::read_dir(root.join(HTML_GENERATIONS_DIRECTORY))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(POINTER_STAGE_PREFIX))
            })
            .collect()
    }

    fn check_pointer_snapshot(root: &Path, failures: &Mutex<Vec<String>>) {
        let bytes = match fs::read(root.join("index.html")) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures
                    .lock()
                    .unwrap()
                    .push(format!("root pointer was unavailable: {error}"));
                return;
            }
        };
        let Some(generation) = parse_canonical_pointer(&bytes) else {
            failures
                .lock()
                .unwrap()
                .push("root index was not a complete canonical pointer".to_string());
            return;
        };
        let index = root
            .join(HTML_GENERATIONS_DIRECTORY)
            .join(&generation)
            .join("index.html");
        match fs::read(&index) {
            Ok(content) if content == b"old" || content == b"new" => {}
            Ok(content)
                if parse_retired_generation_pointer(&content).as_deref()
                    == Some(generation.as_str()) =>
            {
                let current = fs::read(root.join("index.html"))
                    .ok()
                    .and_then(|bytes| parse_canonical_pointer(&bytes));
                let current_content = current.and_then(|current| {
                    fs::read(
                        root.join(HTML_GENERATIONS_DIRECTORY)
                            .join(current)
                            .join("index.html"),
                    )
                    .ok()
                });
                if current_content.as_deref() != Some(b"new") {
                    failures.lock().unwrap().push(
                        "retired generation did not lead to the complete current generation"
                            .to_string(),
                    );
                }
            }
            Ok(content) => failures.lock().unwrap().push(format!(
                "reader observed unexpected generation content {:?}",
                String::from_utf8_lossy(&content)
            )),
            Err(error) => failures
                .lock()
                .unwrap()
                .push(format!("pointer named an incomplete generation: {error}")),
        }
    }
}
