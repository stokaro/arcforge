//! Publish a classic ZIP copy with specified central-directory records removed.

use std::collections::BTreeSet;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use same_file::Handle;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::super::destination::{DestinationParent, directory_identity, private_directory_builder};
use super::super::{
    ArchiveError, ArchiveLimits, ArchiveSnapshot, CancellationToken, EntryId, StreamLimits,
};
use super::headers::{u16_at, u32_at};
use super::stream::check_streamable;
use super::{
    SourceStamp, begin_stream, check_source_identity, hex, open_archive, open_source_reader,
    validate_descriptor, validate_local, verify_source,
};

const STAGED_FILE: &str = "archive.zip";

#[derive(Clone, Debug, Serialize)]
pub struct ZipEditReport {
    pub destination: String,
    pub entries_removed: usize,
    pub entries_retained: usize,
    pub bytes_written: u64,
    pub retained_crc32_verified: bool,
}

#[derive(Debug)]
pub struct SavedZip {
    pub report: ZipEditReport,
    /// The verified copy, associated with its published native path.
    pub snapshot: ArchiveSnapshot,
}

/// Cheap eligibility check for the document controls. Saving also verifies
/// record layout, preserved fields, compression and destination capabilities.
pub fn supports_zip_removal(snapshot: &ArchiveSnapshot) -> bool {
    let source = &snapshot.source_state;
    let footer = &source.footer;
    footer.file_len < u64::from(u32::MAX)
        && footer.directory_offset + footer.directory_size == footer.eocd_offset
        && source.records.iter().all(|record| {
            let central = &record.raw.header;
            [20, 24, 42]
                .iter()
                .all(|&field| u32_at(central, field) != u32::MAX)
                && u16_at(central, 34) == 0
        })
}

/// Remove exact entry IDs from this snapshot and publish a new verified ZIP.
/// Only the final, atomic no-overwrite link creates the requested filename.
pub fn save_zip_without_entries(
    snapshot: &ArchiveSnapshot,
    removed: &BTreeSet<EntryId>,
    destination: &Path,
    limits: &StreamLimits,
    cancel: &CancellationToken,
) -> Result<SavedZip, ArchiveError> {
    rewrite(
        snapshot,
        removed,
        destination,
        limits,
        cancel,
        #[cfg(test)]
        || {},
    )
}

pub(in crate::archive) fn rewrite(
    snapshot: &ArchiveSnapshot,
    removed: &BTreeSet<EntryId>,
    destination: &Path,
    limits: &StreamLimits,
    cancel: &CancellationToken,
    #[cfg(test)] before_publish: impl FnOnce(),
) -> Result<SavedZip, ArchiveError> {
    cancel.check()?;
    let source = &snapshot.source_state;
    if !supports_zip_removal(snapshot) {
        return Err(ArchiveError::Unsupported(
            "Save As currently supports classic ZIP only".into(),
        ));
    }
    if removed.is_empty() {
        return Err(ArchiveError::Unsupported(
            "select at least one ZIP entry to delete".into(),
        ));
    }
    if let Some(&id) = removed.last().filter(|&&id| id >= source.footer.count) {
        return Err(ArchiveError::EntryNotFound(id));
    }
    if source.footer.count > limits.max_entries {
        return Err(ArchiveError::LimitExceeded(
            "ZIP rewrite entry count".into(),
        ));
    }
    let target = DestinationParent::open(
        destination,
        "choose a new ZIP filename under an existing parent",
    )?;
    match target.parent.symlink_metadata(&target.leaf) {
        Ok(_) => return Err(target.exists_error()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let read_limits = ArchiveLimits {
        max_read_bytes: limits.max_read_bytes,
        operation_timeout: limits.operation_timeout,
        ..ArchiveLimits::default()
    };
    let mut reader = open_source_reader(source, &read_limits, cancel)?;
    verify_source(&mut reader, source)?;

    // Physical order defines the exact extent of each local record. Central
    // directory ordering and IDs are independent of that order.
    let mut physical: Vec<_> = (0..source.records.len()).collect();
    physical.sort_unstable_by_key(|&index| source.records[index].raw.local_offset);
    let mut expected_start = 0;
    let mut retained_bytes = 0_u64;
    let mut regions = vec![None; physical.len()];
    for &index in &physical {
        reader.check()?;
        let record = &source.records[index];
        let raw = &record.raw;
        if raw.local_offset != expected_start {
            return Err(ArchiveError::Unsupported(
                "overlapping local records or unrecognized ZIP padding".into(),
            ));
        }
        let kept = !removed.contains(&(index as u64));
        let choice = kept.then(|| snapshot.policy.choice_for(index as u64));
        let local = validate_local(&mut reader, record, source.footer.directory_offset, choice)?;
        let end = if raw.flags() & 8 == 0 {
            local.data_end
        } else {
            validate_descriptor(
                &mut reader,
                record,
                local.data_end,
                source.footer.directory_offset,
            )?
        };
        if kept {
            check_streamable(record)?;
            retained_bytes = retained_bytes.checked_add(raw.size).ok_or_else(|| {
                ArchiveError::LimitExceeded("total size of retained ZIP entries".into())
            })?;
            if raw.size > limits.max_entry_bytes || retained_bytes > limits.max_total_bytes {
                return Err(ArchiveError::LimitExceeded(
                    "uncompressed size of retained ZIP entries".into(),
                ));
            }
            // Moving records is safe for these offset-independent extra fields.
            check_extra_fields(&local.extra)?;
            check_extra_fields(&raw.extra)?;
            if raw.flags() & !(0x0800 | 0x000e) != 0 {
                return Err(ArchiveError::Unsupported(
                    "retained entry uses ZIP flags that cannot be saved".into(),
                ));
            }
            regions[index] = Some((local.data_start, local.fingerprint, end));
        }
        expected_start = end;
    }
    if expected_start != source.footer.directory_offset {
        return Err(ArchiveError::Unsupported(
            "unrecognized data before ZIP central directory".into(),
        ));
    }
    check_source_identity(&reader, &source.path, &source.identity, &source.stamp)?;

    let staging = StagingZip::new(&target)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = staging.directory.open_with(STAGED_FILE, &options)?;
    let identity = Handle::from_file(file.try_clone()?.into_std())?;
    let mut writer = BufWriter::new(file);
    let mut directory_offset = 0_u64;
    let mut new_offsets = vec![None; regions.len()];
    let mut local_header = Vec::new();
    for index in physical {
        let Some((data_start, fingerprint, end)) = regions[index] else {
            continue;
        };
        let start = source.records[index].raw.local_offset;
        let header_len = (data_start - start) as usize; // 30 fixed bytes and two u16 lengths
        local_header.resize(header_len, 0);
        reader.read_at(start, &mut local_header)?;
        let actual: [u8; 32] = Sha256::digest(&local_header).into();
        if actual != fingerprint {
            return Err(ArchiveError::SourceChanged);
        }
        writer.write_all(&local_header)?;
        let remaining = end - data_start;
        if io::copy(&mut (&mut reader).take(remaining), &mut writer)? != remaining {
            return Err(ArchiveError::SourceChanged);
        }
        new_offsets[index] = Some(directory_offset as u32);
        // Disjoint input regions were bounded by the classic source length.
        directory_offset += end - start;
    }
    let mut directory_size = 0_u64;
    let mut count = 0_u16;
    let mut policy = snapshot.policy.clone();
    policy.entries.clear();
    for (index, offset) in new_offsets.into_iter().enumerate() {
        reader.check()?;
        let Some(offset) = offset else { continue };
        let raw = &source.records[index].raw;
        let mut header = raw.header;
        header[42..46].copy_from_slice(&offset.to_le_bytes());
        for bytes in [&header[..], &raw.name, &raw.extra, &raw.comment] {
            writer.write_all(bytes)?;
            directory_size += bytes.len() as u64;
        }
        if let Some(choice) = snapshot.policy.entries.get(&(index as u64)) {
            policy.entries.insert(u64::from(count), *choice);
        }
        count += 1; // Bounded by the classic ZIP count validated on open.
    }
    write_footer(
        &mut writer,
        directory_offset,
        directory_size,
        count,
        &source.footer.comment,
    )?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    let bytes_written = writer.get_ref().metadata()?.len();
    drop(writer);

    staging.verify()?;
    let metadata_limits = ArchiveLimits {
        operation_timeout: reader.remaining_time(),
        ..ArchiveLimits::default()
    };
    let mut saved = open_archive(&staging.path(), &policy, &metadata_limits, cancel)?;
    if saved.source_state.identity != identity || saved.entries.len() != usize::from(count) {
        return Err(ArchiveError::DestinationChanged);
    }
    let verify_limits = StreamLimits {
        operation_timeout: reader.remaining_time(),
        ..limits.clone()
    };
    let mut stream = begin_stream(&saved, &verify_limits, cancel)?;
    for entry in &saved.entries {
        stream.copy_entry(entry.id, &mut io::sink(), &mut |_| {})?;
    }
    stream.finish()?;

    #[cfg(test)]
    before_publish();
    verify_source(&mut reader, source)?;
    // Verify the name still references the validated inode. All writes and
    // publication are relative to the initially opened destination handles.
    staging.verify()?;
    let mut read = OpenOptions::new();
    read.read(true).follow(FollowSymlinks::No);
    let staged = staging.directory.open_with(STAGED_FILE, &read)?.into_std();
    let stamp = SourceStamp::from_metadata(staged.metadata()?)?;
    if Handle::from_file(staged)? != identity || stamp != saved.source_state.stamp {
        return Err(ArchiveError::DestinationChanged);
    }
    let state = Arc::get_mut(&mut saved.source_state)
        .ok_or_else(|| ArchiveError::Io("saved ZIP still has an active reader".into()))?;
    state.path.clone_from(&target.path);
    saved.source = target.path.to_string_lossy().into_owned();
    reader.check()?;
    staging.publish()?;
    // The file is published. A late cancel cannot undo the result.
    // Where possible, keep the verified identity through the published name.
    // A Windows handle opened through the staging link can delay its removal.
    if let Ok(file) = target.parent.open_with(&target.leaf, &read)
        && let Ok(published) = Handle::from_file(file.into_std())
        && published == identity
    {
        state.identity = published;
    }
    Ok(SavedZip {
        report: ZipEditReport {
            destination: saved.source.clone(),
            entries_removed: removed.len(),
            entries_retained: usize::from(count),
            bytes_written,
            retained_crc32_verified: true,
        },
        snapshot: saved,
    })
}

fn check_extra_fields(mut fields: &[u8]) -> Result<(), ArchiveError> {
    // Framing was checked while reading the local/central header.
    while !fields.is_empty() {
        let id = u16_at(fields, 0);
        if !matches!(
            id,
            0x000a | 0x5455 | 0x5855 | 0x6375 | 0x7075 | 0x7855 | 0x7875
        ) {
            return Err(ArchiveError::Unsupported(format!(
                "retained entry has unverified ZIP extra field 0x{id:04x}"
            )));
        }
        fields = &fields[4 + usize::from(u16_at(fields, 2))..];
    }
    Ok(())
}

fn write_footer(
    writer: &mut impl Write,
    offset: u64,
    size: u64,
    count: u16,
    comment: &[u8],
) -> io::Result<()> {
    let mut footer = [0; 22];
    footer[..4].copy_from_slice(b"PK\x05\x06");
    footer[8..10].copy_from_slice(&count.to_le_bytes());
    footer[10..12].copy_from_slice(&count.to_le_bytes());
    footer[12..16].copy_from_slice(&(size as u32).to_le_bytes());
    footer[16..20].copy_from_slice(&(offset as u32).to_le_bytes());
    footer[20..22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    writer.write_all(&footer)?;
    writer.write_all(comment)
}

struct StagingZip<'a> {
    directory: Dir,
    identity: Handle,
    // Fields drop in declaration order. Windows denies directory removal
    // until both capability handles to this directory have closed.
    cleanup: StagingLocation<'a>,
}

struct StagingLocation<'a> {
    target: &'a DestinationParent,
    leaf: PathBuf,
    remove_on_drop: bool,
}

impl<'a> StagingZip<'a> {
    fn new(target: &'a DestinationParent) -> Result<Self, ArchiveError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| ArchiveError::Io(format!("temporary ZIP name: {error}")))?;
        let leaf = PathBuf::from(format!(".arcforge-save-{}", hex(&random)));
        target.verify_parent()?;
        target
            .parent
            .create_dir_with(&leaf, &private_directory_builder())?;
        let directory = target.parent.open_dir_nofollow(&leaf)?;
        let identity = directory_identity(&directory)?;
        Ok(Self {
            directory,
            identity,
            cleanup: StagingLocation {
                target,
                leaf,
                remove_on_drop: false,
            },
        })
    }

    fn verify(&self) -> Result<(), ArchiveError> {
        self.cleanup
            .target
            .verify_directory(&self.cleanup.leaf, &self.identity)
    }

    fn path(&self) -> PathBuf {
        self.cleanup
            .target
            .parent_path
            .join(&self.cleanup.leaf)
            .join(STAGED_FILE)
    }

    fn publish(&self) -> Result<(), ArchiveError> {
        let target = self.cleanup.target;
        self.directory
            .hard_link(STAGED_FILE, &target.parent, &target.leaf)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    target.exists_error()
                } else {
                    ArchiveError::Io(format!(
                        "could not publish the new ZIP using an exclusive hard link: {error}"
                    ))
                }
            })
    }
}

impl Drop for StagingZip<'_> {
    fn drop(&mut self) {
        let _ = self.directory.remove_file(STAGED_FILE);
        self.cleanup.remove_on_drop = self.verify().is_ok();
    }
}

impl Drop for StagingLocation<'_> {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = self.target.parent.remove_dir(&self.leaf);
        }
    }
}
