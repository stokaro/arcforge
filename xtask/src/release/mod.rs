//! Release inventory and evidence for the actual delivered development bundles.

mod manifest;
mod unpack;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) use manifest::{Manifest, Target, VERSION};
use unpack::{Directory, read_small, regular_file};

use crate::{Options, Result};

const EVIDENCE_CLASS: &str = "native-command-smoke";
const SMOKE_STAGE: &str = "zip-staged-delete";
const SMOKE_EXTRACT_DIRECTORY: &str = "arcforge-smoke-output";
const SMOKE_EDITED_ZIP: &str = "arcforge-smoke-edited.zip";
const SMOKE_EXTRACT_BYTES: u64 =
    (b"Hello from ArcForge.\n".len() + "Привет из ZIP.\n".len()) as u64;

pub fn matrix(root: &Path) -> Result<()> {
    println!("{}", Manifest::load(root)?.matrix());
    Ok(())
}

pub(crate) fn source_state(root: &Path) -> Result<(Option<String>, bool)> {
    let commit = match Command::new("git")
        .args(["rev-parse", "--show-toplevel", "HEAD"])
        .current_dir(root)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((None, true)),
        Err(error) => return Err(error.into()),
    };
    if !commit.status.success() {
        return Ok((None, true));
    }
    let output = String::from_utf8(commit.stdout)?;
    let mut lines = output.lines();
    let source_root = lines.next().ok_or("git returned no source root")?;
    // A source export nested inside another checkout must not inherit its identity.
    if fs::canonicalize(source_root)? != fs::canonicalize(root)? {
        return Ok((None, true));
    }
    let commit = lines
        .next()
        .ok_or("git returned no source commit")?
        .trim()
        .to_owned();
    if !valid_commit(&commit) {
        return Err("git returned an invalid source commit".into());
    }
    let status = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=normal"])
        .current_dir(root)
        .output()?;
    if !status.status.success() {
        return Err("git could not establish source cleanliness".into());
    }
    Ok((Some(commit), !status.stdout.is_empty()))
}

pub(crate) fn valid_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn build_info(
    manifest: &Manifest,
    target: &Target,
    commit: Option<&str>,
    dirty: bool,
) -> Value {
    json!({
        "schema_version": 1,
        "version": VERSION,
        "target": target.target,
        "source_commit": commit,
        "source_dirty": dirty,
        "target_manifest_sha256": manifest.sha256,
    })
}

pub fn verify_package(root: &Path, options: &Options) -> Result<()> {
    let manifest = Manifest::load(root)?;
    let host = crate::package::host_target(root)?;
    let target = manifest.target(options.target.as_deref().unwrap_or(&host))?;
    let dist = options
        .directory
        .clone()
        .unwrap_or_else(|| root.join("dist"));
    let evidence_path = dist.join(target.evidence_filename());
    // A failing attempt must never leave an earlier success eligible for release.
    remove_if_present(&evidence_path)?;
    if target.target != host {
        return Err(format!(
            "native delivery smoke requires {0}; host is {host}",
            target.target
        )
        .into());
    }
    let (commit, dirty) = source_state(root)?;
    let expected_build = build_info(&manifest, target, commit.as_deref(), dirty);
    let (directory, digest) = inspect_package(root, &dist, target, &expected_build)?;
    let checks = smoke(&directory.0, target)?;
    let evidence = json!({
        "schema_version": 1,
        "evidence_class": EVIDENCE_CLASS,
        "archive": target.filename(),
        "archive_sha256": digest,
        "build": expected_build,
        "host_target": host,
        "checks": checks,
    });
    let temporary = evidence_path.with_extension("json.partial");
    remove_if_present(&temporary)?;
    fs::write(&temporary, serde_json::to_vec_pretty(&evidence)?)?;
    fs::rename(&temporary, &evidence_path)?;
    println!("Verified unpacked bundle: {}", target.filename());
    println!(
        "Evidence: {} ({EVIDENCE_CLASS}; no desktop interaction)",
        evidence_path.display()
    );
    Ok(())
}

pub fn verify_release(root: &Path, options: &Options) -> Result<()> {
    let commit = options
        .commit
        .as_deref()
        .ok_or("verify-release requires --commit FULL_SHA")?;
    let manifest = Manifest::load(root)?;
    let dist = options
        .directory
        .clone()
        .unwrap_or_else(|| root.join("dist"));
    verify_inventory(&manifest, &dist)?;
    for target in &manifest.targets {
        let expected_build = build_info(&manifest, target, Some(commit), false);
        let (_directory, digest) = inspect_package(root, &dist, target, &expected_build)?;
        let evidence: Value = serde_json::from_slice(&read_small(
            &dist.join(target.evidence_filename()),
            256 * 1024,
        )?)?;
        validate_evidence(&evidence, target, &expected_build, &digest)?;
    }
    println!(
        "Verified complete release: {} native bundles, {} exact assets, source {commit}",
        manifest.targets.len(),
        manifest.targets.len() * 3
    );
    Ok(())
}

fn verify_inventory(manifest: &Manifest, dist: &Path) -> Result<()> {
    let expected: BTreeSet<_> = manifest
        .targets
        .iter()
        .flat_map(Target::asset_names)
        .collect();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(dist)? {
        let entry = entry?;
        regular_file(&entry.path())?;
        actual.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 release asset")?,
        );
    }
    if expected != actual {
        return Err(format!(
            "release asset mismatch; missing: {:?}; unexpected: {:?}",
            expected.difference(&actual).collect::<Vec<_>>(),
            actual.difference(&expected).collect::<Vec<_>>()
        )
        .into());
    }
    Ok(())
}

fn inspect_package(
    root: &Path,
    dist: &Path,
    target: &Target,
    expected_build: &Value,
) -> Result<(Directory, String)> {
    let archive = dist.join(target.filename());
    unpack::check_archive(&archive)?;
    let digest = crate::package::hash_file(&archive)?;
    let checksum = read_small(&dist.join(format!("{}.sha256", target.filename())), 512)?;
    if checksum != format!("{digest}  {}\n", target.filename()).as_bytes() {
        return Err(format!("checksum mismatch for {}", target.filename()).into());
    }
    let directory = unpack::unpack(&archive, &target.archive, &target.stem())?;
    let delivered_build: Value = serde_json::from_slice(&read_small(
        &directory.0.join("BUILD-INFO.json"),
        64 * 1024,
    )?)?;
    if &delivered_build != expected_build {
        return Err(format!(
            "source/version/target metadata mismatch for {}",
            target.filename()
        )
        .into());
    }
    verify_bundle_inventory(root, &directory.0, target)?;
    for relative in [&target.cli, &target.gui] {
        crate::package::verify_binary(&directory.0.join(relative), &target.target)?;
    }
    for filename in ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "README.md"] {
        same_file(&root.join(filename), &directory.0.join(filename))?;
    }
    for relative in ["licenses", "docs"] {
        same_directory(&root.join(relative), &directory.0.join(relative))?;
    }
    for filename in ["DEPENDENCIES.json", "DEVELOPMENT-BUNDLE.txt"] {
        regular_file(&directory.0.join(filename))?;
    }
    if target.target.contains("apple") {
        let resources = directory.0.join("ArcForge.app/Contents/Resources");
        for filename in [
            "LICENSE",
            "NOTICE",
            "THIRD_PARTY_NOTICES.md",
            "DEPENDENCIES.json",
            "BUILD-INFO.json",
        ] {
            same_file(&directory.0.join(filename), &resources.join(filename))?;
        }
        same_directory(&directory.0.join("licenses"), &resources.join("licenses"))?;
        same_file(
            &directory.0.join(&target.cli),
            &directory.0.join("ArcForge.app/Contents/MacOS/arcforge-cli"),
        )?;
        regular_file(&directory.0.join("ArcForge.app/Contents/Info.plist"))?;
    }
    Ok((directory, digest))
}

fn relative_files(root: &Path, directory: &Path) -> Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            files.extend(relative_files(root, &entry.path())?);
        } else {
            regular_file(&entry.path())?;
            let relative = entry
                .path()
                .strip_prefix(root)?
                .iter()
                .map(|part| part.to_str().ok_or("non-UTF-8 packaged filename"))
                .collect::<std::result::Result<Vec<_>, _>>()?
                .join("/");
            files.insert(relative);
        }
    }
    Ok(files)
}

fn verify_bundle_inventory(root: &Path, delivered: &Path, target: &Target) -> Result<()> {
    let mut expected: BTreeSet<String> = [
        "LICENSE",
        "NOTICE",
        "THIRD_PARTY_NOTICES.md",
        "README.md",
        "BUILD-INFO.json",
        "DEPENDENCIES.json",
        "DEVELOPMENT-BUNDLE.txt",
        &target.cli,
        &target.gui,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for relative in ["docs", "licenses"] {
        expected.extend(relative_files(root, &root.join(relative))?);
    }
    if target.target.contains("apple") {
        expected.insert("ArcForge.app/Contents/Info.plist".to_owned());
        expected.insert("ArcForge.app/Contents/MacOS/arcforge-cli".to_owned());
        for relative in [
            "LICENSE",
            "NOTICE",
            "THIRD_PARTY_NOTICES.md",
            "DEPENDENCIES.json",
            "BUILD-INFO.json",
        ] {
            expected.insert(format!("ArcForge.app/Contents/Resources/{relative}"));
        }
        for relative in relative_files(root, &root.join("licenses"))? {
            expected.insert(format!("ArcForge.app/Contents/Resources/{relative}"));
        }
    }
    let actual = relative_files(delivered, delivered)?;
    if actual != expected {
        return Err(format!(
            "bundle layout mismatch; missing: {:?}; unexpected: {:?}",
            expected.difference(&actual).collect::<Vec<_>>(),
            actual.difference(&expected).collect::<Vec<_>>()
        )
        .into());
    }
    Ok(())
}

fn same_file(expected: &Path, delivered: &Path) -> Result<()> {
    regular_file(expected)?;
    regular_file(delivered)?;
    if crate::package::hash_file(expected)? != crate::package::hash_file(delivered)? {
        return Err(format!(
            "delivered content differs from source: {}",
            delivered.display()
        )
        .into());
    }
    Ok(())
}

fn same_directory(expected: &Path, delivered: &Path) -> Result<()> {
    for entry in fs::read_dir(expected)? {
        let entry = entry?;
        let destination = delivered.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            same_directory(&entry.path(), &destination)?;
        } else {
            same_file(&entry.path(), &destination)?;
        }
    }
    Ok(())
}

fn smoke_commands(target: &Target) -> Vec<(&str, Vec<&'static str>)> {
    vec![
        (&target.cli, vec!["--version"]),
        (&target.cli, vec!["commands", "--json"]),
        (&target.cli, vec!["doctor", "--json"]),
        (&target.gui, vec!["--version"]),
        (&target.cli, vec!["list", "arcforge-smoke.zip", "--json"]),
        (
            &target.cli,
            vec!["preview", "arcforge-smoke.zip", "--entry", "1", "--json"],
        ),
        (
            &target.cli,
            vec![
                "extract",
                "arcforge-smoke.zip",
                "--output",
                SMOKE_EXTRACT_DIRECTORY,
                "--json",
            ],
        ),
        (
            &target.cli,
            vec![
                "delete",
                "arcforge-smoke.zip",
                "--entry",
                "0",
                "--output",
                SMOKE_EDITED_ZIP,
                "--json",
            ],
        ),
        (
            &target.cli,
            vec!["preview", SMOKE_EDITED_ZIP, "--entry", "0", "--json"],
        ),
    ]
}

fn smoke(directory: &Path, target: &Target) -> Result<Vec<Value>> {
    // Independent Python writer fixture, installed only after inventory validation.
    let hex: String = include_str!("../../../tests/fixtures/python-zipfile.hex")
        .split_whitespace()
        .collect();
    let fixture: Vec<_> = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16))
        .collect::<std::result::Result<_, _>>()?;
    fs::write(directory.join("arcforge-smoke.zip"), &fixture)?;
    let mut checks = Vec::new();
    for (index, (program, arguments)) in smoke_commands(target).iter().enumerate() {
        let stdout_path = directory.join(format!("smoke-{index}.stdout"));
        let stderr_path = directory.join(format!("smoke-{index}.stderr"));
        let mut child = Command::new(directory.join(program))
            .args(arguments)
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout_path)?)
            .stderr(File::create(&stderr_path)?)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline
                || fs::metadata(&stdout_path)?.len() > 1024 * 1024
                || fs::metadata(&stderr_path)?.len() > 1024 * 1024
            {
                child.kill()?;
                child.wait()?;
                return Err(format!(
                    "smoke command exceeded time/output limit: {program} {arguments:?}"
                )
                .into());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let stdout = read_small(&stdout_path, 1024 * 1024)?;
        let stderr = read_small(&stderr_path, 1024 * 1024)?;
        if !status.success() || !stderr.is_empty() {
            return Err(format!(
                "unpacked smoke failed: {program} {arguments:?}: {status}; {}",
                String::from_utf8_lossy(&stderr)
            )
            .into());
        }
        validate_smoke_output(index, &stdout)?;
        if index == 6 {
            validate_extracted_fixture(directory, &fixture)?;
        } else if index == 8 {
            validate_smoke_source(directory, &fixture)?;
        }
        checks.push(json!({
            "program": program,
            "arguments": arguments,
            "exit_code": 0,
            "stdout_sha256": format!("{:x}", Sha256::digest(&stdout)),
        }));
    }
    Ok(checks)
}

fn validate_extracted_fixture(directory: &Path, original: &[u8]) -> Result<()> {
    let destination = directory.join(SMOKE_EXTRACT_DIRECTORY);
    if !fs::symlink_metadata(&destination)?.file_type().is_dir()
        || relative_files(&destination, &destination)?
            != BTreeSet::from(["hello.txt".into(), "folder/Привет.txt".into()])
    {
        return Err("unpacked CLI produced an unexpected extraction inventory".into());
    }
    for (name, expected) in [
        ("hello.txt", b"Hello from ArcForge.\n".as_slice()),
        ("folder/Привет.txt", "Привет из ZIP.\n".as_bytes()),
    ] {
        if read_small(&destination.join(name), expected.len() as u64)? != expected {
            return Err(format!("unpacked CLI extracted incorrect bytes for {name}").into());
        }
    }
    validate_smoke_source(directory, original)
}

fn validate_smoke_source(directory: &Path, original: &[u8]) -> Result<()> {
    if read_small(&directory.join("arcforge-smoke.zip"), original.len() as u64)? == original {
        Ok(())
    } else {
        Err("unpacked CLI changed the source ZIP fixture".into())
    }
}

fn validate_smoke_output(index: usize, stdout: &[u8]) -> Result<()> {
    if matches!(index, 0 | 3) {
        if stdout != format!("ArcForge {VERSION} ({SMOKE_STAGE})\n").as_bytes() {
            return Err("unpacked executable reported an unexpected version".into());
        }
    } else {
        let value: Value = serde_json::from_slice(stdout)?;
        if index == 1 {
            let commands = value["data"]
                .as_array()
                .ok_or("commands response has no array")?;
            let implemented = [
                "version", "commands", "doctor", "list", "info", "preview", "extract", "delete",
            ];
            let planned = ["create", "test", "update", "convert", "hash"];
            let mut names = BTreeSet::new();
            if value["kind"] != "commands"
                || commands.len() != implemented.len() + planned.len()
                || commands.iter().any(|command| {
                    let Some(name) = command["name"].as_str() else {
                        return true;
                    };
                    !names.insert(name)
                        || !matches!(
                            command["availability"].as_str(),
                            Some("implemented" | "planned")
                        )
                })
                || implemented.iter().any(|name| {
                    !commands
                        .iter()
                        .any(|row| row["name"] == *name && row["availability"] == "implemented")
                })
                || planned.iter().any(|name| {
                    !commands
                        .iter()
                        .any(|row| row["name"] == *name && row["availability"] == "planned")
                })
            {
                return Err("unpacked CLI violated command discovery contract".into());
            }
        } else if index == 2
            && (value["kind"] != "doctor"
                || value["data"]["application"]["name"] != "ArcForge"
                || value["data"]["application"]["version"] != VERSION
                || value["data"]["application"]["stage"] != SMOKE_STAGE
                || value["data"]["ready_for_archive_operations"] != true
                || value["data"]["archive_provider_count"] != 1
                || value["data"]["supported_archive_formats"] != json!(["zip", "zip64"])
                || value["data"]["providers"][0]["operations"]
                    != json!(["list", "info", "preview", "extract", "delete"])
                || value["data"]["providers"][0]["extraction_compression"]
                    != json!(["stored", "deflate"])
                || value["data"]["providers"][0]["write"] != false
                || value["data"]["providers"][0]["encrypted_content"] != false)
        {
            return Err("unpacked CLI violated the ZIP capability contract".into());
        } else if index == 4 {
            let entries = &value["data"]["entries"];
            let revision = value["data"]["metadata_revision"].as_str().unwrap_or("");
            if value["kind"] != "list"
                || entries.as_array().map(Vec::len) != Some(2)
                || entries[0]["id"] != 0
                || entries[1]["id"] != 1
                || entries[0]["name"]["display"] != "hello.txt"
                || entries[1]["name"]["display"] != "folder/Привет.txt"
                || entries[0]["raw_name_hex"] != "68656c6c6f2e747874"
                || entries[1]["compression_method"] != 8
                || revision.len() != 64
                || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("unpacked CLI could not list the independent ZIP fixture".into());
            }
        } else if matches!(index, 5 | 8)
            && (value["kind"] != "preview"
                || value["data"]["preview"]["entry_id"] != u8::from(index == 5)
                || value["data"]["preview"]["text"] != "Привет из ZIP.\n"
                || value["data"]["preview"]["crc32_verified"] != true)
        {
            return Err("unpacked CLI could not verify the independent Deflate fixture".into());
        } else if index == 6 {
            let report = &value["data"]["report"];
            if value["kind"] != "extract"
                || report["status"] != "completed"
                || report["destination_created"] != true
                || report["entries_planned"] != 2
                || report["completed_entries"] != json!([0, 1])
                || report["files_written"] != 2
                || report["directories_created"] != 1
                || report["bytes_written"] != SMOKE_EXTRACT_BYTES
                || !report["incomplete"].is_null()
                || !report["error"].is_null()
            {
                return Err(
                    "unpacked CLI did not complete extraction of the independent ZIP fixture"
                        .into(),
                );
            }
        } else if index == 7 {
            let report = &value["data"];
            if value["kind"] != "delete"
                || report["entries_removed"] != 1
                || report["entries_retained"] != 1
                || report["retained_crc32_verified"] != true
                || report["bytes_written"]
                    .as_u64()
                    .is_none_or(|count| count < 22)
            {
                return Err("unpacked CLI did not save the edited ZIP fixture".into());
            }
        }
    }
    Ok(())
}

fn validate_evidence(evidence: &Value, target: &Target, build: &Value, digest: &str) -> Result<()> {
    if evidence["schema_version"] != 1
        || evidence["evidence_class"] != EVIDENCE_CLASS
        || evidence["archive"] != target.filename()
        || evidence["archive_sha256"] != digest
        || &evidence["build"] != build
        || evidence["host_target"] != target.target
    {
        return Err(format!("stale/mismatched native evidence for {}", target.target).into());
    }
    let checks = evidence["checks"]
        .as_array()
        .ok_or("missing native smoke checks")?;
    let expected = smoke_commands(target);
    if checks.len() != expected.len() {
        return Err("incomplete native smoke evidence".into());
    }
    for (check, (program, arguments)) in checks.iter().zip(expected) {
        let hash = check["stdout_sha256"].as_str().unwrap_or("");
        if check["program"] != program
            || check["arguments"] != json!(arguments)
            || check["exit_code"] != 0
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid native smoke command evidence".into());
        }
    }
    Ok(())
}

pub(crate) fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
