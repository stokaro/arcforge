//! A bounded, read-only adapter around the pinned zip crate's duplicate-preserving
//! iterator. The small header reads retain original evidence and validate the
//! allocation/range boundaries; the dependency owns ZIP metadata and decoding.

mod headers;
mod io;
mod rewrite;
mod stream;

#[cfg(test)]
pub(super) use rewrite::rewrite as save_with_publish_hook;
pub use rewrite::{SavedZip, ZipEditReport, save_zip_without_entries, supports_zip_removal};
pub use stream::{ArchiveStream, begin_stream};

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use same_file::Handle;
use sha2::{Digest, Sha256};
use zip::read::{ArchiveOffset, Config, OwnedZipFileEntry};
use zip::unstable::read::ZipIterable;
use zip::{HasZipMetadata, ZipReadOptions};

use super::{
    ArchiveEntry, ArchiveError, ArchiveLimits, ArchiveSnapshot, CancellationToken, EntryId,
    EntryKind, EntryPreview, FilenameChoice, FilenamePolicy, NameProvenance, NameState,
    resolve_name,
};
use headers::{
    CentralRecord, Footer, checked_end, invalid, preflight, read_central, u16_at, u32_at, u64_at,
    zip64_extra, zip64_value,
};
use io::BudgetReader;

static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(super) struct SourceState {
    pub path: PathBuf,
    // Keeping this original handle alive prevents filesystem identity reuse.
    identity: Handle,
    stamp: SourceStamp,
    fingerprint: [u8; 32],
    footer: Footer,
    records: Vec<ProviderRecord>,
}

#[derive(Debug)]
struct ProviderRecord {
    raw: CentralRecord,
    metadata: OwnedZipFileEntry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl SourceStamp {
    fn from_metadata(metadata: Metadata) -> Result<Self, ArchiveError> {
        if !metadata.is_file() {
            return Err(ArchiveError::Unsupported(
                "source is not a regular file".into(),
            ));
        }
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// Open a metadata snapshot without decompressing entries or modifying the source.
pub fn open_archive(
    path: &Path,
    policy: &FilenamePolicy,
    limits: &ArchiveLimits,
    cancel: &CancellationToken,
) -> Result<ArchiveSnapshot, ArchiveError> {
    #[cfg(test)]
    {
        open_archive_impl(path, policy, limits, cancel, None)
    }
    #[cfg(not(test))]
    {
        open_archive_impl(path, policy, limits, cancel)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OpenTestStage {
    BeforeFreeze,
    BeforeDiscovery,
}

#[cfg(test)]
type MetadataMutationHook<'a> = &'a dyn Fn(OpenTestStage, &Path);

#[cfg(test)]
pub(super) fn open_archive_with_metadata_hook(
    path: &Path,
    policy: &FilenamePolicy,
    limits: &ArchiveLimits,
    cancel: &CancellationToken,
    hook: MetadataMutationHook<'_>,
) -> Result<ArchiveSnapshot, ArchiveError> {
    open_archive_impl(path, policy, limits, cancel, Some(hook))
}

fn open_archive_impl(
    path: &Path,
    policy: &FilenamePolicy,
    limits: &ArchiveLimits,
    cancel: &CancellationToken,
    #[cfg(test)] mutation_hook: Option<MetadataMutationHook<'_>>,
) -> Result<ArchiveSnapshot, ArchiveError> {
    cancel.check()?;
    validate_policy(policy, None)?;
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    SourceStamp::from_metadata(fs::metadata(&path)?)?;
    let file = File::open(&path)?;
    let stamp = SourceStamp::from_metadata(file.metadata()?)?;
    let identity = Handle::from_file(file.try_clone()?)?;
    let mut reader = BudgetReader::new(file, stamp.len, limits, cancel);
    let footer = preflight(&mut reader, stamp.len, limits)?;
    validate_policy(policy, Some(footer.count))?;
    #[cfg(test)]
    if let Some(hook) = mutation_hook {
        hook(OpenTestStage::BeforeFreeze, &path);
    }
    reader.freeze_metadata(footer.directory_offset, footer.eocd_offset)?;
    // Checking the live file once is insufficient: it may change between that
    // check and a dependency allocation. Validate exactly the immutable bytes
    // that the dependency will see, then reject any changed directory layout.
    if preflight(&mut reader, stamp.len, limits)? != footer {
        return Err(ArchiveError::SourceChanged);
    }
    let fingerprint = metadata_fingerprint(&mut reader, &footer)?;
    check_source_identity(&reader, &path, &identity, &stamp)?;
    reader.directory_view(footer.eocd_offset);
    #[cfg(test)]
    if let Some(hook) = mutation_hook {
        hook(OpenTestStage::BeforeDiscovery, &path);
    }
    let mut archive = ZipIterable::try_new(
        reader,
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
    )?;
    if archive.len() as u64 != footer.count {
        return invalid("provider selected a different central directory");
    }
    let count = usize::try_from(footer.count).map_err(|_| {
        ArchiveError::LimitExceeded("entry count does not fit this platform".into())
    })?;
    let mut records = Vec::new();
    records.try_reserve(count).map_err(|_| {
        ArchiveError::LimitExceeded("could not reserve bounded entry metadata".into())
    })?;
    let files = archive.files()?;
    let directory_end = footer.directory_offset + footer.directory_size;
    let mut next_offset = footer.directory_offset;
    for _ in 0..count {
        files.reader_mut().check()?;
        let raw = read_central(files.reader_mut(), next_offset, directory_end)?;
        let metadata: OwnedZipFileEntry = files
            .next()
            .ok_or_else(|| ArchiveError::InvalidArchive("central directory ended early".into()))??
            .into();
        if metadata.central_header_start() != next_offset
            || metadata.header_start() != raw.local_offset
            || metadata.size() != raw.size
            || metadata.compressed_size() != raw.compressed_size
            || metadata.crc32() != raw.crc32()
        {
            return invalid("provider metadata disagrees with original central-header fields");
        }
        checked_end(
            raw.local_offset,
            30,
            footer.directory_offset,
            "local-header offset",
        )?;
        checked_end(
            raw.local_offset,
            raw.compressed_size,
            footer.directory_offset,
            "entry extent",
        )?;
        next_offset = raw.next_offset;
        records.push(ProviderRecord { raw, metadata });
    }
    if next_offset != directory_end || files.next().is_some() {
        return invalid("central-directory count and byte extent disagree");
    }
    files.reader_mut().resume_file_reads()?;
    check_source_identity(files.reader_mut(), &path, &identity, &stamp)?;
    if metadata_fingerprint(files.reader_mut(), &footer)? != fingerprint {
        return Err(ArchiveError::SourceChanged);
    }
    check_source_identity(files.reader_mut(), &path, &identity, &stamp)?;
    let name_timeout = files.reader_mut().remaining_time();
    let source_state = Arc::new(SourceState {
        path,
        identity,
        stamp,
        fingerprint,
        footer,
        records,
    });
    let (entries, compressed_size, uncompressed_size) =
        build_entries(&source_state.records, policy, cancel, name_timeout)?;
    let sequence = SNAPSHOT_SEQUENCE
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| ArchiveError::LimitExceeded("snapshot identifier space exhausted".into()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = format!("{:x}-{timestamp:x}-{sequence:x}", std::process::id());
    Ok(ArchiveSnapshot {
        id,
        metadata_revision: hex(&source_state.fingerprint),
        source: source_state.path.to_string_lossy().into_owned(),
        entries,
        compressed_size,
        uncompressed_size,
        comment_hex: hex(&source_state.footer.comment),
        warnings: snapshot_warnings(),
        policy: policy.clone(),
        source_state,
    })
}

/// Reinterpret retained original filename bytes without reopening or rewriting files.
/// Snapshot and entry identities stay unchanged, including after name collisions.
pub fn reinterpret(
    snapshot: &ArchiveSnapshot,
    policy: &FilenamePolicy,
    cancel: &CancellationToken,
) -> Result<ArchiveSnapshot, ArchiveError> {
    cancel.check()?;
    validate_policy(policy, Some(snapshot.source_state.records.len() as u64))?;
    let (entries, compressed_size, uncompressed_size) = build_entries(
        &snapshot.source_state.records,
        policy,
        cancel,
        ArchiveLimits::default().operation_timeout,
    )?;
    Ok(ArchiveSnapshot {
        id: snapshot.id.clone(),
        metadata_revision: hex(&snapshot.source_state.fingerprint),
        source: snapshot.source.clone(),
        entries,
        compressed_size,
        uncompressed_size,
        comment_hex: hex(&snapshot.source_state.footer.comment),
        warnings: snapshot_warnings(),
        policy: policy.clone(),
        source_state: Arc::clone(&snapshot.source_state),
    })
}

/// Read one complete small Stored/Deflate payload. Success requires matching
/// local metadata, output size, CRC at EOF, and unchanged source observations.
pub fn preview_entry(
    snapshot: &ArchiveSnapshot,
    entry_id: EntryId,
    limits: &ArchiveLimits,
    cancel: &CancellationToken,
) -> Result<EntryPreview, ArchiveError> {
    cancel.check()?;
    let source = &snapshot.source_state;
    let record = usize::try_from(entry_id)
        .ok()
        .and_then(|index| source.records.get(index))
        .ok_or(ArchiveError::EntryNotFound(entry_id))?;
    let raw = &record.raw;
    if is_directory(record) {
        return Err(ArchiveError::Unsupported(
            "directories have no file preview".into(),
        ));
    }
    if encrypted(record) {
        return Err(ArchiveError::Unsupported(
            "encrypted entry previews are not implemented".into(),
        ));
    }
    if !matches!(raw.method(), 0 | 8) {
        return Err(ArchiveError::Unsupported(format!(
            "preview supports Stored (0) and Deflate (8), entry uses method {}",
            raw.method(),
        )));
    }
    if raw.size > limits.max_preview_bytes {
        return Err(ArchiveError::LimitExceeded(format!(
            "entry needs {} bytes; whole-entry preview limit is {} bytes",
            raw.size, limits.max_preview_bytes,
        )));
    }
    if raw.compressed_size > limits.max_preview_compressed_bytes {
        return Err(ArchiveError::LimitExceeded(
            "compressed preview input bytes".into(),
        ));
    }
    if source.footer.directory_size > limits.max_central_directory_bytes
        || source.records.len() as u64 > limits.max_entries
        || source
            .footer
            .directory_size
            .checked_add(source.footer.comment.len() as u64)
            .is_none_or(|size| size > limits.max_metadata_bytes)
    {
        return Err(ArchiveError::LimitExceeded(
            "snapshot exceeds operation metadata limits".into(),
        ));
    }
    let mut reader = open_source_reader(source, limits, cancel)?;
    verify_source(&mut reader, source)?;
    let choice = snapshot.policy.choice_for(entry_id);
    let local = validate_local(
        &mut reader,
        record,
        source.footer.directory_offset,
        Some(choice),
    )?;
    // The library caches this offset. Publish only our checked value, so shared
    // previews cannot initialize its cache from different racing local headers.
    pin_data_start(record, local.data_start)?;
    let mut bytes = Vec::new();
    let capacity = usize::try_from(raw.size).map_err(|_| {
        ArchiveError::LimitExceeded("preview size does not fit this platform".into())
    })?;
    bytes.try_reserve_exact(capacity).map_err(|_| {
        ArchiveError::LimitExceeded("could not reserve bounded preview bytes".into())
    })?;
    {
        // Borrowing is deliberate: an owned ZipFile drains its remaining input
        // on Drop. A cancelled or failed bounded read must stop immediately.
        let mut file = record
            .metadata
            .as_ref()
            .with_reader(&mut reader, ZipReadOptions::new())?;
        let mut chunk = [0; 16 * 1024];
        loop {
            cancel.check()?;
            let count = file.read(&mut chunk)?;
            if count == 0 {
                break; // zip's CRC reader checks the complete payload at EOF.
            }
            let new_length = (bytes.len() as u64)
                .checked_add(count as u64)
                .ok_or_else(|| ArchiveError::LimitExceeded("preview output overflow".into()))?;
            if new_length > limits.max_preview_bytes || new_length > raw.size {
                return Err(ArchiveError::LimitExceeded(
                    "actual decompressed preview bytes".into(),
                ));
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
    }
    if bytes.len() as u64 != raw.size {
        return invalid("preview output is shorter than the declared entry size");
    }
    if raw.flags() & 8 != 0 {
        validate_descriptor(
            &mut reader,
            record,
            local.data_end,
            source.footer.directory_offset,
        )?;
    }
    let after = validate_local(
        &mut reader,
        record,
        source.footer.directory_offset,
        Some(choice),
    )?;
    if after != local {
        return Err(ArchiveError::SourceChanged);
    }
    verify_source(&mut reader, source)?;
    // A content encoding guess would be a separate policy. Only strict UTF-8 is
    // offered as text; callers always retain the complete binary/hex payload.
    let text = std::str::from_utf8(&bytes).ok().map(str::to_owned);
    let data_hex = hex(&bytes);
    reader.check()?;
    Ok(EntryPreview {
        entry_id,
        data_hex,
        bytes,
        text,
        crc32_verified: true,
    })
}

fn validate_policy(policy: &FilenamePolicy, count: Option<u64>) -> Result<(), ArchiveError> {
    policy
        .validate()
        .map_err(|message| ArchiveError::Unsupported(format!("filename policy: {message}")))?;
    if let Some(count) = count
        && let Some(id) = policy.entries.keys().find(|&&id| id >= count)
    {
        return Err(ArchiveError::EntryNotFound(*id));
    }
    Ok(())
}

fn build_entries(
    records: &[ProviderRecord],
    policy: &FilenamePolicy,
    cancel: &CancellationToken,
    timeout: std::time::Duration,
) -> Result<(Vec<ArchiveEntry>, u64, u64), ArchiveError> {
    let started = Instant::now();
    let mut entries = Vec::with_capacity(records.len());
    let mut compressed = 0_u64;
    let mut uncompressed = 0_u64;
    for (ordinal, record) in records.iter().enumerate() {
        cancel.check()?;
        if started.elapsed() >= timeout {
            return Err(ArchiveError::LimitExceeded(
                "filename interpretation time budget".into(),
            ));
        }
        let id = ordinal as u64;
        let raw = &record.raw;
        compressed = compressed
            .checked_add(raw.compressed_size)
            .ok_or_else(|| ArchiveError::InvalidArchive("compressed-size total overflow".into()))?;
        uncompressed = uncompressed.checked_add(raw.size).ok_or_else(|| {
            ArchiveError::InvalidArchive("uncompressed-size total overflow".into())
        })?;
        let mut warnings = Vec::new();
        let kind = entry_kind(record);
        if kind == EntryKind::Symlink {
            warnings.push("Symbolic-link metadata; preview reads its stored payload without following a target.".into());
        }
        if kind == EntryKind::Special {
            warnings.push(
                "Unsupported or conflicting entry type metadata; extraction is unavailable.".into(),
            );
        }
        entries.push(ArchiveEntry {
            id,
            name: resolve_name(&raw.name, raw.flags(), &raw.extra, policy.choice_for(id)),
            raw_name_hex: hex(&raw.name),
            raw_extra_hex: hex(&raw.extra),
            raw_comment_hex: hex(&raw.comment),
            flags: raw.flags(),
            compressed_size: raw.compressed_size,
            size: raw.size,
            compression_method: raw.method(),
            crc32: raw.crc32(),
            kind,
            unix_mode: raw_unix_mode(record),
            is_directory: is_directory(record),
            encrypted: encrypted(record),
            local_header_offset: raw.local_offset,
            modified: record.metadata.last_modified().map(|date| date.to_string()),
            warnings,
        });
    }
    add_collision_warnings(&mut entries, records, cancel, started, timeout)?;
    cancel.check()?;
    if started.elapsed() >= timeout {
        return Err(ArchiveError::LimitExceeded(
            "filename interpretation time budget".into(),
        ));
    }
    Ok((entries, compressed, uncompressed))
}

fn add_collision_warnings(
    entries: &mut [ArchiveEntry],
    records: &[ProviderRecord],
    cancel: &CancellationToken,
    started: Instant,
    timeout: std::time::Duration,
) -> Result<(), ArchiveError> {
    let mut raw_names = BTreeMap::new();
    let mut display_names = BTreeMap::new();
    let mut collisions = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        cancel.check()?;
        if started.elapsed() >= timeout {
            return Err(ArchiveError::LimitExceeded(
                "filename collision time budget".into(),
            ));
        }
        if let Some(&first) = raw_names.get(records[index].raw.name.as_slice()) {
            collisions.push((first, index, false));
        } else {
            raw_names.insert(records[index].raw.name.as_slice(), index);
        }
        if let Some(&first) = display_names.get(entry.name.display.as_str()) {
            collisions.push((first, index, true));
        } else {
            display_names.insert(entry.name.display.as_str(), index);
        }
    }
    for (first, current, interpreted) in collisions {
        cancel.check()?;
        if started.elapsed() >= timeout {
            return Err(ArchiveError::LimitExceeded(
                "filename collision time budget".into(),
            ));
        }
        let kind = if interpreted {
            "Interpreted filename"
        } else {
            "Original filename bytes"
        };
        let general = format!("{kind} repeats; records remain distinct by entry ID.");
        if !entries[first].warnings.contains(&general) {
            entries[first].warnings.push(general);
        }
        entries[current].warnings.push(format!(
            "{kind} also occurs at entry {first}; records remain distinct."
        ));
    }
    Ok(())
}

fn is_directory(record: &ProviderRecord) -> bool {
    entry_kind(record) == EntryKind::Directory
}

fn raw_unix_mode(record: &ProviderRecord) -> Option<u32> {
    let mode = u32_at(&record.raw.header, 38) >> 16;
    (mode != 0).then_some(mode)
}

fn entry_kind(record: &ProviderRecord) -> EntryKind {
    let attributes = u32_at(&record.raw.header, 38);
    // Volume labels, devices and Windows reparse points are not regular files.
    if attributes & 0x0448 != 0 {
        return EntryKind::Special;
    }
    let directory_hint = record.raw.name.ends_with(b"/") || attributes & 0x10 != 0;
    let file_type = raw_unix_mode(record).unwrap_or(0) & 0o170_000;
    match file_type {
        0o100_000 if directory_hint => EntryKind::Special,
        0o100_000 => EntryKind::File,
        0o040_000 => EntryKind::Directory,
        0o120_000 if directory_hint => EntryKind::Special,
        0o120_000 => EntryKind::Symlink,
        0 if directory_hint => EntryKind::Directory,
        0 => EntryKind::File,
        _ => EntryKind::Special,
    }
}

fn encrypted(record: &ProviderRecord) -> bool {
    record.raw.flags() & 0x41 != 0 || record.metadata.encrypted()
}

fn snapshot_warnings() -> Vec<String> {
    vec![
        "Read-only metadata snapshot. Local headers are checked when a selected entry is previewed or streamed.".into(),
        "Listing does not verify payloads. CRC is verified only for successful complete entry reads.".into(),
    ]
}

fn open_source_reader(
    source: &SourceState,
    limits: &ArchiveLimits,
    cancel: &CancellationToken,
) -> Result<BudgetReader, ArchiveError> {
    SourceStamp::from_metadata(
        fs::metadata(&source.path).map_err(|_| ArchiveError::SourceChanged)?,
    )
    .map_err(|_| ArchiveError::SourceChanged)?;
    let file = File::open(&source.path).map_err(|_| ArchiveError::SourceChanged)?;
    let current_identity = Handle::from_file(file.try_clone()?)?;
    if current_identity != source.identity {
        return Err(ArchiveError::SourceChanged);
    }
    Ok(BudgetReader::new(file, source.stamp.len, limits, cancel))
}

fn pin_data_start(record: &ProviderRecord, data_start: u64) -> Result<(), ArchiveError> {
    let cached_start = &record.metadata.get_metadata().data_start;
    if cached_start.set(data_start).is_err() && cached_start.get() != Some(&data_start) {
        return Err(ArchiveError::SourceChanged);
    }
    Ok(())
}

fn check_source_identity(
    reader: &BudgetReader,
    path: &Path,
    identity: &Handle,
    stamp: &SourceStamp,
) -> Result<(), ArchiveError> {
    reader.check()?;
    let current = SourceStamp::from_metadata(reader.file.metadata()?)?;
    let path_stamp = fs::metadata(path)
        .ok()
        .and_then(|value| SourceStamp::from_metadata(value).ok());
    if &current != stamp || path_stamp.as_ref() != Some(stamp) {
        return Err(ArchiveError::SourceChanged);
    }
    let path_identity = Handle::from_path(path).map_err(|_| ArchiveError::SourceChanged)?;
    if &path_identity != identity {
        return Err(ArchiveError::SourceChanged);
    }
    reader.check()?;
    Ok(())
}

fn verify_source(reader: &mut BudgetReader, source: &SourceState) -> Result<(), ArchiveError> {
    check_source_identity(reader, &source.path, &source.identity, &source.stamp)?;
    if metadata_fingerprint(reader, &source.footer)? != source.fingerprint {
        return Err(ArchiveError::SourceChanged);
    }
    check_source_identity(reader, &source.path, &source.identity, &source.stamp)
}

fn metadata_fingerprint(
    reader: &mut BudgetReader,
    footer: &Footer,
) -> Result<[u8; 32], ArchiveError> {
    let mut hash = Sha256::new();
    hash.update(footer.file_len.to_le_bytes());
    hash.update(footer.directory_offset.to_le_bytes());
    let mut remaining = footer.file_len - footer.directory_offset;
    reader.seek(SeekFrom::Start(footer.directory_offset))?;
    let mut chunk = [0; 64 * 1024];
    while remaining != 0 {
        reader.check()?;
        let size = usize::try_from(remaining.min(chunk.len() as u64))
            .map_err(|_| ArchiveError::LimitExceeded("metadata hash chunk size".into()))?;
        reader.read_exact(&mut chunk[..size])?;
        hash.update(&chunk[..size]);
        remaining -= size as u64;
    }
    Ok(hash.finalize().into())
}

#[derive(Debug, Eq, PartialEq)]
struct LocalEvidence {
    data_start: u64,
    data_end: u64,
    extra: Vec<u8>,
    fingerprint: [u8; 32],
}

fn validate_local(
    reader: &mut BudgetReader,
    record: &ProviderRecord,
    directory_offset: u64,
    choice: Option<FilenameChoice>,
) -> Result<LocalEvidence, ArchiveError> {
    let central = &record.raw;
    checked_end(central.local_offset, 30, directory_offset, "local header")?;
    let mut local = [0; 30];
    reader.read_at(central.local_offset, &mut local)?;
    if local[..4] != *b"PK\x03\x04" {
        return invalid("selected entry has no local file header");
    }
    if u16_at(&local, 6) != central.flags()
        || u16_at(&local, 8) != central.method()
        || u16_at(&local, 10) != u16_at(&central.header, 12)
        || u16_at(&local, 12) != u16_at(&central.header, 14)
    {
        return invalid(
            "selected local flags, method, or timestamp disagree with central metadata",
        );
    }
    let name_len = usize::from(u16_at(&local, 26));
    let extra_len = usize::from(u16_at(&local, 28));
    let data_start = checked_end(
        central.local_offset,
        30 + name_len as u64 + extra_len as u64,
        directory_offset,
        "local filename and extra fields",
    )?;
    let data_end = checked_end(
        data_start,
        central.compressed_size,
        directory_offset,
        "selected compressed data",
    )?;
    let mut name = vec![0; name_len];
    let mut extra = vec![0; extra_len];
    reader.read_at(central.local_offset + 30, &mut name)?;
    reader.read_at(central.local_offset + 30 + name_len as u64, &mut extra)?;
    if name != central.name {
        return invalid("selected local and central filename bytes disagree");
    }
    let zip64 = zip64_extra(&extra)?;
    let mut position = 0;
    let size = zip64_value(u32_at(&local, 22), zip64, &mut position)?;
    let compressed_size = zip64_value(u32_at(&local, 18), zip64, &mut position)?;
    let descriptor = central.flags() & 8 != 0;
    if (!descriptor || u32_at(&local, 14) != 0) && u32_at(&local, 14) != central.crc32()
        || (!descriptor || size != 0) && size != central.size
        || (!descriptor || compressed_size != 0) && compressed_size != central.compressed_size
    {
        return invalid("selected local CRC or sizes disagree with central metadata");
    }
    if let Some(choice) = choice {
        let local_name = resolve_name(&name, central.flags(), &extra, choice);
        let central_name = resolve_name(&central.name, central.flags(), &central.extra, choice);
        if local_name.state == NameState::Conflict || central_name.state == NameState::Conflict {
            return invalid("selected filename declarations conflict");
        }
        if local_name.provenance == NameProvenance::UnicodePath
            && local_name.display != central_name.display
        {
            return invalid(
                "local Unicode Path metadata disagrees with the displayed central-directory evidence",
            );
        }
    }
    let mut hash = Sha256::new();
    hash.update(local);
    hash.update(name);
    hash.update(&extra);
    Ok(LocalEvidence {
        data_start,
        data_end,
        extra,
        fingerprint: hash.finalize().into(),
    })
}

fn validate_descriptor(
    reader: &mut BudgetReader,
    record: &ProviderRecord,
    offset: u64,
    directory_offset: u64,
) -> Result<u64, ArchiveError> {
    let raw = &record.raw;
    let mut local_sizes = [0; 8];
    reader.read_at(raw.local_offset + 18, &mut local_sizes)?;
    let wide = raw.size >= u64::from(u32::MAX)
        || raw.compressed_size >= u64::from(u32::MAX)
        || u32_at(&local_sizes, 0) == u32::MAX
        || u32_at(&local_sizes, 4) == u32::MAX;
    let minimum = if wide { 20 } else { 12 };
    checked_end(offset, minimum, directory_offset, "data descriptor")?;
    let available = (directory_offset - offset).min(minimum + 4) as usize;
    let mut bytes = [0; 24];
    reader.read_at(offset, &mut bytes[..available])?;
    // A CRC may equal the optional descriptor signature. Accept either complete
    // matching layout, while always comparing CRC and both sizes to the directory.
    for start in [0, 4] {
        if start == 4 && bytes[..4] != *b"PK\x07\x08" {
            continue;
        }
        if available < start + minimum as usize {
            continue;
        }
        let crc = u32_at(&bytes, start);
        let (compressed, size) = if wide {
            (u64_at(&bytes, start + 4), u64_at(&bytes, start + 12))
        } else {
            (
                u64::from(u32_at(&bytes, start + 4)),
                u64::from(u32_at(&bytes, start + 8)),
            )
        };
        if crc == raw.crc32() && compressed == raw.compressed_size && size == raw.size {
            return Ok(offset + start as u64 + minimum);
        }
    }
    invalid("selected data descriptor disagrees with central metadata")
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        // Writing to a String is infallible.
        let _ = write!(text, "{byte:02x}");
    }
    text
}
