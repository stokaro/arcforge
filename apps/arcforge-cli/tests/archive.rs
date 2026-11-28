use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "arcforge-cli-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("isolated test directory");
        let hex: String = include_str!("../../../tests/fixtures/python-zipfile.hex")
            .split_whitespace()
            .collect();
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect();
        fs::write(directory.join("sample.zip"), bytes).unwrap();
        Self(directory)
    }
    fn path(&self) -> PathBuf {
        self.0.join("sample.zip")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli(command: &str, archive: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arcforge-cli"))
        .arg(command)
        .args(args)
        .arg("--")
        .arg(archive)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run independent CLI process")
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).expect("structured success")
}

fn extract(archive: &Path, destination: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arcforge-cli"))
        .arg("extract")
        .arg("--json")
        .args(args)
        .arg("--output")
        .arg(destination)
        .arg("--")
        .arg(archive)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run extraction without a display")
}

fn archive_failure(output: Output) -> Value {
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).expect("structured failure");
    assert_eq!(value["kind"], "error");
    assert_eq!(value["data"]["exit_code"], 4);
    value
}

#[test]
fn independent_python_zip_lists_inspects_and_previews_without_changing_source() {
    let fixture = Fixture::new();
    let original = fs::read(fixture.path()).unwrap();
    let listing = success(cli("list", &fixture.path(), &["--json"]));
    assert_eq!(listing["kind"], "list");
    assert_eq!(listing["data"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(
        listing["data"]["entries"][0]["name"]["display"],
        "hello.txt"
    );
    assert_eq!(
        listing["data"]["entries"][1]["name"]["display"],
        "folder/Привет.txt"
    );
    assert_eq!(
        listing["data"]["entries"][1]["name"]["provenance"],
        "declared_utf8"
    );
    assert_eq!(
        listing["data"]["entries"][0]["raw_name_hex"],
        "68656c6c6f2e747874"
    );
    let info = success(cli("info", &fixture.path(), &["--json"]));
    assert_eq!(info["data"]["entry_count"], 2);
    assert_eq!(
        info["data"]["metadata_revision"],
        listing["data"]["metadata_revision"]
    );
    assert_eq!(info["data"]["provisional_names"], 0);
    for (id, expected) in [("0", "Hello from ArcForge.\n"), ("1", "Привет из ZIP.\n")] {
        let preview = success(cli("preview", &fixture.path(), &["--json", "--entry", id]));
        assert_eq!(preview["data"]["preview"]["text"], expected);
        assert_eq!(preview["data"]["preview"]["crc32_verified"], true);
    }
    assert_eq!(fs::read(fixture.path()).unwrap(), original);
}

#[test]
fn per_entry_choices_bind_metadata_across_processes_and_reject_stale_revisions() {
    let fixture = Fixture::new();
    let listing = success(cli("list", &fixture.path(), &["--json"]));
    let revision = listing["data"]["metadata_revision"].as_str().unwrap();
    assert_eq!(revision.len(), 64);
    let decoded = success(cli(
        "list",
        &fixture.path(),
        &[
            "--json",
            "--metadata-revision",
            revision,
            "--entry-encoding",
            "1=cp866",
        ],
    ));
    assert_eq!(
        decoded["data"]["entries"][1]["name"]["display"],
        "folder/Привет.txt"
    );
    let recovered = success(cli(
        "list",
        &fixture.path(),
        &[
            "--json",
            "--metadata-revision",
            revision,
            "--recover-entry",
            "1=cp866",
        ],
    ));
    assert_eq!(
        recovered["data"]["entries"][1]["name"]["provenance"],
        "recovery"
    );
    assert_ne!(
        recovered["data"]["entries"][1]["name"]["display"],
        "folder/Привет.txt"
    );
    assert_eq!(
        recovered["data"]["entries"][1]["raw_name_hex"],
        listing["data"]["entries"][1]["raw_name_hex"]
    );
    let mut bytes = fs::read(fixture.path()).unwrap();
    *bytes.last_mut().unwrap() ^= 1; // Change only the EOCD comment.
    fs::write(fixture.path(), bytes).unwrap();
    let rejected = cli(
        "preview",
        &fixture.path(),
        &["--json", "--entry", "0", "--metadata-revision", revision],
    );
    assert_eq!(rejected.status.code(), Some(4));
    assert!(rejected.stdout.is_empty());
    let error: Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(error["data"]["error"]["kind"], "source_changed");
}

#[test]
fn archive_errors_and_usage_are_noninteractive_and_never_report_partial_success() {
    let fixture = Fixture::new();
    for args in [
        vec!["--entry-encoding", "0=cp866"],
        vec!["--filename-encoding-mode", "force"],
        vec!["--filename-encoding", "iso-8859-1"],
        vec!["--metadata-revision", "not-a-revision"],
        vec!["--json", "--json"],
        vec!["--max-bytes", "1"],
    ] {
        let invalid = cli("list", &fixture.path(), &args);
        assert_eq!(invalid.status.code(), Some(2), "{args:?}");
        assert!(invalid.stdout.is_empty());
    }
    let limited = cli(
        "preview",
        &fixture.path(),
        &["--json", "--entry", "1", "--max-bytes", "4"],
    );
    assert_eq!(limited.status.code(), Some(4));
    assert!(limited.stdout.is_empty());
    let error: Value = serde_json::from_slice(&limited.stderr).unwrap();
    assert_eq!(error["data"]["error"]["kind"], "limit_exceeded");
    fs::write(fixture.path(), b"not a ZIP archive").unwrap();
    let broken = cli("list", &fixture.path(), &["--json"]);
    assert_eq!(broken.status.code(), Some(4));
    assert!(broken.stdout.is_empty());
    let error: Value = serde_json::from_slice(&broken.stderr).unwrap();
    assert_eq!(error["kind"], "error");
    assert_eq!(error["data"]["exit_code"], json!(4));
}

#[test]
fn native_archive_paths_are_reopened_without_lossy_conversion() {
    let names = [
        PathBuf::from("архив 資料 𐐷.zip"),
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(b"archive-\xff.zip".to_vec()))
        },
    ];
    for name in names {
        let fixture = Fixture::new();
        let path = fixture.0.join(name);
        fs::rename(fixture.path(), &path).expect("rename archive to native test path");
        let preview = success(cli("preview", &path, &["--json", "--entry", "0"]));
        assert_eq!(preview["data"]["preview"]["text"], "Hello from ArcForge.\n");
    }
}

#[test]
fn extraction_streams_all_or_selected_independent_entries_and_preserves_source() {
    let fixture = Fixture::new();
    let original = fs::read(fixture.path()).unwrap();
    for (name, args, expected_ids) in [
        ("all", vec![], json!([0, 1])),
        ("selected", vec!["--entry", "1"], json!([1])),
        (
            "repeated",
            vec!["--entry", "1", "--entry", "0"],
            json!([0, 1]),
        ),
    ] {
        let destination = fixture.0.join(name);
        let result = success(extract(&fixture.path(), &destination, &args));
        assert_eq!(result["kind"], "extract");
        let report = &result["data"]["report"];
        assert_eq!(report["status"], "completed");
        assert_eq!(report["destination_created"], true);
        assert_eq!(report["completed_entries"], expected_ids);
        assert!(report["incomplete"].is_null());
        assert!(report["error"].is_null());
        assert_eq!(
            fs::read(destination.join("folder/Привет.txt")).unwrap(),
            "Привет из ZIP.\n".as_bytes()
        );
        if name == "selected" {
            assert!(!destination.join("hello.txt").exists());
            assert_eq!(report["files_written"], 1);
        } else {
            assert_eq!(
                fs::read(destination.join("hello.txt")).unwrap(),
                b"Hello from ArcForge.\n"
            );
            assert_eq!(report["files_written"], 2);
        }
    }
    assert_eq!(fs::read(fixture.path()).unwrap(), original);
}

#[test]
fn extraction_rejects_existing_destination_and_invalid_options_before_writing() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("existing");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"unchanged").unwrap();
    let failure = archive_failure(extract(&fixture.path(), &destination, &[]));
    assert_eq!(failure["data"]["error"]["kind"], "destination_exists");
    assert_eq!(
        fs::read(destination.join("sentinel")).unwrap(),
        b"unchanged"
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    for args in [
        vec![],
        vec!["--output", ""],
        vec!["--output", "unused", "--max-bytes", "1"],
        vec!["--output", "unused", "--entry", "0", "--entry", "0"],
        vec!["--output", "one", "--output", "two"],
        vec!["--output", "unused", "--entry-encoding", "0=cp866"],
    ] {
        let invalid = cli("extract", &fixture.path(), &args);
        assert_eq!(invalid.status.code(), Some(2), "{args:?}");
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn extraction_requires_resolved_names_and_current_revision_before_creating_output() {
    let fixture = Fixture::new();
    let mut bytes = fs::read(fixture.path()).unwrap();
    let offsets: Vec<_> = bytes
        .windows(b"hello.txt".len())
        .enumerate()
        .filter_map(|(offset, name)| (name == b"hello.txt").then_some(offset))
        .collect();
    assert_eq!(offsets.len(), 2); // Independent local and central filename bytes.
    for offset in offsets {
        bytes[offset] = 0xe9;
    }
    fs::write(fixture.path(), &bytes).unwrap();
    let listing = success(cli("list", &fixture.path(), &["--json"]));
    let revision = listing["data"]["metadata_revision"].as_str().unwrap();
    let destination = fixture.0.join("unresolved");
    let failure = archive_failure(extract(&fixture.path(), &destination, &[]));
    assert_eq!(
        failure["data"]["error"]["kind"],
        "filename_encoding_required"
    );
    assert!(!destination.exists());

    let destination = fixture.0.join("resolved");
    let args = [
        "--metadata-revision",
        revision,
        "--entry-encoding",
        "0=windows-1252",
        "--entry",
        "0",
    ];
    let result = success(extract(&fixture.path(), &destination, &args));
    assert_eq!(result["data"]["report"]["completed_entries"], json!([0]));
    assert_eq!(
        fs::read(destination.join("éello.txt")).unwrap(),
        b"Hello from ArcForge.\n"
    );
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(fixture.path(), &bytes).unwrap();
    let stale_destination = fixture.0.join("stale");
    let failure = archive_failure(extract(&fixture.path(), &stale_destination, &args));
    assert_eq!(failure["data"]["error"]["kind"], "source_changed");
    assert!(!stale_destination.exists());
}

#[test]
fn crc_failure_returns_nonzero_json_and_retains_incomplete_output() {
    let fixture = Fixture::new();
    let mut bytes = fs::read(fixture.path()).unwrap();
    let offset = bytes
        .windows(b"Hello from ArcForge.\n".len())
        .position(|data| data == b"Hello from ArcForge.\n")
        .unwrap();
    bytes[offset] ^= 1;
    fs::write(fixture.path(), &bytes).unwrap();
    let destination = fixture.0.join("retained");
    let failure = archive_failure(extract(&fixture.path(), &destination, &[]));
    assert_eq!(failure["data"]["error"]["kind"], "invalid_archive");
    let report = &failure["data"]["extraction"];
    assert_eq!(report["status"], "failed");
    assert_eq!(report["destination_created"], true);
    assert_eq!(report["completed_entries"], json!([]));
    assert_eq!(report["incomplete"]["entry_id"], 0);
    assert_eq!(report["incomplete"]["path"], "hello.txt");
    let retained_bytes = fs::metadata(destination.join("hello.txt")).unwrap().len();
    assert!(retained_bytes > 0);
    assert_eq!(report["bytes_written"], retained_bytes);
    assert_eq!(report["incomplete"]["bytes_written"], retained_bytes);
    assert!(!destination.join("folder").exists());
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
}

#[test]
fn extraction_output_paths_preserve_native_unicode_and_supported_native_bytes() {
    let names = [
        PathBuf::from("результат 資料 𐐷"),
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(b"output-\xff".to_vec()))
        },
    ];
    for name in names {
        let fixture = Fixture::new();
        let destination = fixture.0.join(name);
        let report = success(extract(&fixture.path(), &destination, &["--entry", "0"]));
        assert_eq!(report["data"]["report"]["status"], "completed");
        assert_eq!(
            fs::read(destination.join("hello.txt")).unwrap(),
            b"Hello from ArcForge.\n"
        );
    }
}
