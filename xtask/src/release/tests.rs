use super::*;
use std::io::Write;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn failed_native_retry_removes_previous_success_evidence() -> Result<()> {
    let workspace = crate::workspace_root()?;
    let manifest = Manifest::load(&workspace)?;
    let host = crate::package::host_target(&workspace)?;
    let target = manifest.target(&host)?;
    let directory = Directory::new()?;
    let root = directory.0.join("source");
    let dist = directory.0.join("dist");
    fs::create_dir_all(root.join("release"))?;
    fs::create_dir(&dist)?;
    fs::copy(
        workspace.join("rust-toolchain.toml"),
        root.join("rust-toolchain.toml"),
    )?;
    fs::copy(
        workspace.join("release/targets.json"),
        root.join("release/targets.json"),
    )?;
    fs::write(dist.join(target.filename()), b"incomplete bundle")?;
    fs::write(
        dist.join(format!("{}.sha256", target.filename())),
        format!("{}  {}\n", "0".repeat(64), target.filename()),
    )?;
    let evidence = dist.join(target.evidence_filename());
    fs::write(&evidence, b"old successful evidence")?;
    let options = Options {
        target: Some(host),
        directory: Some(dist),
        ..Options::default()
    };
    let error = verify_package(&root, &options).unwrap_err();
    assert!(error.to_string().contains("checksum mismatch"), "{error}");
    assert!(
        !evidence.exists(),
        "failed retry must invalidate earlier success"
    );
    Ok(())
}

fn fixture_root(path: &Path) -> Result<()> {
    fs::create_dir_all(path.join("licenses"))?;
    fs::create_dir_all(path.join("docs"))?;
    for filename in [
        "LICENSE",
        "NOTICE",
        "THIRD_PARTY_NOTICES.md",
        "README.md",
        "licenses/example.txt",
        "docs/example.md",
    ] {
        fs::write(path.join(filename), format!("fixture {filename}\n"))?;
    }
    Ok(())
}

fn executable(target: &str) -> Vec<u8> {
    let arm = target.starts_with("aarch64");
    let mut bytes = vec![0; 128];
    if target.contains("linux") {
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[18..20].copy_from_slice(&(if arm { 183_u16 } else { 62_u16 }).to_le_bytes());
    } else if target.contains("apple") {
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        bytes[4..8].copy_from_slice(
            &(if arm {
                0x0100_000c_u32
            } else {
                0x0100_0007_u32
            })
            .to_le_bytes(),
        );
    } else {
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&(if arm { 0xaa64_u16 } else { 0x8664_u16 }).to_le_bytes());
    }
    bytes
}

fn bundle_fixture(root: &Path, dist: &Path, target: &Target, build: &Value) -> Result<String> {
    let staging = Directory::new()?;
    for filename in [
        "LICENSE",
        "NOTICE",
        "THIRD_PARTY_NOTICES.md",
        "README.md",
        "licenses/example.txt",
        "docs/example.md",
    ] {
        let destination = staging.0.join(filename);
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::copy(root.join(filename), destination)?;
    }
    for relative in [&target.cli, &target.gui] {
        let destination = staging.0.join(relative);
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::write(destination, executable(&target.target))?;
    }
    fs::write(
        staging.0.join("BUILD-INFO.json"),
        serde_json::to_vec(build)?,
    )?;
    fs::write(staging.0.join("DEPENDENCIES.json"), b"{}")?;
    fs::write(staging.0.join("DEVELOPMENT-BUNDLE.txt"), b"fixture")?;
    if target.target.contains("apple") {
        let resources = staging.0.join("ArcForge.app/Contents/Resources");
        fs::create_dir_all(resources.join("licenses"))?;
        for filename in [
            "LICENSE",
            "NOTICE",
            "THIRD_PARTY_NOTICES.md",
            "DEPENDENCIES.json",
            "BUILD-INFO.json",
            "licenses/example.txt",
        ] {
            fs::copy(staging.0.join(filename), resources.join(filename))?;
        }
        fs::copy(
            staging.0.join(&target.cli),
            staging.0.join("ArcForge.app/Contents/MacOS/arcforge-cli"),
        )?;
        fs::write(
            staging.0.join("ArcForge.app/Contents/Info.plist"),
            b"fixture",
        )?;
    }
    fs::create_dir_all(dist)?;
    let archive = dist.join(target.filename());
    if target.archive == "zip" {
        crate::package::write_zip(&staging.0, &target.stem(), &archive)?;
    } else {
        crate::package::write_tar(&staging.0, &target.stem(), &archive)?;
    }
    let digest = crate::package::hash_file(&archive)?;
    fs::write(
        dist.join(format!("{}.sha256", target.filename())),
        format!("{digest}  {}\n", target.filename()),
    )?;
    fs::write(
        dist.join(target.evidence_filename()),
        serde_json::to_vec(&evidence_fixture(target, build, &digest))?,
    )?;
    Ok(digest)
}

fn evidence_fixture(target: &Target, build: &Value, digest: &str) -> Value {
    json!({
        "schema_version": 1,
        "evidence_class": EVIDENCE_CLASS,
        "archive": target.filename(),
        "archive_sha256": digest,
        "build": build,
        "host_target": target.target,
        "checks": smoke_commands(target).iter().map(|(program, arguments)| json!({
            "program": program, "arguments": arguments, "exit_code": 0,
            "stdout_sha256": "0".repeat(64)
        })).collect::<Vec<_>>()
    })
}

#[test]
fn release_requires_every_asset_and_matching_source_evidence() -> Result<()> {
    let manifest = Manifest::load(&crate::workspace_root()?)?;
    let directory = Directory::new()?;
    let root = directory.0.join("source");
    let dist = directory.0.join("dist");
    fixture_root(&root)?;
    fs::create_dir_all(root.join("release"))?;
    fs::copy(
        crate::workspace_root()?.join("release/targets.json"),
        root.join("release/targets.json"),
    )?;
    for target in &manifest.targets {
        let build = build_info(&manifest, target, Some(COMMIT), false);
        bundle_fixture(&root, &dist, target, &build)?;
    }
    let options = Options {
        directory: Some(dist.clone()),
        commit: Some(COMMIT.to_owned()),
        ..Options::default()
    };
    verify_release(&root, &options)?;
    let target = &manifest.targets[0];
    let evidence_path = dist.join(target.evidence_filename());
    let original = fs::read(&evidence_path)?;
    fs::remove_file(&evidence_path)?;
    assert!(
        verify_release(&root, &options)
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
    fs::write(&evidence_path, &original)?;
    fs::write(dist.join("unexpected.txt"), b"unexpected")?;
    assert!(
        verify_release(&root, &options)
            .unwrap_err()
            .to_string()
            .contains("unexpected")
    );
    fs::remove_file(dist.join("unexpected.txt"))?;
    let mut evidence: Value = serde_json::from_slice(&original)?;
    evidence["archive_sha256"] = json!("0".repeat(64));
    fs::write(&evidence_path, serde_json::to_vec(&evidence)?)?;
    assert!(
        verify_release(&root, &options)
            .unwrap_err()
            .to_string()
            .contains("evidence")
    );
    fs::write(&evidence_path, original)?;
    let other_commit = Options {
        commit: Some("f".repeat(40)),
        ..options
    };
    assert!(
        verify_release(&root, &other_commit)
            .unwrap_err()
            .to_string()
            .contains("metadata mismatch")
    );
    Ok(())
}

#[test]
fn inspection_rejects_tampering_dirty_sources_and_wrong_target() -> Result<()> {
    let manifest = Manifest::load(&crate::workspace_root()?)?;
    let directory = Directory::new()?;
    let root = directory.0.join("source");
    let dist = directory.0.join("dist");
    fixture_root(&root)?;
    let target = &manifest.targets[0];
    let expected = build_info(&manifest, target, Some(COMMIT), false);
    bundle_fixture(&root, &dist, target, &expected)?;
    let (unpacked, _) = inspect_package(&root, &dist, target, &expected)?;
    fs::write(unpacked.0.join("unexpected-runtime-file"), b"unexpected")?;
    assert!(verify_bundle_inventory(&root, &unpacked.0, target).is_err());
    fs::OpenOptions::new()
        .append(true)
        .open(dist.join(target.filename()))?
        .write_all(b"tamper")?;
    assert!(
        inspect_package(&root, &dist, target, &expected)
            .err()
            .unwrap()
            .to_string()
            .contains("checksum mismatch")
    );
    let dirty = build_info(&manifest, target, Some(COMMIT), true);
    bundle_fixture(&root, &dist, target, &dirty)?;
    assert!(inspect_package(&root, &dist, target, &expected).is_err());
    let wrong = build_info(&manifest, &manifest.targets[1], Some(COMMIT), false);
    bundle_fixture(&root, &dist, target, &wrong)?;
    assert!(inspect_package(&root, &dist, target, &expected).is_err());
    bundle_fixture(&root, &dist, target, &expected)?;
    fs::write(root.join("LICENSE"), b"changed license")?;
    assert!(inspect_package(&root, &dist, target, &expected).is_err());
    Ok(())
}

#[test]
fn evidence_requires_the_actual_native_command_contract() -> Result<()> {
    let manifest = Manifest::load(&crate::workspace_root()?)?;
    let target = &manifest.targets[0];
    let build = build_info(&manifest, target, Some(COMMIT), false);
    let digest = "a".repeat(64);
    let evidence = evidence_fixture(target, &build, &digest);
    validate_evidence(&evidence, target, &build, &digest)?;
    for key in ["host_target", "evidence_class"] {
        let mut invalid = evidence.clone();
        invalid[key] = json!("cross-compiled");
        assert!(validate_evidence(&invalid, target, &build, &digest).is_err());
    }
    let mut incomplete = evidence.clone();
    incomplete["checks"].as_array_mut().unwrap().pop();
    assert!(validate_evidence(&incomplete, target, &build, &digest).is_err());
    let mut failed = evidence;
    failed["checks"][0]["exit_code"] = json!(1);
    assert!(validate_evidence(&failed, target, &build, &digest).is_err());
    assert!(validate_smoke_output(0, b"wrong version").is_err());
    validate_smoke_output(0, format!("ArcForge {VERSION} (zip-browser)\n").as_bytes())?;
    assert!(validate_smoke_output(1, br#"{"kind":"commands","data":[]}"#).is_err());
    let command_rows: Vec<_> = [
        "version", "commands", "doctor", "list", "info", "preview", "create", "extract", "test", "update",
        "convert", "hash",
    ]
    .iter()
    .enumerate()
    .map(|(index, name)| {
        json!({
            "name": name, "availability": if index < 6 { "implemented" } else { "planned" }
        })
    })
    .collect();
    let mut commands = json!({"kind":"commands", "data": command_rows});
    validate_smoke_output(1, &serde_json::to_vec(&commands)?)?;
    commands["data"][6]["availability"] = json!("implemented");
    assert!(validate_smoke_output(1, &serde_json::to_vec(&commands)?).is_err());
    assert!(validate_smoke_output(2, br#"{"kind":"doctor","data":{}}"#).is_err());
    assert!(validate_smoke_output(4, br#"{"kind":"list","data":{"entries":[]}}"#).is_err());
    let mut preview = json!({"kind":"preview","data":{"preview":{
        "entry_id":1,"text":"Привет из ZIP.\n","crc32_verified":true
    }}});
    validate_smoke_output(5, &serde_json::to_vec(&preview)?)?;
    preview["data"]["preview"]["crc32_verified"] = json!(false);
    assert!(validate_smoke_output(5, &serde_json::to_vec(&preview)?).is_err());
    Ok(())
}

#[test]
fn unpack_rejects_zip64_locator_before_allocating_directory() -> Result<()> {
    let directory = Directory::new()?;
    let archive = directory.0.join("locator.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive)?);
    writer.start_file(
        "bundle/file",
        zip::write::SimpleFileOptions::default()
            .system(zip::System::Unix)
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o644),
    )?;
    writer.write_all(b"payload")?;
    writer.finish()?;
    let original = fs::read(&archive)?;
    let position = original
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .unwrap();
    let mut invalid = original[..position].to_vec();
    let mut locator = [0; 20];
    locator[..4].copy_from_slice(b"PK\x06\x07");
    locator[16..20].copy_from_slice(&1_u32.to_le_bytes());
    invalid.extend_from_slice(&locator);
    invalid.extend_from_slice(&original[position..]);
    fs::write(&archive, invalid)?;
    assert!(
        unpack::unpack(&archive, "zip", "bundle")
            .err()
            .unwrap()
            .to_string()
            .contains("ZIP directory")
    );
    Ok(())
}

fn zip_name_fixture(path: &Path, names: &[&str]) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(File::create(path)?);
    for name in names {
        writer.start_file(
            *name,
            zip::write::SimpleFileOptions::default()
                .system(zip::System::Unix)
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o644),
        )?;
        writer.write_all(b"payload")?;
    }
    writer.finish()?;
    Ok(fs::read(path)?)
}

#[test]
fn unpack_rejects_duplicate_raw_zip_names_hidden_by_index() -> Result<()> {
    let directory = Directory::new()?;
    let path = directory.0.join("duplicate.zip");
    let mut bytes = zip_name_fixture(&path, &["bundle/first", "bundle/other"])?;
    let positions: Vec<_> = bytes
        .windows(12)
        .enumerate()
        .filter_map(|(index, bytes)| (bytes == b"bundle/other").then_some(index))
        .collect();
    assert_eq!(positions.len(), 2, "one local and one central filename");
    for index in positions {
        bytes[index..index + 12].copy_from_slice(b"bundle/first");
    }
    fs::write(&path, bytes)?;
    assert!(unpack::unpack(&path, "zip", "bundle").is_err());
    Ok(())
}

#[test]
fn unpack_rejects_disagreement_between_local_and_central_zip_names() -> Result<()> {
    let directory = Directory::new()?;
    let path = directory.0.join("disagreement.zip");
    let mut bytes = zip_name_fixture(&path, &["bundle/first"])?;
    let local = bytes
        .windows(12)
        .position(|bytes| bytes == b"bundle/first")
        .unwrap();
    bytes[local..local + 12].copy_from_slice(b"bundle/other");
    fs::write(&path, bytes)?;
    assert!(unpack::unpack(&path, "zip", "bundle").is_err());
    Ok(())
}

#[test]
fn unpack_preserves_generated_gnu_long_names() -> Result<()> {
    let directory = Directory::new()?;
    let stage = directory.0.join("stage");
    fs::create_dir(&stage)?;
    let name = format!("{}.txt", "long".repeat(30));
    fs::write(stage.join(&name), b"long-name payload")?;
    let path = directory.0.join("long.tar.gz");
    crate::package::write_tar(&stage, "bundle", &path)?;
    let unpacked = unpack::unpack(&path, "tar.gz", "bundle")?;
    assert_eq!(fs::read(unpacked.0.join(name))?, b"long-name payload");
    Ok(())
}

#[test]
fn unpack_rejects_unexpected_pax_metadata() -> Result<()> {
    let directory = Directory::new()?;
    let path = directory.0.join("pax.tar.gz");
    let gzip = flate2::write::GzEncoder::new(File::create(&path)?, flate2::Compression::default());
    let mut writer = tar::Builder::new(gzip);
    let payload = b"16 comment=note\n";
    let mut extension = tar::Header::new_gnu();
    extension.set_path("bundle/metadata")?;
    extension.set_entry_type(tar::EntryType::XHeader);
    extension.set_size(payload.len() as u64);
    extension.set_mode(0o644);
    extension.set_cksum();
    writer.append(&extension, &payload[..])?;
    let mut file = tar::Header::new_gnu();
    file.set_path("bundle/file")?;
    file.set_size(0);
    file.set_mode(0o644);
    file.set_cksum();
    writer.append(&file, std::io::empty())?;
    writer.into_inner()?.finish()?;
    assert!(unpack::unpack(&path, "tar.gz", "bundle").is_err());
    Ok(())
}

#[test]
fn unpack_rejects_paths_links_and_case_collisions() -> Result<()> {
    let directory = Directory::new()?;
    let archive = directory.0.join("test.zip");
    for names in [
        vec!["bundle/../escaped"],
        vec!["/absolute"],
        vec!["bundle/C:/file"],
        vec!["bundle/a\\b"],
        vec!["bundle/NUL.txt"],
        vec!["bundle/name."],
        vec!["bundle/A", "bundle/a"],
    ] {
        let mut writer = zip::ZipWriter::new(File::create(&archive)?);
        for name in names {
            writer.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .system(zip::System::Unix)
                    .compression_method(zip::CompressionMethod::Stored)
                    .unix_permissions(0o644),
            )?;
            writer.write_all(b"payload")?;
        }
        writer.finish()?;
        assert!(unpack::unpack(&archive, "zip", "bundle").is_err());
    }
    let archive = directory.0.join("test.tar.gz");
    for entry_type in [tar::EntryType::Symlink, tar::EntryType::Link] {
        let gzip =
            flate2::write::GzEncoder::new(File::create(&archive)?, flate2::Compression::default());
        let mut writer = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_path("bundle/link")?;
        header.set_link_name("../../outside")?;
        header.set_entry_type(entry_type);
        header.set_size(0);
        header.set_mode(0o644);
        header.set_cksum();
        writer.append(&header, std::io::empty())?;
        writer.into_inner()?.finish()?;
        assert!(unpack::unpack(&archive, "tar.gz", "bundle").is_err());
    }
    assert!(!directory.0.join("escaped").exists());
    assert!(!directory.0.join("outside").exists());
    Ok(())
}
