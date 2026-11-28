use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use zip::ZipReadOptions;

use super::super::{
    ArchiveError, ArchiveLimits, ArchiveSnapshot, CancellationToken, EntryId, EntryKind,
    StreamLimits, StreamProgress, StreamTotals, StreamedEntry,
};
use super::io::BudgetReader;
use super::{
    ProviderRecord, check_source_identity, encrypted, entry_kind, open_source_reader,
    pin_data_start, validate_descriptor, validate_local, validate_policy, verify_source,
};

const OUTPUT_CHUNK: usize = 16 * 1024;

/// One extraction operation's source cursor, cumulative budgets and validation.
///
/// Sinks may contain partial data after an error. The caller owns file creation,
/// placement, flushing and partial-output reporting. A successful `copy_entry`
/// validates that entry; `finish` is required to validate the complete source
/// metadata again before the caller reports operation success.
#[derive(Debug)]
pub struct ArchiveStream<'a> {
    snapshot: &'a ArchiveSnapshot,
    reader: BudgetReader,
    limits: StreamLimits,
    cancel: CancellationToken,
    started: Instant,
    entries: u64,
    bytes_written: u64,
    failure: Option<ArchiveError>,
}

pub fn begin_stream<'a>(
    snapshot: &'a ArchiveSnapshot,
    limits: &StreamLimits,
    cancel: &CancellationToken,
) -> Result<ArchiveStream<'a>, ArchiveError> {
    let started = Instant::now();
    check_control(cancel, started, limits.operation_timeout)?;
    validate_policy(
        &snapshot.policy,
        Some(snapshot.source_state.records.len() as u64),
    )?;
    let read_limits = ArchiveLimits {
        max_read_bytes: limits.max_read_bytes,
        operation_timeout: limits.operation_timeout,
        ..ArchiveLimits::default()
    };
    let mut reader = open_source_reader(&snapshot.source_state, &read_limits, cancel)?;
    verify_source(&mut reader, &snapshot.source_state)?;
    check_control(cancel, started, limits.operation_timeout)?;
    Ok(ArchiveStream {
        snapshot,
        reader,
        limits: limits.clone(),
        cancel: cancel.clone(),
        started,
        entries: 0,
        bytes_written: 0,
        failure: None,
    })
}

impl ArchiveStream<'_> {
    /// Copy a complete regular file, or verify an empty directory with a sink.
    /// Every error poisons the session. No later call can report success after a
    /// failed CRC, cancellation, source conflict or sink write.
    pub fn copy_entry(
        &mut self,
        entry_id: EntryId,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(StreamProgress),
    ) -> Result<StreamedEntry, ArchiveError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.copy_entry_inner(entry_id, sink, progress);
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    /// Observed work, including bytes accepted by a sink before an error.
    pub fn totals(&self) -> StreamTotals {
        StreamTotals {
            entries: self.entries,
            bytes_written: self.bytes_written,
            bytes_read: self.reader.bytes_read(),
        }
    }

    /// Recheck the actual source's metadata fingerprint before reporting success.
    /// This does not flush sinks or publish destination paths.
    pub fn finish(mut self) -> Result<StreamTotals, ArchiveError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        self.check()?;
        verify_source(&mut self.reader, &self.snapshot.source_state)?;
        self.check()?;
        Ok(self.totals())
    }

    fn check(&self) -> Result<(), ArchiveError> {
        check_control(&self.cancel, self.started, self.limits.operation_timeout)?;
        self.reader.check()
    }

    fn copy_entry_inner(
        &mut self,
        entry_id: EntryId,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(StreamProgress),
    ) -> Result<StreamedEntry, ArchiveError> {
        self.check()?;
        let source = &self.snapshot.source_state;
        let record = usize::try_from(entry_id)
            .ok()
            .and_then(|index| source.records.get(index))
            .ok_or(ArchiveError::EntryNotFound(entry_id))?;
        let raw = &record.raw;
        check_streamable(record)?;
        if self.entries >= self.limits.max_entries {
            return Err(ArchiveError::LimitExceeded("streamed entry count".into()));
        }
        if raw.size > self.limits.max_entry_bytes {
            return Err(ArchiveError::LimitExceeded(
                "per-entry streamed output bytes".into(),
            ));
        }
        let expected_total = self
            .bytes_written
            .checked_add(raw.size)
            .ok_or_else(|| ArchiveError::LimitExceeded("streamed output total overflow".into()))?;
        if expected_total > self.limits.max_total_bytes {
            return Err(ArchiveError::LimitExceeded(
                "cumulative streamed output bytes".into(),
            ));
        }
        check_source_identity(&self.reader, &source.path, &source.identity, &source.stamp)?;
        let choice = self.snapshot.policy.choice_for(entry_id);
        let local = validate_local(
            &mut self.reader,
            record,
            source.footer.directory_offset,
            Some(choice),
        )?;
        pin_data_start(record, local.data_start)?;
        let mut entry_bytes_written = 0_u64;
        progress(StreamProgress {
            entry_id,
            entry_bytes_written,
            total_bytes_written: self.bytes_written,
            entry_size: raw.size,
        });
        {
            // Borrowed metadata prevents ZipFile::Drop from draining a failed or
            // cancelled entry's remaining input.
            let mut file = record
                .metadata
                .as_ref()
                .with_reader(&mut self.reader, ZipReadOptions::new())?;
            let mut chunk = [0; OUTPUT_CHUNK];
            loop {
                check_control(&self.cancel, self.started, self.limits.operation_timeout)?;
                let remaining = raw
                    .size
                    .saturating_sub(entry_bytes_written)
                    .min(
                        self.limits
                            .max_entry_bytes
                            .saturating_sub(entry_bytes_written),
                    )
                    .min(
                        self.limits
                            .max_total_bytes
                            .saturating_sub(self.bytes_written),
                    );
                // One byte of lookahead distinguishes EOF from excess output;
                // that byte is never written if any declared or actual limit fails.
                let requested = remaining.saturating_add(1).min(OUTPUT_CHUNK as u64) as usize;
                let count = file.read(&mut chunk[..requested])?;
                if count == 0 {
                    break; // Complete read through the dependency's CRC check.
                }
                let entry_end = entry_bytes_written
                    .checked_add(count as u64)
                    .ok_or_else(|| ArchiveError::LimitExceeded("entry output overflow".into()))?;
                let total_end = self
                    .bytes_written
                    .checked_add(count as u64)
                    .ok_or_else(|| {
                        ArchiveError::LimitExceeded("streamed output total overflow".into())
                    })?;
                if entry_end > raw.size {
                    return Err(ArchiveError::InvalidArchive(
                        "streamed output exceeds the declared entry size".into(),
                    ));
                }
                if entry_end > self.limits.max_entry_bytes
                    || total_end > self.limits.max_total_bytes
                {
                    return Err(ArchiveError::LimitExceeded(
                        "actual streamed output bytes".into(),
                    ));
                }
                let mut pending = &chunk[..count];
                while !pending.is_empty() {
                    check_control(&self.cancel, self.started, self.limits.operation_timeout)?;
                    let written = match sink.write(pending) {
                        Ok(0) => {
                            return Err(ArchiveError::Io(
                                "destination write returned zero before the entry completed".into(),
                            ));
                        }
                        Ok(written) if written <= pending.len() => written,
                        Ok(_) => {
                            return Err(ArchiveError::Io(
                                "destination writer reported more bytes than supplied".into(),
                            ));
                        }
                        Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                        Err(error) => {
                            return Err(ArchiveError::Io(format!(
                                "destination write failed: {error}"
                            )));
                        }
                    };
                    entry_bytes_written += written as u64;
                    self.bytes_written += written as u64;
                    pending = &pending[written..];
                    progress(StreamProgress {
                        entry_id,
                        entry_bytes_written,
                        total_bytes_written: self.bytes_written,
                        entry_size: raw.size,
                    });
                }
            }
        }
        if entry_bytes_written != raw.size {
            return Err(ArchiveError::InvalidArchive(
                "streamed output is shorter than the declared entry size".into(),
            ));
        }
        if raw.flags() & 8 != 0 {
            validate_descriptor(
                &mut self.reader,
                record,
                local.data_end,
                source.footer.directory_offset,
            )?;
        }
        let after = validate_local(
            &mut self.reader,
            record,
            source.footer.directory_offset,
            Some(choice),
        )?;
        if after != local {
            return Err(ArchiveError::SourceChanged);
        }
        check_source_identity(&self.reader, &source.path, &source.identity, &source.stamp)?;
        self.check()?;
        self.entries += 1; // max_entries was checked before work; this cannot overflow.
        Ok(StreamedEntry {
            entry_id,
            bytes_written: entry_bytes_written,
            crc32_verified: true,
        })
    }
}

pub(super) fn check_streamable(record: &ProviderRecord) -> Result<(), ArchiveError> {
    let raw = &record.raw;
    match entry_kind(record) {
        EntryKind::File => {}
        EntryKind::Directory if raw.size == 0 => {}
        EntryKind::Directory => {
            return Err(ArchiveError::Unsupported(
                "directory entries containing file data cannot be streamed".into(),
            ));
        }
        EntryKind::Symlink | EntryKind::Special => {
            return Err(ArchiveError::Unsupported(
                "symbolic links, special files and conflicting entry types cannot be streamed"
                    .into(),
            ));
        }
    }
    if encrypted(record) {
        return Err(ArchiveError::Unsupported(
            "encrypted entries cannot be streamed".into(),
        ));
    }
    if !matches!(raw.method(), 0 | 8) {
        return Err(ArchiveError::Unsupported(format!(
            "streaming supports Stored (0) and Deflate (8), entry uses method {}",
            raw.method()
        )));
    }
    Ok(())
}

fn check_control(
    cancel: &CancellationToken,
    started: Instant,
    timeout: Duration,
) -> Result<(), ArchiveError> {
    cancel.check()?;
    if started.elapsed() >= timeout {
        return Err(ArchiveError::LimitExceeded(
            "streaming session time budget".into(),
        ));
    }
    Ok(())
}
