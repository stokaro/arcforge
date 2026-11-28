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

#[cfg(unix)]
#[test]
fn non_utf8_archive_path_is_reopened_using_original_os_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let path = fixture
        .0
        .join(std::ffi::OsString::from_vec(b"archive-\xff.zip".to_vec()));
    fs::rename(fixture.path(), &path).unwrap();
    let preview = success(cli("preview", &path, &["--json", "--entry", "0"]));
    assert_eq!(preview["data"]["preview"]["text"], "Hello from ArcForge.\n");
}
