//! Deterministic archive layout for unsigned development bundles.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::{Compression, GzBuilder};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

use crate::{Options, Result};

#[cfg(test)]
mod tests;

pub fn run(root: &Path, options: &Options) -> Result<()> {
    crate::notices::run(root)?;
    let manifest = crate::release::Manifest::load(root)?;
    let target = match &options.target {
        Some(target) => target.clone(),
        None => host_target(root)?,
    };
    let delivery = manifest.target(&target)?;
    crate::release::remove_if_present(&root.join("dist").join(delivery.evidence_filename()))?;
    let mut build = crate::cargo(root);
    build.args([
        "build",
        "--locked",
        "--release",
        "--target",
        &target,
        "-p",
        "arcforge",
        "-p",
        "arcforge-cli",
    ]);
    crate::run(&mut build)?;

    let metadata = crate::metadata(root, true)?;
    let target_dir = PathBuf::from(
        metadata["target_directory"]
            .as_str()
            .ok_or("metadata has no target_directory")?,
    );
    let version = crate::release::VERSION;
    let stem = delivery.stem();
    let staging = target_dir.join("arcforge-package").join(&stem);
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let binary_dir = target_dir.join(&target).join("release");
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    for name in ["arcforge", "arcforge-cli"] {
        let filename = format!("{name}{suffix}");
        let source = binary_dir.join(&filename);
        verify_binary(&source, &target)?;
        let destination = if name == "arcforge" {
            staging.join(&delivery.gui)
        } else {
            staging.join(&delivery.cli)
        };
        copy_file(&source, &destination)?;
    }
    if target.contains("apple") {
        let app = staging.join("ArcForge.app/Contents");
        fs::create_dir_all(app.join("Resources"))?;
        copy_file(
            &staging.join("arcforge-cli"),
            &app.join("MacOS/arcforge-cli"),
        )?;
        let numeric_version = version.split('-').next().ok_or("invalid version")?;
        fs::write(
            app.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>CFBundleIdentifier</key><string>org.stokaro.arcforge</string>\n<key>CFBundleName</key><string>ArcForge</string>\n<key>CFBundleDisplayName</key><string>ArcForge</string>\n<key>CFBundleExecutable</key><string>arcforge</string>\n<key>CFBundlePackageType</key><string>APPL</string>\n<key>CFBundleShortVersionString</key><string>{numeric_version}</string>\n<key>CFBundleVersion</key><string>{numeric_version}</string>\n<key>NSHighResolutionCapable</key><true/>\n</dict></plist>\n"
            ),
        )?;
    }
    for filename in ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "README.md"] {
        copy_file(&root.join(filename), &staging.join(filename))?;
    }
    copy_directory(&root.join("licenses"), &staging.join("licenses"))?;
    if root.join("docs").is_dir() {
        copy_directory(&root.join("docs"), &staging.join("docs"))?;
    }
    let dependencies: Vec<_> = metadata["packages"].as_array().ok_or("metadata has no packages")?.iter().map(|p| serde_json::json!({
        "name": p["name"], "version": p["version"], "source": p["source"],
        "license": p["license"], "license_file": p["license_file"], "repository": p["repository"]
    })).collect();
    fs::write(
        staging.join("DEPENDENCIES.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope": "Cargo lockfile inventory, including build/dev/tooling and other targets; declarations do not replace license texts",
            "target": target, "version": version, "packages": dependencies
        }))?,
    )?;
    fs::write(
        staging.join("DEVELOPMENT-BUNDLE.txt"),
        format!(
            "ArcForge {version}\nTarget: {target}\n\nUnsigned development bundle, not a production installer.\nNo file associations, shell extensions, signing or notarization are installed.\nThe application provides read-only ZIP browsing and small-entry previews; consult README.md for supported operations and limits.\n"
        ),
    )?;
    let (commit, dirty) = crate::release::source_state(root)?;
    fs::write(
        staging.join("BUILD-INFO.json"),
        serde_json::to_vec_pretty(&crate::release::build_info(
            &manifest,
            delivery,
            commit.as_deref(),
            dirty,
        ))?,
    )?;

    // Preserve notices when a macOS user moves only the application bundle.
    if target.contains("apple") {
        let resources = staging.join("ArcForge.app/Contents/Resources");
        for filename in [
            "LICENSE",
            "NOTICE",
            "THIRD_PARTY_NOTICES.md",
            "DEPENDENCIES.json",
            "BUILD-INFO.json",
        ] {
            copy_file(&staging.join(filename), &resources.join(filename))?;
        }
        copy_directory(&staging.join("licenses"), &resources.join("licenses"))?;
    }

    let dist = root.join("dist");
    fs::create_dir_all(&dist)?;
    let extension = delivery.archive.as_str();
    let archive = dist.join(delivery.filename());
    let temporary_archive = archive.with_extension(format!("{extension}.partial"));
    if extension == "tar.gz" {
        write_tar(&staging, &stem, &temporary_archive)?;
    } else {
        write_zip(&staging, &stem, &temporary_archive)?;
    }
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    fs::rename(&temporary_archive, &archive)?;
    let digest = hash_file(&archive)?;
    let filename = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid archive filename")?;
    fs::write(
        dist.join(format!("{filename}.sha256")),
        format!("{digest}  {filename}\n"),
    )?;
    println!("Created {}\nSHA-256 {digest}", archive.display());
    Ok(())
}

pub(crate) fn host_target(root: &Path) -> Result<String> {
    let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("-vV")
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err("rustc -vV failed".into());
    }
    String::from_utf8(output.stdout)?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| "rustc did not report its host target".into())
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(source).map_err(|error| format!("{}: {error}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!("packaged file must be a regular file: {}", source.display()).into());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    if !fs::symlink_metadata(source)?.is_dir() {
        return Err(format!("expected a real directory: {}", source.display()).into());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &destination.join(entry.file_name()))?;
        } else {
            copy_file(&entry.path(), &destination.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            result.extend(files(&entry.path())?);
        } else if entry.file_type()?.is_file() {
            result.push(entry.path());
        } else {
            return Err(format!("unsupported packaged entry: {}", entry.path().display()).into());
        }
    }
    result.sort();
    Ok(result)
}

fn mode(path: &Path) -> u32 {
    if matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("arcforge" | "arcforge-cli" | "arcforge.exe" | "arcforge-cli.exe")
    ) {
        0o755
    } else {
        0o644
    }
}

pub(crate) fn write_tar(staging: &Path, stem: &str, output: &Path) -> Result<()> {
    let gzip = GzBuilder::new()
        .mtime(0)
        .write(File::create(output)?, Compression::default());
    let mut archive = tar::Builder::new(gzip);
    for path in files(staging)? {
        let mut header = tar::Header::new_gnu();
        header.set_size(fs::metadata(&path)?.len());
        header.set_mode(mode(&path));
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        archive.append_data(
            &mut header,
            Path::new(stem).join(path.strip_prefix(staging)?),
            File::open(&path)?,
        )?;
    }
    archive.into_inner()?.finish()?.sync_all()?;
    Ok(())
}

pub(crate) fn write_zip(staging: &Path, stem: &str, output: &Path) -> Result<()> {
    let mut archive = zip::ZipWriter::new(File::create(output)?);
    for path in files(staging)? {
        let relative = path.strip_prefix(staging)?;
        let components = relative
            .iter()
            .map(|name| name.to_str().ok_or("non-UTF-8 packaged filename"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let name = format!("{stem}/{}", components.join("/"));
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            // Preserve Unix mode metadata on Windows too, as zip 2.x did.
            .system(zip::System::Unix)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(mode(&path))
            .large_file(fs::metadata(&path)?.len() >= u64::from(u32::MAX));
        archive.start_file(name, options)?;
        std::io::copy(&mut File::open(path)?, &mut archive)?;
    }
    archive.finish()?.sync_all()?;
    Ok(())
}

pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn verify_binary(path: &Path, target: &str) -> Result<()> {
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut header = [0_u8; 64];
    file.read_exact(&mut header)?;
    let arm = target.starts_with("aarch64");
    let matches = if target.contains("linux") {
        header.starts_with(b"\x7fELF")
            && header[4] == 2
            && header[5] == 1
            && u16::from_le_bytes([header[18], header[19]]) == if arm { 183 } else { 62 }
    } else if target.contains("apple") {
        header[..4] == [0xcf, 0xfa, 0xed, 0xfe]
            && u32::from_le_bytes([header[4], header[5], header[6], header[7]])
                == if arm { 0x0100_000c } else { 0x0100_0007 }
    } else {
        let offset = u32::from_le_bytes([header[60], header[61], header[62], header[63]]);
        file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut pe = [0_u8; 6];
        file.read_exact(&mut pe)?;
        header.starts_with(b"MZ")
            && pe.starts_with(b"PE\0\0")
            && u16::from_le_bytes([pe[4], pe[5]]) == if arm { 0xaa64 } else { 0x8664 }
    };
    if matches {
        Ok(())
    } else {
        Err(format!("{} is not a {target} executable", path.display()).into())
    }
}
