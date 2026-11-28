use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::time::Instant;

use super::super::{ArchiveError, ArchiveLimits, CancellationToken};

const READ_CHUNK: usize = 64 * 1024;
// zip 9.8.2 searches this much before the logical EOF while finding its EOCD.
const DISCOVERY_TAIL: u64 = 22 + (1 << 16);

#[derive(Debug)]
struct FrozenMetadata {
    start: u64,
    bytes: Vec<u8>,
    prefix: [u8; 4],
}

/// One operation owns one fresh file cursor. Cloned identity handles never read it.
#[derive(Debug)]
pub(super) struct BudgetReader {
    pub file: File,
    len: u64,
    position: u64,
    logical_len: u64,
    comment_length_offset: Option<u64>,
    frozen: Option<FrozenMetadata>,
    bytes_read: u64,
    max_read: u64,
    started: Instant,
    timeout: std::time::Duration,
    cancel: CancellationToken,
}

impl BudgetReader {
    pub fn new(file: File, len: u64, limits: &ArchiveLimits, cancel: &CancellationToken) -> Self {
        Self {
            file,
            len,
            position: 0,
            logical_len: len,
            comment_length_offset: None,
            frozen: None,
            bytes_read: 0,
            max_read: limits.max_read_bytes,
            started: Instant::now(),
            timeout: limits.operation_timeout,
            cancel: cancel.clone(),
        }
    }

    pub fn check(&self) -> Result<(), ArchiveError> {
        self.cancel.check()?;
        if self.started.elapsed() >= self.timeout {
            return Err(ArchiveError::LimitExceeded("operation time budget".into()));
        }
        Ok(())
    }

    pub fn remaining_time(&self) -> std::time::Duration {
        self.timeout.saturating_sub(self.started.elapsed())
    }

    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// Freeze only the bounded directory/footer and the dependency's small
    /// discovery window. A second preflight must validate this exact copy before
    /// exposing it to the dependency: the source can change during this copy.
    pub fn freeze_metadata(
        &mut self,
        directory_offset: u64,
        eocd_offset: u64,
    ) -> Result<(), ArchiveError> {
        self.check()?;
        let discovery_start = (eocd_offset + 22).saturating_sub(DISCOVERY_TAIL);
        let start = directory_offset.min(discovery_start);
        let size = usize::try_from(self.len - start)
            .map_err(|_| ArchiveError::LimitExceeded("frozen metadata window".into()))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| {
            ArchiveError::LimitExceeded("could not reserve bounded metadata window".into())
        })?;
        bytes.resize(size, 0);
        self.read_at(start, &mut bytes)?;
        let mut prefix = [0; 4];
        self.read_at(0, &mut prefix)?;
        self.frozen = Some(FrozenMetadata {
            start,
            bytes,
            prefix,
        });
        Ok(())
    }

    /// Subsequent checks must observe the actual file, never the frozen copy.
    pub fn resume_file_reads(&mut self) -> Result<(), ArchiveError> {
        self.frozen = None;
        self.full_view();
        self.position = self.file.seek(SeekFrom::Start(self.position))?;
        self.check()
    }

    /// Discovery receives the already validated EOCD with its comment omitted.
    /// Otherwise an EOCD-looking sequence inside the opaque archive comment can
    /// make the dependency discover an un-preflighted ZIP64 allocation candidate.
    /// Original comment bytes remain in our evidence and metadata fingerprint.
    pub fn directory_view(&mut self, eocd_offset: u64) {
        self.logical_len = eocd_offset + 22;
        self.comment_length_offset = Some(eocd_offset + 20);
    }

    fn full_view(&mut self) {
        self.logical_len = self.len;
        self.comment_length_offset = None;
    }

    pub fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), ArchiveError> {
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buffer)?;
        Ok(())
    }
}

impl Read for BudgetReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.check().map_err(io::Error::other)?;
        if buffer.is_empty() || self.position >= self.logical_len {
            return Ok(0);
        }
        let count = buffer
            .len()
            .min(READ_CHUNK)
            .min(usize::try_from(self.logical_len - self.position).unwrap_or(usize::MAX));
        let count = if let Some(frozen) = &self.frozen {
            let source = if self.position >= frozen.start {
                let offset = usize::try_from(self.position - frozen.start).map_err(|_| {
                    io::Error::other(ArchiveError::InvalidArchive(
                        "frozen metadata offset overflow".into(),
                    ))
                })?;
                frozen.bytes.get(offset..).ok_or_else(|| {
                    io::Error::other(ArchiveError::InvalidArchive(
                        "read outside frozen metadata".into(),
                    ))
                })?
            } else if self.position < 4 {
                &frozen.prefix[self.position as usize..]
            } else {
                return Err(io::Error::other(ArchiveError::InvalidArchive(
                    "dependency read outside the bounded frozen metadata window".into(),
                )));
            };
            let count = count.min(source.len());
            buffer[..count].copy_from_slice(&source[..count]);
            count
        } else {
            let remaining = self.max_read.saturating_sub(self.bytes_read);
            if remaining == 0 {
                return Err(io::Error::other(ArchiveError::LimitExceeded(
                    "actual input bytes read".into(),
                )));
            }
            let count = count.min(usize::try_from(remaining).unwrap_or(usize::MAX));
            let count = self.file.read(&mut buffer[..count])?;
            self.bytes_read += count as u64;
            count
        };
        if let Some(comment) = self.comment_length_offset {
            for offset in [comment, comment + 1] {
                if offset >= self.position && offset - self.position < count as u64 {
                    buffer[(offset - self.position) as usize] = 0;
                }
            }
        }
        self.position += count as u64;
        self.check().map_err(io::Error::other)?;
        Ok(count)
    }
}

impl Seek for BudgetReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.check().map_err(io::Error::other)?;
        let offset = match target {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.logical_len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        }
        .filter(|offset| *offset <= self.logical_len)
        .ok_or_else(|| {
            io::Error::other(ArchiveError::InvalidArchive(
                "seek is outside the source file".into(),
            ))
        })?;
        self.position = if self.frozen.is_some() {
            offset
        } else {
            self.file.seek(SeekFrom::Start(offset))?
        };
        Ok(self.position)
    }
}
