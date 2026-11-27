//! Narrow reader for our generated bundles, not an application archive provider.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Result;

const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Directory(pub PathBuf);

impl Directory {
    pub fn new() -> Result<Self> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arcforge-delivery-{}-{timestamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self(fs::canonicalize(path)?))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Extractor<'a> {
    destination: &'a Path,
    stem: &'a str,
    names: BTreeSet<String>,
    total: u64,
}

impl Extractor<'_> {
    fn file(&mut self, name: &str, size: u64, mode: u32, reader: &mut impl Read) -> Result<()> {
        let relative = relative_path(name, self.stem)?;
        if !self.names.insert(relative.to_lowercase()) || self.names.len() > MAX_ENTRIES {
            return Err("duplicate/case-colliding name or too many bundle entries".into());
        }
        self.total = self.total.checked_add(size).ok_or("bundle size overflow")?;
        if size > MAX_FILE || self.total > MAX_TOTAL {
            return Err("bundle exceeds extraction size limit".into());
        }
        if mode & !0o100777 != 0 || !matches!(mode & 0o777, 0o644 | 0o755) {
            return Err(format!("unsupported file permissions in bundle: {name}").into());
        }
        let path = self.destination.join(relative);
        fs::create_dir_all(path.parent().ok_or("bundle entry has no parent")?)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let copied = std::io::copy(&mut reader.take(size + 1), &mut output)?;
        if copied != size {
            return Err(format!("bundle entry size mismatch: {name}").into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777))?;
        }
        Ok(())
    }
}

pub(super) fn unpack(archive: &Path, extension: &str, stem: &str) -> Result<Directory> {
    check_archive(archive)?;
    let directory = Directory::new()?;
    let mut extractor = Extractor {
        destination: &directory.0,
        stem,
        names: BTreeSet::new(),
        total: 0,
    };
    if extension == "zip" {
        let mut file = File::open(archive)?;
        check_zip_directory(&mut file)?;
        let mut zip = zip::ZipArchive::new(file)?;
        if zip.len() > MAX_ENTRIES {
            return Err("too many bundle entries".into());
        }
        // The ZIP name index otherwise hides repeated raw central-directory names.
        if zip.has_duplicate_names() {
            return Err("duplicate raw ZIP names in development bundle".into());
        }
        zip.validate_header_consistency()?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let name = entry.name()?.into_owned();
            let mode = entry.unix_mode().ok_or("ZIP entry has no Unix mode")?;
            if entry.is_dir()
                || mode & 0o170000 != 0o100000
                || entry.compression() != zip::CompressionMethod::Stored
            {
                return Err("bundle ZIP must contain only stored regular files".into());
            }
            extractor.file(&name, entry.size(), mode, &mut entry)?;
        }
    } else if extension == "tar.gz" {
        // Bound the decompressed stream, including metadata/extension headers.
        let reader = flate2::read::GzDecoder::new(File::open(archive)?).take(MAX_TOTAL + 1);
        let mut tar = tar::Archive::new(reader);
        let mut long_name = None;
        // Raw iteration avoids tar's unbounded preprocessing of GNU/PAX payloads.
        // Our writer emits GNU long names, but no PAX, links or sparse entries.
        for (index, entry) in tar.entries()?.raw(true).enumerate() {
            if index >= MAX_ENTRIES * 2 {
                return Err("too many bundle TAR headers".into());
            }
            let mut entry = entry?;
            if entry.header().entry_type().is_gnu_longname() {
                if long_name.is_some() || entry.size() > 1200 {
                    return Err("duplicate or oversized GNU long-name metadata".into());
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                let bytes = bytes
                    .strip_suffix(&[0])
                    .ok_or("unterminated GNU long name")?;
                long_name = Some(std::str::from_utf8(bytes)?.to_owned());
                continue;
            }
            if !entry.header().entry_type().is_file() {
                return Err("bundle TAR must contain only regular files".into());
            }
            let name = match long_name.take() {
                Some(name) => name,
                None => std::str::from_utf8(&entry.path_bytes())?.to_owned(),
            };
            extractor.file(&name, entry.size(), entry.header().mode()?, &mut entry)?;
        }
        if long_name.is_some() {
            return Err("GNU long name has no following file".into());
        }
        // Consume trailer to check gzip integrity and enforce the stream bound.
        let mut remaining = tar.into_inner();
        std::io::copy(&mut remaining, &mut std::io::sink())?;
        if remaining.limit() == 0 {
            return Err("bundle exceeds decompressed stream size limit".into());
        }
    } else {
        return Err("unsupported bundle archive type".into());
    }
    if extractor.names.is_empty() {
        return Err("empty development bundle".into());
    }
    Ok(directory)
}

pub(super) fn check_archive(path: &Path) -> Result<()> {
    regular_file(path)?;
    if fs::metadata(path)?.len() > MAX_ARCHIVE {
        return Err("bundle exceeds compressed size limit".into());
    }
    Ok(())
}

// ZIP64 is unnecessary within these development-bundle bounds. Inspect EOCD
// before ZipArchive allocates its central-directory map.
fn check_zip_directory(file: &mut File) -> Result<()> {
    let length = file.metadata()?.len();
    let tail_length = length.min(65_557);
    file.seek(SeekFrom::End(-i64::try_from(tail_length)?))?;
    let mut tail = vec![0; usize::try_from(tail_length)?];
    file.read_exact(&mut tail)?;
    let position = tail
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .ok_or("ZIP has no end record")?;
    let end = &tail[position..];
    if end.len() < 22 {
        return Err("truncated ZIP end record".into());
    }
    let word = |offset| u16::from_le_bytes([end[offset], end[offset + 1]]);
    let directory_size = u32::from_le_bytes(end[12..16].try_into()?);
    let directory_offset = u32::from_le_bytes(end[16..20].try_into()?);
    let has_zip64_locator = position >= 20 && tail[position - 20..position - 16] == *b"PK\x06\x07";
    if word(4) != 0
        || word(6) != 0
        || word(8) != word(10)
        || usize::from(word(10)) > MAX_ENTRIES
        || directory_size > 16 * 1024 * 1024
        || directory_offset == u32::MAX
        || has_zip64_locator
        || u64::from(directory_offset) + u64::from(directory_size)
            != length - tail_length + u64::try_from(position)?
        || usize::from(word(20)) + 22 != end.len()
    {
        return Err("unsupported or oversized ZIP directory".into());
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn relative_path<'a>(name: &'a str, stem: &str) -> Result<&'a str> {
    let relative = name
        .strip_prefix(stem)
        .and_then(|name| name.strip_prefix('/'))
        .ok_or("bundle entry has incorrect root directory")?;
    if relative.len() > 1024 || relative.contains(['\\', ':', '\0']) {
        return Err("nonportable bundle path".into());
    }
    for part in relative.split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && matches!(base.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("unsafe/nonportable bundle path component".into());
        }
    }
    Ok(relative)
}

pub(super) fn regular_file(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(format!("expected a regular file: {}", path.display()).into());
    }
    Ok(())
}

pub(super) fn read_small(path: &Path, max: u64) -> Result<Vec<u8>> {
    regular_file(path)?;
    if fs::metadata(path)?.len() > max {
        return Err(format!("file exceeds metadata limit: {}", path.display()).into());
    }
    Ok(fs::read(path)?)
}
