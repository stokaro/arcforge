use super::super::{ArchiveError, ArchiveLimits};
use super::io::BudgetReader;

const MAX_ZIP64_EXTENSION: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Footer {
    pub file_len: u64,
    pub directory_offset: u64,
    pub directory_size: u64,
    pub count: u64,
    pub eocd_offset: u64,
    pub comment: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct CentralRecord {
    pub header: [u8; 46],
    pub name: Vec<u8>,
    pub extra: Vec<u8>,
    pub comment: Vec<u8>,
    pub next_offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub local_offset: u64,
}

impl CentralRecord {
    pub fn flags(&self) -> u16 {
        u16_at(&self.header, 8)
    }

    pub fn method(&self) -> u16 {
        u16_at(&self.header, 10)
    }

    pub fn crc32(&self) -> u32 {
        u32_at(&self.header, 16)
    }
}

/// Validate the bounded allocation boundary before the dependency sees ZIP64.
/// This first slice accepts a single volume with absolute offsets, contiguous
/// central directory/footer, and an EOCD whose comment ends at physical EOF.
pub(super) fn preflight(
    reader: &mut BudgetReader,
    file_len: u64,
    limits: &ArchiveLimits,
) -> Result<Footer, ArchiveError> {
    reader.check()?;
    if file_len < 22 {
        return invalid("missing end-of-central-directory record");
    }
    let tail_len = usize::try_from(file_len.min(22 + u64::from(u16::MAX)))
        .map_err(|_| ArchiveError::LimitExceeded("EOCD tail length".into()))?;
    let tail_start = file_len - tail_len as u64;
    let mut tail = vec![0; tail_len];
    reader.read_at(tail_start, &mut tail)?;
    let end = (0..=tail_len - 22)
        .rev()
        .find(|&position| {
            tail[position..position + 4] == *b"PK\x05\x06"
                && position + 22 + usize::from(u16_at(&tail, position + 20)) == tail_len
        })
        .ok_or_else(|| {
            ArchiveError::Unsupported(
                "requires a ZIP EOCD ending at EOF; trailing data is not supported".into(),
            )
        })?;
    let eocd = &tail[end..end + 22];
    let eocd_offset = tail_start + end as u64;
    let count32 = u16_at(eocd, 10);
    let directory_size32 = u32_at(eocd, 12);
    let directory_offset32 = u32_at(eocd, 16);
    let needs_zip64 = count32 == u16::MAX
        || u16_at(eocd, 8) == u16::MAX
        || directory_size32 == u32::MAX
        || directory_offset32 == u32::MAX;
    let mut locator = [0; 20];
    let has_zip64 = if eocd_offset >= 20 {
        reader.read_at(eocd_offset - 20, &mut locator)?;
        locator[..4] == *b"PK\x06\x07"
    } else {
        false
    };
    let (directory_offset, directory_size, count, footer_start) = if has_zip64 {
        if u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
            return unsupported("multi-volume ZIP64 archives");
        }
        let zip64_offset = u64_at(&locator, 8);
        checked_end(zip64_offset, 56, eocd_offset - 20, "ZIP64 end record")?;
        let mut fixed = [0; 56];
        reader.read_at(zip64_offset, &mut fixed)?;
        if fixed[..4] != *b"PK\x06\x06" {
            return invalid("ZIP64 locator does not identify a ZIP64 end record");
        }
        let record_size = u64_at(&fixed, 4);
        if record_size < 44 {
            return invalid("ZIP64 end record is shorter than its fixed fields");
        }
        if record_size - 44 > MAX_ZIP64_EXTENSION {
            return Err(ArchiveError::LimitExceeded(
                "ZIP64 extensible data sector exceeds 64 KiB".into(),
            ));
        }
        let record_end = zip64_offset
            .checked_add(12)
            .and_then(|offset| offset.checked_add(record_size))
            .ok_or_else(|| ArchiveError::InvalidArchive("ZIP64 size overflow".into()))?;
        if record_end != eocd_offset - 20 {
            return unsupported("noncontiguous ZIP64 end record and locator");
        }
        if u32_at(&fixed, 16) != 0
            || u32_at(&fixed, 20) != 0
            || u64_at(&fixed, 24) != u64_at(&fixed, 32)
            || !matches!(u16_at(eocd, 4), 0 | u16::MAX)
            || !matches!(u16_at(eocd, 6), 0 | u16::MAX)
        {
            return unsupported("multi-volume ZIP64 archives");
        }
        let count = u64_at(&fixed, 32);
        let size = u64_at(&fixed, 40);
        let offset = u64_at(&fixed, 48);
        if (count32 != u16::MAX && u64::from(count32) != count)
            || (u16_at(eocd, 8) != u16::MAX && u64::from(u16_at(eocd, 8)) != count)
            || (directory_size32 != u32::MAX && u64::from(directory_size32) != size)
            || (directory_offset32 != u32::MAX && u64::from(directory_offset32) != offset)
        {
            return invalid("ZIP32 and ZIP64 footer declarations disagree");
        }
        (offset, size, count, zip64_offset)
    } else {
        if needs_zip64 {
            return invalid("ZIP64 placeholders require a ZIP64 end record and locator");
        }
        if u16_at(eocd, 4) != 0 || u16_at(eocd, 6) != 0 || u16_at(eocd, 8) != count32 {
            return unsupported("multi-volume ZIP archives");
        }
        (
            u64::from(directory_offset32),
            u64::from(directory_size32),
            u64::from(count32),
            eocd_offset,
        )
    };
    if count > limits.max_entries {
        return Err(ArchiveError::LimitExceeded(
            "central-directory entry count".into(),
        ));
    }
    if directory_size > limits.max_central_directory_bytes {
        return Err(ArchiveError::LimitExceeded(
            "central-directory bytes".into(),
        ));
    }
    if count > directory_size / 46 || (count == 0 && directory_size != 0) {
        return invalid("central-directory count cannot fit its declared size");
    }
    if checked_end(
        directory_offset,
        directory_size,
        file_len,
        "central directory",
    )? != footer_start
    {
        return unsupported("central directory must end immediately at the ZIP footer");
    }
    if count == 0 {
        if directory_offset != 0 {
            return unsupported("prefixed empty ZIP archives");
        }
    } else {
        let mut signature = [0; 4];
        reader.read_at(0, &mut signature)?;
        if signature != *b"PK\x03\x04" {
            return unsupported("self-extracting or prefixed ZIP archives");
        }
    }
    let comment = tail[end + 22..].to_vec();
    if directory_size
        .checked_add(comment.len() as u64)
        .is_none_or(|total| total > limits.max_metadata_bytes)
    {
        return Err(ArchiveError::LimitExceeded("raw metadata bytes".into()));
    }
    Ok(Footer {
        file_len,
        directory_offset,
        directory_size,
        count,
        eocd_offset,
        comment,
    })
}

pub(super) fn read_central(
    reader: &mut BudgetReader,
    offset: u64,
    directory_end: u64,
) -> Result<CentralRecord, ArchiveError> {
    checked_end(offset, 46, directory_end, "central header")?;
    let mut header = [0; 46];
    reader.read_at(offset, &mut header)?;
    if header[..4] != *b"PK\x01\x02" {
        return invalid("central directory contains a non-entry record");
    }
    let name_len = usize::from(u16_at(&header, 28));
    let extra_len = usize::from(u16_at(&header, 30));
    let comment_len = usize::from(u16_at(&header, 32));
    let total = 46 + name_len + extra_len + comment_len;
    let next_offset = checked_end(offset, total as u64, directory_end, "central metadata")?;
    let mut name = vec![0; name_len];
    let mut extra = vec![0; extra_len];
    let mut comment = vec![0; comment_len];
    reader.read_at(offset + 46, &mut name)?;
    reader.read_at(offset + 46 + name_len as u64, &mut extra)?;
    reader.read_at(
        offset + 46 + name_len as u64 + extra_len as u64,
        &mut comment,
    )?;
    let zip64 = zip64_extra(&extra)?;
    let mut position = 0;
    let size = zip64_value(u32_at(&header, 24), zip64, &mut position)?;
    let compressed_size = zip64_value(u32_at(&header, 20), zip64, &mut position)?;
    let local_offset = zip64_value(u32_at(&header, 42), zip64, &mut position)?;
    let disk = u16_at(&header, 34);
    if disk == u16::MAX {
        let body = zip64.ok_or_else(|| {
            ArchiveError::InvalidArchive("missing ZIP64 disk-number field".into())
        })?;
        if body.len().saturating_sub(position) < 4 {
            return invalid("truncated ZIP64 disk-number field");
        }
        if u32_at(body, position) != 0 {
            return unsupported("entries spanning multiple volumes");
        }
    } else if disk != 0 {
        return unsupported("entries spanning multiple volumes");
    }
    Ok(CentralRecord {
        header,
        name,
        extra,
        comment,
        next_offset,
        compressed_size,
        size,
        local_offset,
    })
}

/// Validate TLV framing and return the unique ZIP64 extra field, if present.
pub(super) fn zip64_extra(extra: &[u8]) -> Result<Option<&[u8]>, ArchiveError> {
    let mut position = 0;
    let mut zip64 = None;
    while position < extra.len() {
        if extra.len() - position < 4 {
            return invalid("truncated extra-field header");
        }
        let tag = u16_at(extra, position);
        let length = usize::from(u16_at(extra, position + 2));
        position += 4;
        if length > extra.len() - position {
            return invalid("extra-field length exceeds available metadata");
        }
        if tag == 1 {
            if zip64.is_some() {
                return invalid("duplicate ZIP64 extra fields are ambiguous");
            }
            zip64 = Some(&extra[position..position + length]);
        }
        position += length;
    }
    Ok(zip64)
}

pub(super) fn zip64_value(
    value: u32,
    extra: Option<&[u8]>,
    position: &mut usize,
) -> Result<u64, ArchiveError> {
    if value != u32::MAX {
        return Ok(u64::from(value));
    }
    let extra = extra.ok_or_else(|| {
        ArchiveError::InvalidArchive("ZIP64 placeholder lacks a ZIP64 extra field".into())
    })?;
    if extra.len().saturating_sub(*position) < 8 {
        return invalid("truncated ZIP64 extra-field value");
    }
    let value = u64_at(extra, *position);
    *position += 8;
    Ok(value)
}

pub(super) fn checked_end(
    offset: u64,
    size: u64,
    bound: u64,
    description: &str,
) -> Result<u64, ArchiveError> {
    offset
        .checked_add(size)
        .filter(|end| *end <= bound)
        .ok_or_else(|| {
            ArchiveError::InvalidArchive(format!("{description} exceeds its byte range"))
        })
}

pub(super) fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

pub(super) fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

pub(super) fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

pub(super) fn invalid<T>(message: &str) -> Result<T, ArchiveError> {
    Err(ArchiveError::InvalidArchive(message.into()))
}

fn unsupported<T>(message: &str) -> Result<T, ArchiveError> {
    Err(ArchiveError::Unsupported(message.into()))
}
