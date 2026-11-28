use super::*;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::archive::{
    ArchiveLimits, FilenameChoice, FilenameEncoding, FilenamePolicy, open_archive,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(entries: &[Entry<'_>]) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "arcforge-extraction-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("source.zip"), zip_bytes(entries)).unwrap();
        Self(directory)
    }
    fn archive(&self) -> PathBuf {
        self.0.join("source.zip")
    }
    fn output(&self) -> PathBuf {
        self.0.join("output")
    }
    fn snapshot(&self) -> ArchiveSnapshot {
        open_archive(
            &self.archive(),
            &FilenamePolicy::default(),
            &ArchiveLimits::default(),
            &CancellationToken::new(),
        )
        .unwrap()
    }
    fn extract(&self, selection: ExtractionSelection) -> Result<ExtractionReport, ArchiveError> {
        extract_archive(
            &self.snapshot(),
            &selection,
            &self.output(),
            &ExtractionLimits::default(),
            &CancellationToken::new(),
            &mut |_| {},
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Entry<'a> {
    name: &'a [u8],
    data: &'a [u8],
    flags: u16,
    mode: u32,
    bad_crc: bool,
}
impl<'a> Entry<'a> {
    fn file(name: &'a str, data: &'a [u8]) -> Self {
        Self {
            name: name.as_bytes(),
            data,
            flags: 0x800,
            mode: 0o100644,
            bad_crc: false,
        }
    }
    fn directory(name: &'a str) -> Self {
        Self {
            name: name.as_bytes(),
            data: b"",
            flags: 0x800,
            mode: 0o40755,
            bad_crc: false,
        }
    }
}

fn zip_bytes(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for entry in entries {
        let offset = bytes.len() as u32;
        let crc = crc32fast::hash(entry.data) ^ u32::from(entry.bad_crc);
        bytes.extend_from_slice(&0x04034b50_u32.to_le_bytes());
        for word in [20, entry.flags, 0, 0, 0] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        for word in [crc, entry.data.len() as u32, entry.data.len() as u32] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(entry.name);
        bytes.extend_from_slice(entry.data);
        central.extend_from_slice(&0x02014b50_u32.to_le_bytes());
        for word in [0x0314, 20, entry.flags, 0, 0, 0] {
            central.extend_from_slice(&word.to_le_bytes());
        }
        for word in [crc, entry.data.len() as u32, entry.data.len() as u32] {
            central.extend_from_slice(&word.to_le_bytes());
        }
        for word in [entry.name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&word.to_le_bytes());
        }
        central.extend_from_slice(&(entry.mode << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(entry.name);
    }
    let offset = bytes.len() as u32;
    let size = central.len() as u32;
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(&0x06054b50_u32.to_le_bytes());
    for word in [0u16, 0, entries.len() as u16, entries.len() as u16] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

#[test]
fn all_and_selected_extraction_preserve_payloads_and_source_bytes() {
    for selection in [
        ExtractionSelection::All,
        ExtractionSelection::Entries(vec![1]),
    ] {
        let fixture = Fixture::new(&[
            Entry::file("hello.txt", b"hello"),
            Entry::file("folder/Привет.txt", "Привет\n".as_bytes()),
        ]);
        let source = fs::read(fixture.archive()).unwrap();
        let report = fixture.extract(selection.clone()).unwrap();
        assert_eq!(report.status, ExtractionStatus::Completed);
        assert!(report.error.is_none() && report.incomplete.is_none());
        assert_eq!(
            fs::read(fixture.output().join("folder/Привет.txt")).unwrap(),
            "Привет\n".as_bytes()
        );
        assert_eq!(
            fixture.output().join("hello.txt").exists(),
            selection == ExtractionSelection::All
        );
        assert_eq!(fs::read(fixture.archive()).unwrap(), source);
        assert_eq!(report.directories_created, 1);
    }
}

#[test]
fn independent_deflate_fixture_extracts_both_complete_files() {
    let fixture = Fixture::new(&[]);
    let hex: String = include_str!("../../../../tests/fixtures/python-zipfile.hex")
        .split_whitespace()
        .collect();
    let bytes: Vec<_> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    fs::write(fixture.archive(), &bytes).unwrap();
    let report = fixture.extract(ExtractionSelection::All).unwrap();
    assert_eq!(report.status, ExtractionStatus::Completed);
    assert_eq!(report.completed_entries, [0, 1]);
    assert_eq!(report.bytes_written, 44);
    assert_eq!(
        fs::read(fixture.output().join("hello.txt")).unwrap(),
        b"Hello from ArcForge.\n"
    );
    assert_eq!(
        fs::read(fixture.output().join("folder/Привет.txt")).unwrap(),
        "Привет из ZIP.\n".as_bytes()
    );
    assert_eq!(fs::read(fixture.archive()).unwrap(), bytes);
}

#[test]
fn selecting_a_directory_expands_descendants_by_stable_entry_id() {
    let fixture = Fixture::new(&[
        Entry::directory("folder/"),
        Entry::file("folder/a", b"a"),
        Entry::file("folder/nested/b", b"b"),
        Entry::file("other", b"x"),
    ]);
    let report = fixture
        .extract(ExtractionSelection::Entries(vec![0, 1, 0]))
        .unwrap();
    assert_eq!(report.status, ExtractionStatus::Completed);
    assert_eq!(report.completed_entries, [0, 1, 2]);
    assert!(!fixture.output().join("other").exists());
    assert_eq!(
        fs::read(fixture.output().join("folder/nested/b")).unwrap(),
        b"b"
    );
}

#[test]
fn unsafe_paths_are_rejected_before_destination_creation() {
    for name in [
        "../outside",
        "/absolute",
        "C:drive",
        "C:/drive",
        "\\\\server\\share",
        "folder\\file",
        "folder//file",
        "./file",
        "folder/../file",
        "nul.txt",
        "CON .txt",
        "COM¹",
        "COM0",
        "COM0.txt",
        "LPT0",
        "con\u{2003}.txt",
        "COM0\u{a0}",
        "lpt9.txt",
        "file:stream",
        "folder./file",
        "file ",
        "a\0b",
        "a\nb",
        "a?b",
    ] {
        let fixture = Fixture::new(&[Entry::file(name, b"data")]);
        let result = fixture.extract(ExtractionSelection::All);
        assert!(
            matches!(result, Err(ArchiveError::UnsafeEntryPath { .. })),
            "{name:?}: {result:?}"
        );
        assert!(!fixture.output().exists(), "{name:?}");
    }
}

#[test]
fn duplicate_case_unicode_and_prefix_collisions_fail_before_writes() {
    for (a, b) in [
        ("file", "file"),
        ("file", "FILE"),
        ("é.txt", "e\u{301}.txt"),
        ("straße", "STRASSE"),
        ("Folder/a", "folder/b"),
        ("a", "a/b"),
    ] {
        let fixture = Fixture::new(&[Entry::file(a, b"one"), Entry::file(b, b"two")]);
        let result = fixture.extract(ExtractionSelection::All);
        assert!(
            matches!(result, Err(ArchiveError::PathCollision { .. })),
            "{a:?}/{b:?}: {result:?}"
        );
        assert!(!fixture.output().exists());
    }
}

#[test]
fn provisional_names_require_an_explicit_choice_before_writing_paths() {
    let fixture = Fixture::new(&[Entry {
        name: b"\xe0.txt",
        data: b"legacy",
        flags: 0,
        mode: 0o100644,
        bad_crc: false,
    }]);
    assert!(
        matches!(fixture.extract(ExtractionSelection::All), Err(ArchiveError::FilenameEncodingRequired(ids)) if ids == [0])
    );
    assert!(!fixture.output().exists());
    let policy = FilenamePolicy {
        archive: FilenameChoice {
            encoding: Some(FilenameEncoding::Cp866),
            recovery: false,
        },
        ..Default::default()
    };
    let snapshot = open_archive(
        &fixture.archive(),
        &policy,
        &ArchiveLimits::default(),
        &CancellationToken::new(),
    )
    .unwrap();
    let report = extract_archive(
        &snapshot,
        &ExtractionSelection::All,
        &fixture.output(),
        &ExtractionLimits::default(),
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.status, ExtractionStatus::Completed);
    assert_eq!(fs::read(fixture.output().join("р.txt")).unwrap(), b"legacy");
}

#[test]
fn links_special_files_and_existing_destinations_are_never_overwritten() {
    for mode in [0o120777, 0o010600, 0o020600] {
        let fixture = Fixture::new(&[Entry {
            mode,
            ..Entry::file("link", b"../outside")
        }]);
        assert!(matches!(
            fixture.extract(ExtractionSelection::All),
            Err(ArchiveError::Unsupported(_))
        ));
        assert!(!fixture.output().exists());
    }
    for directory in [false, true] {
        let fixture = Fixture::new(&[Entry::file("file", b"new")]);
        if directory {
            fs::create_dir(fixture.output()).unwrap();
            fs::write(fixture.output().join("sentinel"), b"keep").unwrap();
        } else {
            fs::write(fixture.output(), b"keep").unwrap();
        }
        assert!(matches!(
            fixture.extract(ExtractionSelection::All),
            Err(ArchiveError::DestinationExists(_))
        ));
        let sentinel = if directory {
            fixture.output().join("sentinel")
        } else {
            fixture.output()
        };
        assert_eq!(fs::read(sentinel).unwrap(), b"keep");
    }
}

#[test]
fn limits_invalid_selection_and_precancel_create_no_destination() {
    let fixture = Fixture::new(&[Entry::file("nested/file", b"contents")]);
    let snapshot = fixture.snapshot();
    let mut limits = ExtractionLimits::default();
    limits.stream.max_total_bytes = 2;
    assert!(matches!(
        extract_archive(
            &snapshot,
            &ExtractionSelection::All,
            &fixture.output(),
            &limits,
            &CancellationToken::new(),
            &mut |_| {}
        ),
        Err(ArchiveError::LimitExceeded(_))
    ));
    assert!(matches!(
        fixture.extract(ExtractionSelection::Entries(vec![9])),
        Err(ArchiveError::EntryNotFound(9))
    ));
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        extract_archive(
            &snapshot,
            &ExtractionSelection::All,
            &fixture.output(),
            &ExtractionLimits::default(),
            &token,
            &mut |_| {}
        )
        .unwrap_err(),
        ArchiveError::Cancelled
    );
    assert!(!fixture.output().exists());
}

#[test]
fn path_and_plan_budgets_are_enforced_before_destination_creation() {
    let fixture = Fixture::new(&[Entry::file("one/two/file", b"data")]);
    let snapshot = fixture.snapshot();
    for limits in [
        ExtractionLimits {
            max_path_depth: 2,
            ..Default::default()
        },
        ExtractionLimits {
            max_path_bytes: 5,
            ..Default::default()
        },
        ExtractionLimits {
            max_plan_bytes: 4,
            ..Default::default()
        },
        ExtractionLimits {
            max_output_paths: 2,
            ..Default::default()
        },
    ] {
        assert!(
            extract_archive(
                &snapshot,
                &ExtractionSelection::All,
                &fixture.output(),
                &limits,
                &CancellationToken::new(),
                &mut |_| {}
            )
            .is_err()
        );
        assert!(!fixture.output().exists());
    }
}

#[cfg(unix)]
#[test]
fn existing_destination_symlink_is_not_followed_or_replaced() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(&[Entry::file("file", b"new")]);
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"keep").unwrap();
    symlink(&outside, fixture.output()).unwrap();
    assert!(matches!(
        fixture.extract(ExtractionSelection::All),
        Err(ArchiveError::DestinationExists(_))
    ));
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"keep");
    assert!(!outside.join("file").exists());
    assert!(
        fs::symlink_metadata(fixture.output())
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn crc_failure_retains_verified_files_and_reports_the_incomplete_file() {
    let fixture = Fixture::new(&[
        Entry::file("good", b"good"),
        Entry {
            bad_crc: true,
            ..Entry::file("broken", b"broken")
        },
        Entry::file("later", b"later"),
    ]);
    let source = fs::read(fixture.archive()).unwrap();
    let report = fixture.extract(ExtractionSelection::All).unwrap();
    assert_eq!(report.status, ExtractionStatus::Failed);
    assert_eq!(report.completed_entries, [0]);
    assert_eq!(report.files_written, 1);
    let partial = report.incomplete.unwrap();
    assert_eq!(partial.entry_id, 1);
    assert_eq!(
        partial.bytes_written,
        fs::metadata(fixture.output().join("broken")).unwrap().len()
    );
    assert_eq!(fs::read(fixture.output().join("good")).unwrap(), b"good");
    assert!(!fixture.output().join("later").exists());
    assert_eq!(fs::read(fixture.archive()).unwrap(), source);
}

#[test]
fn cancellation_during_streaming_reports_actual_partial_bytes() {
    let contents = vec![b'x'; 100_000];
    let fixture = Fixture::new(&[
        Entry::file("large", &contents),
        Entry::file("later", b"later"),
    ]);
    let token = CancellationToken::new();
    let report = extract_archive(
        &fixture.snapshot(),
        &ExtractionSelection::All,
        &fixture.output(),
        &ExtractionLimits::default(),
        &token,
        &mut |value| {
            if value.bytes_written > 0 {
                token.cancel();
            }
        },
    )
    .unwrap();
    assert_eq!(report.status, ExtractionStatus::Cancelled);
    assert_eq!(report.error, Some(ArchiveError::Cancelled));
    assert!(report.completed_entries.is_empty());
    let partial = report.incomplete.unwrap();
    assert!(partial.bytes_written > 0 && partial.bytes_written < contents.len() as u64);
    assert_eq!(
        fs::metadata(fixture.output().join("large")).unwrap().len(),
        partial.bytes_written
    );
    assert!(!fixture.output().join("later").exists());
}

#[test]
fn extraction_deadline_includes_time_spent_before_streaming() {
    use std::time::Duration;
    let contents = vec![b'x'; 100_000];
    let fixture = Fixture::new(&[Entry::file("large", &contents)]);
    let mut limits = ExtractionLimits::default();
    limits.stream.operation_timeout = Duration::from_secs(2);
    let mut delayed_after_write = false;
    let report = extract_archive(
        &fixture.snapshot(),
        &ExtractionSelection::All,
        &fixture.output(),
        &limits,
        &CancellationToken::new(),
        &mut |value| {
            if value.phase == ExtractionPhase::Checking {
                std::thread::sleep(Duration::from_secs(1));
            } else if value.bytes_written > 0 && !delayed_after_write {
                delayed_after_write = true;
                std::thread::sleep(Duration::from_millis(1200));
            }
        },
    )
    .unwrap();
    assert_eq!(report.status, ExtractionStatus::Failed);
    assert!(matches!(report.error, Some(ArchiveError::LimitExceeded(_))));
    // The single operation deadline must stop this file, not restart after
    // preflight and allow its remaining bytes before reporting a late timeout.
    assert!(report.completed_entries.is_empty());
    let incomplete = report.incomplete.unwrap();
    assert!(incomplete.bytes_written > 0 && incomplete.bytes_written < contents.len() as u64);
    assert_eq!(
        fs::metadata(fixture.output().join("large")).unwrap().len(),
        incomplete.bytes_written
    );
}

#[test]
fn concurrent_hardlink_at_output_leaf_is_not_truncated_or_claimed_as_partial() {
    let fixture = Fixture::new(&[Entry::file("file", b"new")]);
    let sentinel = fixture.0.join("sentinel");
    fs::write(&sentinel, b"keep").unwrap();
    let report = extract_impl(
        &fixture.snapshot(),
        &ExtractionSelection::All,
        &fixture.output(),
        &ExtractionLimits::default(),
        &CancellationToken::new(),
        &mut |_| {},
        Some(&|stage| {
            if stage == TestStage::BeforeEntry(0) {
                fs::hard_link(&sentinel, fixture.output().join("file")).unwrap();
            }
        }),
    )
    .unwrap();
    assert_eq!(report.status, ExtractionStatus::Failed);
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
    assert_eq!(report.bytes_written, 0);
    assert!(report.incomplete.is_none());
}

#[cfg(unix)]
#[test]
fn parent_replacement_is_detected_before_destination_creation() {
    let fixture = Fixture::new(&[Entry::file("file", b"new")]);
    let parent = fixture.0.join("parent");
    let moved = fixture.0.join("moved");
    let outside = fixture.0.join("outside");
    fs::create_dir(&parent).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"keep").unwrap();
    let result = extract_impl(
        &fixture.snapshot(),
        &ExtractionSelection::All,
        &parent.join("new"),
        &ExtractionLimits::default(),
        &CancellationToken::new(),
        &mut |_| {},
        Some(&|stage| {
            if stage == TestStage::ParentOpened {
                fs::rename(&parent, &moved).unwrap();
                std::os::unix::fs::symlink(&outside, &parent).unwrap();
            }
        }),
    );
    assert_eq!(result.unwrap_err(), ArchiveError::DestinationChanged);
    assert!(!outside.join("new").exists());
    assert!(!moved.join("new").exists());
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn directory_and_destination_symlink_swaps_cannot_escape() {
    for replace_root in [false, true] {
        let fixture = Fixture::new(&[
            Entry::directory("nested/"),
            Entry::file("nested/file", b"new"),
        ]);
        let outside = fixture.0.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("file"), b"keep").unwrap();
        let report = extract_impl(
            &fixture.snapshot(),
            &ExtractionSelection::All,
            &fixture.output(),
            &ExtractionLimits::default(),
            &CancellationToken::new(),
            &mut |_| {},
            Some(&|stage| {
                if replace_root && stage == TestStage::DestinationOpened {
                    fs::rename(fixture.output(), fixture.0.join("retained")).unwrap();
                    std::os::unix::fs::symlink(&outside, fixture.output()).unwrap();
                } else if !replace_root && stage == TestStage::BeforeEntry(1) {
                    fs::remove_dir(fixture.output().join("nested")).unwrap();
                    std::os::unix::fs::symlink(&outside, fixture.output().join("nested")).unwrap();
                }
            }),
        )
        .unwrap();
        assert_eq!(report.status, ExtractionStatus::Failed);
        assert_eq!(fs::read(outside.join("file")).unwrap(), b"keep");
    }
}

#[test]
fn unicode_destination_and_empty_archive_work_without_lossy_path_reopening() {
    let fixture = Fixture::new(&[]);
    let destination = fixture.0.join("выход 資料 𐐷");
    let report = extract_archive(
        &fixture.snapshot(),
        &ExtractionSelection::All,
        &destination,
        &ExtractionLimits::default(),
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.status, ExtractionStatus::Completed);
    assert!(destination.is_dir());
    assert_eq!(report.bytes_written, 0);
}
use std::path::PathBuf;
