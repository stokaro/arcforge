//! Byte-level fixtures intentionally do not use the ZIP dependency's writer.

use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::*;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempArchive {
    directory: PathBuf,
    path: PathBuf,
}

impl TempArchive {
    fn new(bytes: &[u8]) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "arcforge-provider-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create isolated fixture directory");
        let path = directory.join("fixture.zip");
        fs::write(&path, bytes).expect("write fixture");
        Self { directory, path }
    }

    fn open(&self) -> ArchiveSnapshot {
        open_archive(
            &self.path,
            &FilenamePolicy::default(),
            &ArchiveLimits::default(),
            &CancellationToken::new(),
        )
        .expect("open independent ZIP fixture")
    }
}

impl Drop for TempArchive {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[derive(Clone)]
struct EntryFixture {
    name: Vec<u8>,
    local_name: Option<Vec<u8>>,
    extra: Vec<u8>,
    local_extra: Option<Vec<u8>>,
    comment: Vec<u8>,
    flags: u16,
    method: u16,
    data: Vec<u8>,
    compressed: Vec<u8>,
    declared_size: Option<u32>,
    descriptor: Option<bool>,
}

impl EntryFixture {
    fn stored(name: &[u8], data: &[u8]) -> Self {
        Self {
            name: name.to_vec(),
            local_name: None,
            extra: Vec::new(),
            local_extra: None,
            comment: Vec::new(),
            flags: 0,
            method: 0,
            data: data.to_vec(),
            compressed: data.to_vec(),
            declared_size: None,
            descriptor: None,
        }
    }

    fn deflated(name: &[u8], data: &[u8]) -> Self {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).expect("encode raw deflate fixture");
        let mut result = Self::stored(name, data);
        result.method = 8;
        result.compressed = encoder.finish().expect("finish raw deflate fixture");
        result
    }
}

struct ZipFixture {
    bytes: Vec<u8>,
    central_offset: usize,
    central_headers: Vec<usize>,
    data_offsets: Vec<usize>,
    descriptor_offsets: Vec<Option<usize>>,
    eocd_offset: usize,
}

fn push16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn u16_len(bytes: &[u8]) -> u16 {
    u16::try_from(bytes.len()).expect("fixture field fits u16")
}

fn u32_len(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len()).expect("fixture length fits u32")
}

fn zip_fixture(entries: &[EntryFixture], comment: &[u8]) -> ZipFixture {
    let mut bytes = Vec::new();
    let mut local_offsets = Vec::new();
    let mut data_offsets = Vec::new();
    let mut descriptor_offsets = Vec::new();
    for entry in entries {
        local_offsets.push(u32_len(&bytes));
        let name = entry.local_name.as_deref().unwrap_or(&entry.name);
        let extra = entry.local_extra.as_deref().unwrap_or(&entry.extra);
        let descriptor = entry.descriptor.is_some();
        let flags = entry.flags | if descriptor { 8 } else { 0 };
        let size = entry.declared_size.unwrap_or_else(|| u32_len(&entry.data));
        push32(&mut bytes, 0x0403_4b50);
        push16(&mut bytes, 20);
        push16(&mut bytes, flags);
        push16(&mut bytes, entry.method);
        push16(&mut bytes, 0);
        push16(&mut bytes, 0x5821); // 2024-01-01, a valid DOS date.
        push32(
            &mut bytes,
            if descriptor {
                0
            } else {
                crc32fast::hash(&entry.data)
            },
        );
        push32(
            &mut bytes,
            if descriptor {
                0
            } else {
                u32_len(&entry.compressed)
            },
        );
        push32(&mut bytes, if descriptor { 0 } else { size });
        push16(&mut bytes, u16_len(name));
        push16(&mut bytes, u16_len(extra));
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(extra);
        data_offsets.push(bytes.len());
        bytes.extend_from_slice(&entry.compressed);
        descriptor_offsets.push(entry.descriptor.map(|signature| {
            let offset = bytes.len();
            if signature {
                push32(&mut bytes, 0x0807_4b50);
            }
            push32(&mut bytes, crc32fast::hash(&entry.data));
            push32(&mut bytes, u32_len(&entry.compressed));
            push32(&mut bytes, size);
            offset
        }));
    }
    let central_offset = bytes.len();
    let mut central_headers = Vec::new();
    for (entry, local_offset) in entries.iter().zip(local_offsets) {
        central_headers.push(bytes.len());
        push32(&mut bytes, 0x0201_4b50);
        push16(&mut bytes, 0x0314);
        push16(&mut bytes, 20);
        push16(
            &mut bytes,
            entry.flags | if entry.descriptor.is_some() { 8 } else { 0 },
        );
        push16(&mut bytes, entry.method);
        push16(&mut bytes, 0);
        push16(&mut bytes, 0x5821);
        push32(&mut bytes, crc32fast::hash(&entry.data));
        push32(&mut bytes, u32_len(&entry.compressed));
        push32(
            &mut bytes,
            entry.declared_size.unwrap_or_else(|| u32_len(&entry.data)),
        );
        push16(&mut bytes, u16_len(&entry.name));
        push16(&mut bytes, u16_len(&entry.extra));
        push16(&mut bytes, u16_len(&entry.comment));
        push16(&mut bytes, 0);
        push16(&mut bytes, 0);
        push32(
            &mut bytes,
            if entry.name.ends_with(b"/") { 0x10 } else { 0 },
        );
        push32(&mut bytes, local_offset);
        bytes.extend_from_slice(&entry.name);
        bytes.extend_from_slice(&entry.extra);
        bytes.extend_from_slice(&entry.comment);
    }
    let eocd_offset = bytes.len();
    let central_size = u32::try_from(eocd_offset - central_offset).unwrap();
    push32(&mut bytes, 0x0605_4b50);
    push16(&mut bytes, 0);
    push16(&mut bytes, 0);
    push16(&mut bytes, u16::try_from(entries.len()).unwrap());
    push16(&mut bytes, u16::try_from(entries.len()).unwrap());
    push32(&mut bytes, central_size);
    push32(&mut bytes, u32::try_from(central_offset).unwrap());
    push16(&mut bytes, u16_len(comment));
    bytes.extend_from_slice(comment);
    ZipFixture {
        bytes,
        central_offset,
        central_headers,
        data_offsets,
        descriptor_offsets,
        eocd_offset,
    }
}

fn unicode_path(raw_name: &[u8], unicode: &str) -> Vec<u8> {
    let mut extra = Vec::new();
    push16(&mut extra, 0x7075);
    push16(&mut extra, u16::try_from(5 + unicode.len()).unwrap());
    extra.push(1);
    push32(&mut extra, crc32fast::hash(raw_name));
    extra.extend_from_slice(unicode.as_bytes());
    extra
}

fn zip64_fixture() -> ZipFixture {
    zip64_fixture_with_payload(b"small ZIP64 payload")
}

fn zip64_fixture_with_payload(payload: &[u8]) -> ZipFixture {
    let mut entry = EntryFixture::stored(b"zip64.txt", payload);
    push16(&mut entry.extra, 1);
    push16(&mut entry.extra, 24);
    push64(&mut entry.extra, entry.data.len() as u64);
    push64(&mut entry.extra, entry.compressed.len() as u64);
    push64(&mut entry.extra, 0);
    entry.local_extra = Some(Vec::new());
    let mut fixture = zip_fixture(&[entry], b"");
    let central = fixture.central_headers[0];
    fixture.bytes[central + 6..central + 8].copy_from_slice(&45_u16.to_le_bytes());
    for offset in [20, 24, 42] {
        fixture.bytes[central + offset..central + offset + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
    }
    let zip64_offset = fixture.eocd_offset;
    fixture.bytes.truncate(zip64_offset);
    push32(&mut fixture.bytes, 0x0606_4b50);
    push64(&mut fixture.bytes, 44);
    push16(&mut fixture.bytes, 45);
    push16(&mut fixture.bytes, 45);
    push32(&mut fixture.bytes, 0);
    push32(&mut fixture.bytes, 0);
    push64(&mut fixture.bytes, 1);
    push64(&mut fixture.bytes, 1);
    push64(
        &mut fixture.bytes,
        (zip64_offset - fixture.central_offset) as u64,
    );
    push64(&mut fixture.bytes, fixture.central_offset as u64);
    push32(&mut fixture.bytes, 0x0706_4b50);
    push32(&mut fixture.bytes, 0);
    push64(&mut fixture.bytes, zip64_offset as u64);
    push32(&mut fixture.bytes, 1);
    fixture.eocd_offset = fixture.bytes.len();
    push32(&mut fixture.bytes, 0x0605_4b50);
    push16(&mut fixture.bytes, 0);
    push16(&mut fixture.bytes, 0);
    push16(&mut fixture.bytes, u16::MAX);
    push16(&mut fixture.bytes, u16::MAX);
    push32(&mut fixture.bytes, u32::MAX);
    push32(&mut fixture.bytes, u32::MAX);
    push16(&mut fixture.bytes, 0);
    fixture
}

fn preview(snapshot: &ArchiveSnapshot, id: EntryId) -> EntryPreview {
    preview_entry(
        snapshot,
        id,
        &ArchiveLimits::default(),
        &CancellationToken::new(),
    )
    .expect("preview complete fixture entry")
}

fn open_with_limits(path: &Path, limits: &ArchiveLimits) -> Result<ArchiveSnapshot, ArchiveError> {
    open_archive(
        path,
        &FilenamePolicy::default(),
        limits,
        &CancellationToken::new(),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn stored_previews_return_complete_text_or_binary_and_verified_crc() {
    let fixture = zip_fixture(
        &[
            EntryFixture::stored(b"hello.txt", b"hello\n"),
            EntryFixture::stored(b"bytes.bin", &[0, 0xff, 0x80, 1]),
        ],
        b"",
    );
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.compressed_size, 10);
    assert_eq!(snapshot.uncompressed_size, 10);
    let text = preview(&snapshot, snapshot.entries[0].id);
    assert_eq!(text.bytes, b"hello\n");
    assert_eq!(text.text.as_deref(), Some("hello\n"));
    assert_eq!(text.data_hex, "68656c6c6f0a");
    assert!(text.crc32_verified);
    let binary = preview(&snapshot, snapshot.entries[1].id);
    assert_eq!(binary.bytes, [0, 0xff, 0x80, 1]);
    assert_eq!(binary.text, None);
    assert!(binary.crc32_verified);
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn deflate_preview_reads_the_complete_small_entry() {
    let payload = b"bounded deflate preview\n".repeat(1024);
    let fixture = zip_fixture(&[EntryFixture::deflated(b"compressed.txt", &payload)], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries[0].compression_method, 8);
    assert!(snapshot.compressed_size < snapshot.uncompressed_size);
    let result = preview(&snapshot, snapshot.entries[0].id);
    assert_eq!(result.bytes, payload);
    assert!(result.crc32_verified);
}

#[test]
fn duplicate_central_names_preserve_both_ordinals_and_payloads() {
    let fixture = zip_fixture(
        &[
            EntryFixture::stored(b"same.txt", b"first"),
            EntryFixture::stored(b"same.txt", b"second"),
        ],
        b"",
    );
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].id, 0);
    assert_eq!(snapshot.entries[1].id, 1);
    assert_eq!(snapshot.entries[0].name.display, "same.txt");
    assert_eq!(snapshot.entries[1].name.display, "same.txt");
    assert_eq!(preview(&snapshot, 0).bytes, b"first");
    assert_eq!(preview(&snapshot, 1).bytes, b"second");
}

#[test]
fn invalid_declared_utf8_remains_inspectable_as_original_bytes() {
    let mut entry = EntryFixture::stored(b"\xff.txt", b"payload");
    entry.flags = 1 << 11;
    let fixture = zip_fixture(&[entry], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].raw_name_hex, "ff2e747874");
    assert_eq!(snapshot.entries[0].flags, 1 << 11);
    assert_eq!(snapshot.entries[0].name.state, NameState::Invalid);
}

#[test]
fn unicode_effective_collisions_preserve_distinct_raw_names_and_entries() {
    let mut first = EntryFixture::stored(b"legacy-one", b"one");
    let mut second = EntryFixture::stored(b"legacy-two", b"two");
    first.extra = unicode_path(&first.name, "same/имя.txt");
    second.extra = unicode_path(&second.name, "same/имя.txt");
    let fixture = zip_fixture(&[first, second], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].name.display, "same/имя.txt");
    assert_eq!(snapshot.entries[1].name.display, "same/имя.txt");
    assert_eq!(
        snapshot.entries[0].name.provenance,
        NameProvenance::UnicodePath
    );
    assert_eq!(snapshot.entries[0].flags, 0);
    assert_eq!(snapshot.entries[1].flags, 0);
    assert_ne!(
        snapshot.entries[0].raw_name_hex,
        snapshot.entries[1].raw_name_hex
    );
    assert_eq!(preview(&snapshot, snapshot.entries[0].id).bytes, b"one");
    assert_eq!(preview(&snapshot, snapshot.entries[1].id).bytes, b"two");
}

#[test]
fn raw_extra_entry_comment_and_archive_comment_are_not_normalized() {
    let mut entry = EntryFixture::stored(b"raw.bin", b"data");
    entry.extra = vec![0xfe, 0xca, 3, 0, 0, 0xff, 0x80];
    entry.comment = vec![0xff, 0, 0x80, b'a'];
    let mut archive_comment = b"raw\xff\0".to_vec();
    // A complete false EOCD inside the comment must not hide the real directory.
    archive_comment.extend_from_slice(&zip_fixture(&[], b"").bytes);
    archive_comment.extend_from_slice(b"tail");
    let fixture = zip_fixture(&[entry.clone()], &archive_comment);
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries[0].raw_extra_hex, hex(&entry.extra));
    assert_eq!(snapshot.entries[0].raw_comment_hex, hex(&entry.comment));
    assert_eq!(snapshot.comment_hex, hex(&archive_comment));
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn empty_zip_is_a_valid_empty_snapshot() {
    let fixture = zip_fixture(&[], b"empty archive");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert!(snapshot.entries.is_empty());
    assert_eq!(snapshot.compressed_size, 0);
    assert_eq!(snapshot.uncompressed_size, 0);
    assert_eq!(snapshot.comment_hex, hex(b"empty archive"));
}

#[test]
fn reinterpretation_preserves_snapshot_and_entry_identity_and_original_bytes() {
    let fixture = zip_fixture(
        &[
            EntryFixture::stored(b"\x80.txt", b"one"),
            EntryFixture::stored(b"\x80.txt", b"two"),
        ],
        b"",
    );
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let mut policy = FilenamePolicy {
        archive: FilenameChoice {
            encoding: Some(FilenameEncoding::Cp866),
            recovery: false,
        },
        ..FilenamePolicy::default()
    };
    policy.entries.insert(
        snapshot.entries[1].id,
        FilenameChoice {
            encoding: Some(FilenameEncoding::Windows1251),
            recovery: false,
        },
    );
    let changed = reinterpret(&snapshot, &policy, &CancellationToken::new()).unwrap();
    assert_eq!(changed.id, snapshot.id);
    assert_eq!(changed.entries[0].id, snapshot.entries[0].id);
    assert_eq!(changed.entries[1].id, snapshot.entries[1].id);
    assert_eq!(changed.entries[0].name.display, "А.txt");
    assert_eq!(changed.entries[1].name.display, "Ђ.txt");
    assert_eq!(
        changed.entries[0].raw_name_hex,
        snapshot.entries[0].raw_name_hex
    );
    assert_eq!(changed.policy, policy);
    assert_eq!(preview(&changed, changed.entries[1].id).bytes, b"two");
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn nonexistent_entry_ids_are_rejected_for_preview_and_filename_overrides() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"one", b"1")], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let result = preview_entry(
        &snapshot,
        99,
        &ArchiveLimits::default(),
        &CancellationToken::new(),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::EntryNotFound(99));
    let mut policy = FilenamePolicy::default();
    policy.entries.insert(99, FilenameChoice::default());
    assert!(reinterpret(&snapshot, &policy, &CancellationToken::new()).is_err());
    assert!(
        open_archive(
            &file.path,
            &policy,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        )
        .is_err()
    );
}

#[test]
fn corrupt_stored_payload_fails_crc_without_modifying_the_source() {
    let mut fixture = zip_fixture(&[EntryFixture::stored(b"bad.txt", b"original")], b"");
    fixture.bytes[fixture.data_offsets[0]] ^= 0x20;
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let result = preview_entry(
        &snapshot,
        0,
        &ArchiveLimits::default(),
        &CancellationToken::new(),
    );
    assert!(matches!(result, Err(ArchiveError::InvalidArchive(_))));
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn truncated_deflate_payload_is_not_reported_as_verified() {
    let mut entry = EntryFixture::deflated(b"truncated.txt", &b"compressed data\n".repeat(1024));
    entry.compressed.truncate(entry.compressed.len() / 2);
    let fixture = zip_fixture(&[entry], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        )
        .is_err()
    );
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn structural_extra_field_truncation_returns_no_partial_snapshot() {
    let first = EntryFixture::stored(b"good", b"one");
    let mut second = EntryFixture::stored(b"bad", b"two");
    second.extra = vec![0x01, 0x00, 0x10, 0x00, 1];
    let fixture = zip_fixture(&[first, second], b"");
    let file = TempArchive::new(&fixture.bytes);
    assert!(open_with_limits(&file.path, &ArchiveLimits::default()).is_err());
}

#[test]
fn truncated_end_record_is_rejected_without_fallback_to_partial_data() {
    let mut fixture = zip_fixture(&[EntryFixture::stored(b"a", b"b")], b"comment");
    fixture.bytes.pop();
    let file = TempArchive::new(&fixture.bytes);
    assert!(open_with_limits(&file.path, &ArchiveLimits::default()).is_err());
}

#[test]
fn zip64_metadata_and_small_payloads_are_supported() {
    let fixture = zip64_fixture();
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].size, 19);
    assert_eq!(snapshot.entries[0].local_header_offset, 0);
    assert_eq!(preview(&snapshot, 0).bytes, b"small ZIP64 payload");
}

#[cfg(unix)]
#[test]
fn a_sparse_zip64_larger_than_four_gib_is_browsed_with_bounded_metadata_reads() {
    let fixture = zip64_fixture();
    let central_offset = (1_u64 << 32) + 128;
    let file = TempArchive::new(&fixture.bytes[..fixture.central_offset]);
    let mut tail = fixture.bytes[fixture.central_offset..].to_vec();
    let zip64_offset = fixture.eocd_offset - 20 - 56 - fixture.central_offset;
    tail[zip64_offset + 48..zip64_offset + 56].copy_from_slice(&central_offset.to_le_bytes());
    let locator_offset = zip64_offset + 56;
    tail[locator_offset + 8..locator_offset + 16]
        .copy_from_slice(&(central_offset + zip64_offset as u64).to_le_bytes());
    let length = central_offset + tail.len() as u64;
    let mut writer = OpenOptions::new().write(true).open(&file.path).unwrap();
    writer.set_len(length).unwrap();
    writer.seek(SeekFrom::Start(central_offset)).unwrap();
    writer.write_all(&tail).unwrap();
    drop(writer);
    let limits = ArchiveLimits {
        max_read_bytes: 1024 * 1024,
        ..ArchiveLimits::default()
    };
    let snapshot = open_with_limits(&file.path, &limits).unwrap();
    assert_eq!(snapshot.entries.len(), 1);
    let result = preview_entry(&snapshot, 0, &limits, &CancellationToken::new()).unwrap();
    assert_eq!(result.bytes, b"small ZIP64 payload");
    assert_eq!(fs::metadata(&file.path).unwrap().len(), length);
}

#[test]
fn zip64_extensible_record_size_is_bounded_before_dependency_parsing() {
    let mut fixture = zip64_fixture();
    let zip64_offset = fixture.eocd_offset - 20 - 56;
    fixture.bytes[zip64_offset + 4..zip64_offset + 12].copy_from_slice(&u64::MAX.to_le_bytes());
    let file = TempArchive::new(&fixture.bytes);
    assert!(open_with_limits(&file.path, &ArchiveLimits::default()).is_err());
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn impossible_central_directory_counts_and_ranges_are_rejected() {
    let base = zip_fixture(&[EntryFixture::stored(b"a", b"b")], b"");
    let mut excessive_count = base.bytes.clone();
    excessive_count[base.eocd_offset + 8..base.eocd_offset + 12]
        .copy_from_slice(&[0xfe, 0xff, 0xfe, 0xff]);
    let count_file = TempArchive::new(&excessive_count);
    assert!(open_with_limits(&count_file.path, &ArchiveLimits::default()).is_err());
    let mut out_of_file = base.bytes;
    out_of_file[base.eocd_offset + 16..base.eocd_offset + 20]
        .copy_from_slice(&0xffff_fffe_u32.to_le_bytes());
    let range_file = TempArchive::new(&out_of_file);
    assert!(open_with_limits(&range_file.path, &ArchiveLimits::default()).is_err());
}

#[test]
fn metadata_count_read_and_time_limits_are_enforced() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"metadata.txt", b"payload")], b"");
    let file = TempArchive::new(&fixture.bytes);
    let limits = [
        ArchiveLimits {
            max_entries: 0,
            ..ArchiveLimits::default()
        },
        ArchiveLimits {
            max_central_directory_bytes: 1,
            ..ArchiveLimits::default()
        },
        ArchiveLimits {
            max_metadata_bytes: 0,
            ..ArchiveLimits::default()
        },
        ArchiveLimits {
            max_read_bytes: 1,
            ..ArchiveLimits::default()
        },
        ArchiveLimits {
            operation_timeout: Duration::ZERO,
            ..ArchiveLimits::default()
        },
    ];
    for limits in limits {
        let error = open_with_limits(&file.path, &limits).unwrap_err();
        assert!(
            matches!(error, ArchiveError::LimitExceeded(_)),
            "{limits:?}: {error}"
        );
    }
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn preview_limits_reject_large_declared_or_compressed_entries() {
    let mut entry = EntryFixture::stored(b"large.txt", b"x");
    entry.declared_size = Some(256 * 1024 + 1);
    let fixture = zip_fixture(&[entry], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert!(matches!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        ),
        Err(ArchiveError::LimitExceeded(_))
    ));
    let small = zip_fixture(&[EntryFixture::stored(b"small", b"data")], b"");
    let small_file = TempArchive::new(&small.bytes);
    let small_snapshot = small_file.open();
    for limits in [
        ArchiveLimits {
            max_preview_bytes: 0,
            ..ArchiveLimits::default()
        },
        ArchiveLimits {
            max_preview_compressed_bytes: 0,
            ..ArchiveLimits::default()
        },
    ] {
        assert!(matches!(
            preview_entry(&small_snapshot, 0, &limits, &CancellationToken::new()),
            Err(ArchiveError::LimitExceeded(_))
        ));
    }
}

#[test]
fn cancellation_reaches_open_reinterpret_and_preview_without_source_changes() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"file", b"data")], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let token = CancellationToken::new();
    token.clone().cancel();
    assert!(token.is_cancelled());
    assert_eq!(
        open_archive(
            &file.path,
            &FilenamePolicy::default(),
            &ArchiveLimits::default(),
            &token
        )
        .unwrap_err(),
        ArchiveError::Cancelled
    );
    assert_eq!(
        reinterpret(&snapshot, &FilenamePolicy::default(), &token).unwrap_err(),
        ArchiveError::Cancelled
    );
    assert_eq!(
        preview_entry(&snapshot, 0, &ArchiveLimits::default(), &token).unwrap_err(),
        ArchiveError::Cancelled
    );
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn same_bytes_and_timestamp_at_a_replaced_path_do_not_reuse_a_snapshot() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"file", b"data")], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let original_modified = fs::metadata(&file.path).unwrap().modified().unwrap();
    let moved = file.directory.join("original.zip");
    fs::rename(&file.path, &moved).unwrap();
    fs::write(&file.path, &fixture.bytes).unwrap();
    File::options()
        .write(true)
        .open(&file.path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(original_modified))
        .unwrap();
    assert_eq!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        )
        .unwrap_err(),
        ArchiveError::SourceChanged
    );
    assert_eq!(fs::read(&moved).unwrap(), fixture.bytes);
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn changed_central_metadata_with_restored_timestamp_invalidates_the_snapshot() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"original.txt", b"data")], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let original_modified = fs::metadata(&file.path).unwrap().modified().unwrap();
    let mut writer = OpenOptions::new().write(true).open(&file.path).unwrap();
    writer
        .seek(SeekFrom::Start((fixture.central_headers[0] + 46) as u64))
        .unwrap();
    writer.write_all(b"O").unwrap();
    writer
        .set_times(FileTimes::new().set_modified(original_modified))
        .unwrap();
    drop(writer);
    assert_eq!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        )
        .unwrap_err(),
        ArchiveError::SourceChanged
    );
}

#[test]
fn source_truncation_invalidates_the_snapshot() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"file", b"data")], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    File::options()
        .write(true)
        .open(&file.path)
        .unwrap()
        .set_len(1)
        .unwrap();
    assert_eq!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        )
        .unwrap_err(),
        ArchiveError::SourceChanged
    );
}

#[test]
fn local_name_disagreement_is_visible_in_listing_but_blocks_preview() {
    let mut entry = EntryFixture::stored(b"central.txt", b"data");
    entry.local_name = Some(b"another.txt".to_vec());
    let fixture = zip_fixture(&[entry], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries[0].name.display, "central.txt");
    assert!(matches!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        ),
        Err(ArchiveError::InvalidArchive(_))
    ));
    assert_eq!(snapshot.entries[0].name.display, "central.txt");
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn unsupported_compression_and_encryption_remain_listed_but_cannot_be_previewed() {
    let mut encrypted = EntryFixture::stored(b"encrypted", b"not actually encrypted");
    encrypted.flags = 1;
    let mut unsupported = EntryFixture::stored(b"bzip2", b"not actually bzip2");
    unsupported.method = 12;
    let fixture = zip_fixture(&[encrypted, unsupported], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert!(snapshot.entries[0].encrypted);
    assert_eq!(snapshot.entries[1].compression_method, 12);
    for entry in &snapshot.entries {
        assert!(matches!(
            preview_entry(
                &snapshot,
                entry.id,
                &ArchiveLimits::default(),
                &CancellationToken::new()
            ),
            Err(ArchiveError::Unsupported(_))
        ));
    }
}

#[test]
fn signed_and_unsigned_data_descriptors_are_checked_for_selected_entries() {
    for signature in [false, true] {
        let mut entry = EntryFixture::deflated(b"descriptor.txt", b"descriptor payload");
        entry.descriptor = Some(signature);
        let fixture = zip_fixture(&[entry], b"");
        let file = TempArchive::new(&fixture.bytes);
        let snapshot = file.open();
        assert_eq!(preview(&snapshot, 0).bytes, b"descriptor payload");
        let mut corrupt = fixture.bytes;
        let crc_offset = fixture.descriptor_offsets[0].unwrap() + if signature { 4 } else { 0 };
        corrupt[crc_offset] ^= 1;
        let corrupt_file = TempArchive::new(&corrupt);
        let corrupt_snapshot = corrupt_file.open();
        assert!(
            preview_entry(
                &corrupt_snapshot,
                0,
                &ArchiveLimits::default(),
                &CancellationToken::new()
            )
            .is_err()
        );
    }
}

#[test]
fn snapshots_cancellation_and_errors_can_cross_worker_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ArchiveSnapshot>();
    assert_send_sync::<ArchiveError>();
    assert_send_sync::<CancellationToken>();
    let fixture = zip_fixture(
        &[
            EntryFixture::stored(b"one", &vec![1; 8192]),
            EntryFixture::stored(b"two", &vec![2; 8192]),
        ],
        b"",
    );
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    let first_snapshot = snapshot.clone();
    let first = std::thread::spawn(move || preview(&first_snapshot, 0));
    let second = std::thread::spawn(move || preview(&snapshot, 1));
    assert_eq!(first.join().unwrap().bytes, vec![1; 8192]);
    assert_eq!(second.join().unwrap().bytes, vec![2; 8192]);
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn explicit_recovery_allows_preview_of_conflicting_unicode_declarations() {
    let mut entry = EntryFixture::stored(b"original.txt", b"selected payload");
    entry.flags = 1 << 11;
    entry.extra = unicode_path(&entry.name, "different.txt");
    let fixture = zip_fixture(&[entry], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries[0].name.state, NameState::Conflict);
    assert!(matches!(
        preview_entry(
            &snapshot,
            0,
            &ArchiveLimits::default(),
            &CancellationToken::new()
        ),
        Err(ArchiveError::InvalidArchive(_))
    ));
    let policy = FilenamePolicy {
        archive: FilenameChoice {
            encoding: Some(FilenameEncoding::Cp437),
            recovery: true,
        },
        ..FilenamePolicy::default()
    };
    let recovered = reinterpret(&snapshot, &policy, &CancellationToken::new()).unwrap();
    assert_eq!(recovered.id, snapshot.id);
    assert_eq!(recovered.entries[0].id, snapshot.entries[0].id);
    assert_eq!(recovered.entries[0].name.display, "original.txt");
    assert_eq!(recovered.entries[0].name.state, NameState::Resolved);
    assert_eq!(preview(&recovered, 0).bytes, b"selected payload");
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn metadata_revision_is_reproducible_but_snapshot_identity_is_per_open() {
    let fixture = zip_fixture(&[EntryFixture::stored(b"name.txt", b"payload")], b"comment");
    let file = TempArchive::new(&fixture.bytes);
    let first = file.open();
    let second = file.open();
    assert_ne!(first.id, second.id);
    assert_eq!(first.metadata_revision, second.metadata_revision);
    assert_eq!(first.metadata_revision.len(), 64);
    let policy = FilenamePolicy {
        archive: FilenameChoice {
            encoding: Some(FilenameEncoding::Cp866),
            recovery: false,
        },
        ..FilenamePolicy::default()
    };
    let interpreted = reinterpret(&first, &policy, &CancellationToken::new()).unwrap();
    assert_eq!(interpreted.id, first.id);
    assert_eq!(interpreted.metadata_revision, first.metadata_revision);
    let mut changed = fixture.bytes.clone();
    changed[fixture.central_headers[0] + 46] = b'N';
    let changed_file = TempArchive::new(&changed);
    assert_ne!(
        changed_file.open().metadata_revision,
        first.metadata_revision
    );
}

#[test]
fn actual_deflate_expansion_past_declared_size_never_returns_a_partial_preview() {
    let mut entry = EntryFixture::deflated(b"understated.txt", &b"expanded payload\n".repeat(1024));
    entry.declared_size = Some(8);
    let fixture = zip_fixture(&[entry], b"");
    let file = TempArchive::new(&fixture.bytes);
    let snapshot = file.open();
    assert_eq!(snapshot.entries[0].size, 8);
    let limits = ArchiveLimits {
        max_preview_bytes: 16,
        ..ArchiveLimits::default()
    };
    assert!(matches!(
        preview_entry(&snapshot, 0, &limits, &CancellationToken::new()),
        Err(ArchiveError::InvalidArchive(_))
    ));
    assert_eq!(fs::read(&file.path).unwrap(), fixture.bytes);
}

#[test]
fn zip64_allocation_headers_are_validated_after_copy_and_frozen_during_discovery() {
    use super::provider::OpenTestStage;
    for stage in [OpenTestStage::BeforeFreeze, OpenTestStage::BeforeDiscovery] {
        let fixture = zip64_fixture();
        let file = TempArchive::new(&fixture.bytes);
        let modified = fs::metadata(&file.path).unwrap().modified().unwrap();
        let record_size_offset = (fixture.eocd_offset - 20 - 56 + 4) as u64;
        let mutation = |current_stage: OpenTestStage, path: &Path| {
            if current_stage != stage {
                return;
            }
            let mut source = OpenOptions::new().write(true).open(path).unwrap();
            source.seek(SeekFrom::Start(record_size_offset)).unwrap();
            source.write_all(&u64::MAX.to_le_bytes()).unwrap();
            // Force byte-level checks instead of an mtime-only failure.
            source
                .set_times(FileTimes::new().set_modified(modified))
                .unwrap();
        };
        let result = super::provider::open_archive_with_metadata_hook(
            &file.path,
            &FilenamePolicy::default(),
            &ArchiveLimits::default(),
            &CancellationToken::new(),
            &mutation,
        );
        match stage {
            OpenTestStage::BeforeFreeze => {
                assert!(matches!(result, Err(ArchiveError::LimitExceeded(_))));
            }
            OpenTestStage::BeforeDiscovery => {
                assert!(matches!(result, Err(ArchiveError::SourceChanged)));
            }
        }
        let after = fs::read(&file.path).unwrap();
        assert_eq!(after.len(), fixture.bytes.len());
        assert_eq!(
            &after[record_size_offset as usize..record_size_offset as usize + 8],
            &u64::MAX.to_le_bytes()
        );
    }
}

#[test]
fn redirected_zip64_locator_cannot_allocate_an_unchecked_extensible_sector() {
    use super::provider::OpenTestStage;
    for stage in [OpenTestStage::BeforeFreeze, OpenTestStage::BeforeDiscovery] {
        let mut fixture = zip64_fixture_with_payload(&vec![0; 70 * 1024]);
        let locator_offset = fixture.eocd_offset - 20;
        let real_zip64_offset = locator_offset - 56;
        let directory = fixture.bytes[fixture.central_offset..real_zip64_offset].to_vec();
        let alternate_directory_offset = fixture.data_offsets[0];
        let alternate_end_offset = alternate_directory_offset + directory.len();
        let record_size = (locator_offset - alternate_end_offset - 12) as u64;
        // The alternative passes zip 9.8.2's physical length, disk and minimum
        // directory-size checks BEFORE read_extensible_data_sector allocates.
        assert!(record_size - 44 > 64 * 1024);
        assert_eq!(
            alternate_end_offset as u64 + 12 + record_size,
            locator_offset as u64
        );
        assert!(alternate_end_offset >= alternate_directory_offset + 46);
        fixture.bytes[alternate_directory_offset..alternate_end_offset].copy_from_slice(&directory);
        let mut alternate_end = Vec::new();
        push32(&mut alternate_end, 0x0606_4b50);
        push64(&mut alternate_end, record_size);
        push16(&mut alternate_end, 45);
        push16(&mut alternate_end, 45);
        push32(&mut alternate_end, 0);
        push32(&mut alternate_end, 0);
        push64(&mut alternate_end, 1);
        push64(&mut alternate_end, 1);
        push64(&mut alternate_end, directory.len() as u64);
        push64(&mut alternate_end, alternate_directory_offset as u64);
        assert_eq!(alternate_end.len(), 56);
        fixture.bytes[alternate_end_offset..alternate_end_offset + 56]
            .copy_from_slice(&alternate_end);
        let file = TempArchive::new(&fixture.bytes);
        // Initially the normal, small EOCD64 is selected; no payload CRC is
        // claimed by listing the deliberately adversarial payload bytes.
        assert_eq!(file.open().entries.len(), 1);
        let modified = fs::metadata(&file.path).unwrap().modified().unwrap();
        let mutation = |current_stage: OpenTestStage, path: &Path| {
            if current_stage != stage {
                return;
            }
            let mut source = OpenOptions::new().write(true).open(path).unwrap();
            source
                .seek(SeekFrom::Start((locator_offset + 8) as u64))
                .unwrap();
            source
                .write_all(&(alternate_end_offset as u64).to_le_bytes())
                .unwrap();
            source
                .set_times(FileTimes::new().set_modified(modified))
                .unwrap();
        };
        let result = super::provider::open_archive_with_metadata_hook(
            &file.path,
            &FilenamePolicy::default(),
            &ArchiveLimits::default(),
            &CancellationToken::new(),
            &mutation,
        );
        match stage {
            OpenTestStage::BeforeFreeze => {
                assert!(matches!(
                    result,
                    Err(ArchiveError::InvalidArchive(_) | ArchiveError::LimitExceeded(_))
                ));
            }
            OpenTestStage::BeforeDiscovery => {
                assert!(matches!(result, Err(ArchiveError::SourceChanged)));
            }
        }
        let after = fs::read(&file.path).unwrap();
        assert_eq!(after.len(), fixture.bytes.len());
        assert_eq!(
            &after[locator_offset + 8..locator_offset + 16],
            &(alternate_end_offset as u64).to_le_bytes()
        );
    }
}
