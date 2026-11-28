//! Read-only archive snapshots shared by the GUI and standalone CLI.

mod names;
mod provider;

#[cfg(test)]
mod provider_tests;

pub use names::*;
pub use provider::{open_archive, preview_entry, reinterpret};

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

/// Central-directory ordinal, scoped to one `ArchiveSnapshot::id`.
pub type EntryId = u64;

#[derive(Clone, Debug, Serialize)]
pub struct ArchiveEntry {
    pub id: EntryId,
    pub name: NameResolution,
    pub raw_name_hex: String,
    pub raw_extra_hex: String,
    pub raw_comment_hex: String,
    pub flags: u16,
    pub compressed_size: u64,
    pub size: u64,
    pub compression_method: u16,
    pub crc32: u32,
    pub is_directory: bool,
    pub encrypted: bool,
    pub local_header_offset: u64,
    pub modified: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArchiveSnapshot {
    pub id: String,
    /// SHA-256 of source length, directory offset, and original central-directory
    /// plus footer bytes. Reproducible across opens; it is not a payload hash.
    pub metadata_revision: String,
    pub source: String,
    pub entries: Vec<ArchiveEntry>,
    /// Sum of compressed entry sizes, not the source file's length.
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub comment_hex: String,
    pub warnings: Vec<String>,
    pub policy: FilenamePolicy,
    #[serde(skip)]
    source_state: Arc<provider::SourceState>,
}

impl ArchiveSnapshot {
    /// Original native path; the display/JSON path is never reopened instead.
    pub fn path(&self) -> &Path {
        &self.source_state.path
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EntryPreview {
    pub entry_id: EntryId,
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub data_hex: String,
    /// Present only when the complete payload is valid UTF-8.
    pub text: Option<String>,
    pub crc32_verified: bool,
}

#[derive(Clone, Debug)]
pub struct ArchiveLimits {
    pub max_entries: u64,
    pub max_central_directory_bytes: u64,
    /// Original central-directory bytes plus the archive comment. Parsed models
    /// and Unicode/hex presentations have additional entry/byte-bounded overhead.
    pub max_metadata_bytes: u64,
    /// Cumulative physical reads, including repeated metadata verification.
    pub max_read_bytes: u64,
    pub max_preview_bytes: u64,
    pub max_preview_compressed_bytes: u64,
    /// Cooperative wall-clock limit; it cannot interrupt an OS-blocked file read.
    pub operation_timeout: Duration,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_central_directory_bytes: 32 * 1024 * 1024,
            max_metadata_bytes: 32 * 1024 * 1024,
            max_read_bytes: 256 * 1024 * 1024,
            max_preview_bytes: 256 * 1024,
            max_preview_compressed_bytes: 16 * 1024 * 1024,
            operation_timeout: Duration::from_secs(15),
        }
    }
}

/// Cooperative cancellation, checked between bounded reads and metadata records.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<(), ArchiveError> {
        if self.is_cancelled() {
            Err(ArchiveError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum ArchiveError {
    Io(String),
    InvalidArchive(String),
    Unsupported(String),
    LimitExceeded(String),
    Cancelled,
    SourceChanged,
    EntryNotFound(EntryId),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "archive I/O failed: {message}"),
            Self::InvalidArchive(message) => write!(f, "invalid ZIP archive: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported archive operation: {message}"),
            Self::LimitExceeded(message) => write!(f, "archive limit exceeded: {message}"),
            Self::Cancelled => f.write_str("archive operation cancelled"),
            Self::SourceChanged => {
                f.write_str("archive source changed; reopen it before continuing")
            }
            Self::EntryNotFound(id) => {
                write!(f, "entry {id} does not belong to this archive snapshot")
            }
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<std::io::Error> for ArchiveError {
    fn from(error: std::io::Error) -> Self {
        if let Some(typed) = error
            .get_ref()
            .and_then(|error| error.downcast_ref::<ArchiveError>())
        {
            return typed.clone();
        }
        match error.kind() {
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData => {
                Self::InvalidArchive(error.to_string())
            }
            _ => Self::Io(error.to_string()),
        }
    }
}

impl From<zip::result::ZipError> for ArchiveError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(error) => Self::from(error),
            zip::result::ZipError::UnsupportedArchive(message) => Self::Unsupported(message.into()),
            other => Self::InvalidArchive(other.to_string()),
        }
    }
}
