//! Native GUI evidence, separate from release packaging and headless unit tests.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Options, Result};

const SCENES: [(&str, &str); 2] = [
    ("01-text-preview", "Documents/Guide.txt"),
    ("02-image-preview", "Photos/Color-study.png"),
];
const PROCESS_DEADLINE: Duration = Duration::from_secs(150);
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn run(root: &Path, options: &Options) -> Result<()> {
    let host = crate::package::host_target(root)?;
    let target = options.target.as_deref().unwrap_or(&host);
    crate::release::Manifest::load(root)?.target(target)?;
    if target != host {
        return Err(
            format!("gui-capture requires the native host {host}, received {target}").into(),
        );
    }
    let metadata = crate::metadata(root, false)?;
    let target_directory = PathBuf::from(
        metadata["target_directory"]
            .as_str()
            .ok_or("Cargo metadata lacks target_directory")?,
    );
    let directory = options.directory.as_ref().map_or_else(
        || target_directory.join("native-gui").join(target),
        |path| root.join(path),
    );
    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&directory).map_err(|error| {
        format!(
            "native capture requires a new output directory {}: {error}",
            directory.display()
        )
    })?;
    let source = crate::release::source_state(root)?;
    let mut evidence = json!({
        "schema_version": 1,
        "status": "preparing",
        "target": target,
        "host_target": host,
        "source_commit": source.0,
        "source_dirty": source.1,
        "build": {
            "command": ["cargo", "build", "--locked", "--target", target, "-p", "arcforge", "--features", "native-capture"],
            "profile": "debug",
            "feature": "native-capture",
            "release_bundle": false,
        },
        "runtime": runtime_provenance(),
        "scenes": [],
    });
    write_json(&directory.join("capture-run.json"), &evidence)?;
    let result = require_clean_source(&source)
        .and_then(|()| capture(root, target, &target_directory, &directory, &mut evidence));
    let source_after = crate::release::source_state(root);
    let result = result.and_then(|()| {
        if source_after? != source {
            Err("source revision or worktree cleanliness changed during native capture".into())
        } else {
            Ok(())
        }
    });
    match &result {
        Ok(()) => evidence["status"] = json!("completed"),
        Err(error) => {
            evidence["status"] = json!("failed");
            evidence["error"] = json!(error.to_string());
        }
    }
    write_json(&directory.join("capture-run.json"), &evidence)?;
    result?;
    println!(
        "Captured two real native client scenes for {target}: {}",
        directory.display()
    );
    Ok(())
}

fn capture(
    root: &Path,
    target: &str,
    target_directory: &Path,
    directory: &Path,
    evidence: &mut Value,
) -> Result<()> {
    let mut build = crate::cargo(root);
    build.args([
        "build",
        "--locked",
        "--target",
        target,
        "-p",
        "arcforge",
        "--features",
        "native-capture",
    ]);
    crate::run(&mut build)?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let binary = target_directory
        .join(target)
        .join("debug")
        .join(format!("arcforge{suffix}"));
    crate::package::verify_binary(&binary, target)?;
    evidence["binary_sha256"] = json!(bounded_sha256(&binary, 1024 * 1024 * 1024)?);

    let fixture = directory.join("workspace-demo.zip");
    let mut generate = Command::new(if cfg!(windows) { "python" } else { "python3" });
    generate
        .current_dir(root)
        .arg(root.join("scripts/generate_ui_fixture.py"))
        .arg(&fixture)
        .arg("--images");
    crate::run(&mut generate)?;
    let fixture_hash = file_sha256(&fixture)?;
    evidence["fixture"] = json!({
        "file": "workspace-demo.zip",
        "generator": "scripts/generate_ui_fixture.py --images",
        "generator_sha256": file_sha256(&root.join("scripts/generate_ui_fixture.py"))?,
        "sha256": fixture_hash,
        "original_content": true,
    });
    let mac_helper = prepare_mac_helper(root, directory);
    let clients = directory.join("client");
    let stdout = directory.join("app.stdout.log");
    let stderr = directory.join("app.stderr.log");
    let mut command = Command::new(&binary);
    command
        .current_dir(root)
        .arg("--capture-native")
        .arg(&clients)
        .arg("--")
        .arg(&fixture)
        .stdin(Stdio::null())
        .stdout(File::create(&stdout)?)
        .stderr(File::create(&stderr)?);
    #[cfg(target_os = "linux")]
    command.env_remove("WAYLAND_DISPLAY");
    let mut child = ChildGuard(command.spawn()?);
    evidence["process_id"] = json!(child.0.id());
    evidence["status"] = json!("capturing");
    write_json(&directory.join("capture-run.json"), evidence)?;
    let mut captured = Vec::new();
    let started = Instant::now();
    loop {
        if started.elapsed() > PROCESS_DEADLINE {
            return Err(
                "native GUI did not finish within 150 seconds; inspect app.stderr.log".into(),
            );
        }
        for path in [&stdout, &stderr] {
            if fs::metadata(path)?.len() > MAX_FILE_BYTES {
                return Err("native GUI log exceeded 64 MiB".into());
            }
        }
        if let Some((scene, entry)) = SCENES.get(captured.len())
            && clients.join(format!("{scene}.ready")).try_exists()?
        {
            let png = clients.join(format!("{scene}.png"));
            let scene_json = clients.join(format!("{scene}.json"));
            let observed = read_json(&scene_json)?;
            let dimensions = png_dimensions(&png)?;
            validate_scene(&observed, scene, entry, dimensions)?;
            if observed["process_id"].as_u64() != Some(u64::from(child.0.id())) {
                return Err(
                    "scene provenance does not belong to the launched native process".into(),
                );
            }
            let chrome = capture_mac_window(&mac_helper, directory, scene, child.0.id());
            captured.push(json!({
                "scene": scene,
                "client_png": format!("client/{scene}.png"),
                "client_sha256": file_sha256(&png)?,
                "metadata": format!("client/{scene}.json"),
                "metadata_sha256": file_sha256(&scene_json)?,
                "native_window": chrome,
            }));
            // The application advances only after the exact scene's native
            // window capture attempt has finished and its outcome is recorded.
            evidence["scenes"] = json!(captured);
            write_json(&directory.join("capture-run.json"), evidence)?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(clients.join(format!("{scene}.continue")))?;
        }
        if let Some(status) = child.0.try_wait()? {
            if !status.success() {
                return Err(
                    format!("native GUI exited with {status}; inspect app.stderr.log").into(),
                );
            }
            if captured.len() != SCENES.len() {
                return Err("native GUI exited before both validated scenes were captured".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if file_sha256(&fixture)? != fixture_hash {
        return Err("native capture changed its source ZIP fixture".into());
    }
    evidence["fixture"]["unchanged_after_capture"] = json!(true);
    evidence["binary_sha256_after_capture"] = json!(bounded_sha256(&binary, 1024 * 1024 * 1024)?);
    if evidence["binary_sha256"] != evidence["binary_sha256_after_capture"] {
        return Err("native executable changed during capture".into());
    }
    Ok(())
}

fn validate_scene(value: &Value, scene: &str, entry: &str, dimensions: [u32; 2]) -> Result<()> {
    if value["schema_version"] != 1
        || value["scene"] != scene
        || value["selected_entry_name"] != entry
        || value["capture_kind"] != "native-client-area"
        || value["native_window_chrome"] != false
        || value["renderer"] != "eframe Glow"
        || value["preview_crc32_verified"] != true
        || value["rendered_frames_after_ready"] != 2
        || value["os"] != std::env::consts::OS
        || value["architecture"] != std::env::consts::ARCH
        || value["pixel_dimensions"] != json!(dimensions)
        || !value["pixels_per_point"]
            .as_f64()
            .is_some_and(|scale| scale.is_finite() && scale > 0.0)
        || value["snapshot_id"].as_str().is_none_or(str::is_empty)
        || value["metadata_revision"].as_str().is_none_or(|revision| {
            revision.len() != 64
                || !revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(format!("invalid native scene provenance for {scene}").into());
    }
    if scene == "02-image-preview" && value["preview_image_size"] != json!([960, 640]) {
        return Err("image scene did not show the fixture's decoded 960×640 PNG".into());
    }
    Ok(())
}

fn runtime_provenance() -> Value {
    let environment: serde_json::Map<String, Value> = [
        "GITHUB_ACTIONS",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "GITHUB_SHA",
        "RUNNER_OS",
        "RUNNER_ARCH",
        "ImageOS",
        "ImageVersion",
        "ARCFORGE_CAPTURE_DISPLAY",
        "DISPLAY",
        "LIBGL_ALWAYS_SOFTWARE",
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| (key.into(), json!(value)))
    })
    .collect();
    let os_command = if cfg!(target_os = "macos") {
        ("sw_vers", Vec::<&str>::new())
    } else if cfg!(windows) {
        ("cmd.exe", vec!["/C", "ver"])
    } else {
        ("uname", vec!["-a"])
    };
    let os_version = Command::new(os_command.0)
        .args(os_command.1)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    json!({
        "os": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "os_version": os_version,
        "environment": environment,
        "window": "actual native eframe window with shared platform::native_options",
        "renderer": "production Glow backend; no offscreen or alternate renderer",
    })
}

fn prepare_mac_helper(root: &Path, directory: &Path) -> Result<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Err("OS window capture is implemented only for macOS; native client pixels are captured on every OS".into());
    }
    let binary = directory.join("capture-macos-window");
    let mut command = Command::new("xcrun");
    command
        .arg("swiftc")
        .arg(root.join("scripts/capture_macos_window.swift"))
        .arg("-o")
        .arg(&binary);
    logged_command(
        &mut command,
        directory,
        "mac-window-helper-build",
        Duration::from_secs(60),
    )?;
    Ok(binary)
}

fn capture_mac_window(helper: &Result<PathBuf>, directory: &Path, scene: &str, pid: u32) -> Value {
    let binary = match helper {
        Ok(binary) => binary,
        Err(error) => return json!({"status": "unavailable", "reason": error.to_string()}),
    };
    let stem = format!("{scene}-native-window");
    let png = directory.join(format!("{stem}.png"));
    let mut command = Command::new(binary);
    command.arg(pid.to_string()).arg(&png);
    let result = logged_command(&mut command, directory, &stem, Duration::from_secs(15))
        .and_then(|()| read_json(&directory.join(format!("{stem}.stdout.log"))));
    match result {
        Ok(mut report) if report["status"] == "captured" => {
            match png_dimensions(&png).and_then(|dimensions| Ok((dimensions, file_sha256(&png)?))) {
                Ok((dimensions, hash)) => {
                    report["png"] = json!(format!("{stem}.png"));
                    report["sha256"] = json!(hash);
                    report["pixel_dimensions"] = json!(dimensions);
                    report
                }
                Err(error) => json!({"status": "unavailable", "reason": error.to_string()}),
            }
        }
        Ok(report) => report,
        Err(error) => json!({"status": "unavailable", "reason": error.to_string()}),
    }
}

fn logged_command(
    command: &mut Command,
    directory: &Path,
    stem: &str,
    timeout: Duration,
) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(File::create(directory.join(format!("{stem}.stdout.log")))?)
        .stderr(File::create(directory.join(format!("{stem}.stderr.log")))?);
    let mut child = ChildGuard(command.spawn()?);
    let started = Instant::now();
    let status: ExitStatus = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            return Err(format!("{stem} exceeded {} seconds", timeout.as_secs()).into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if status.success() {
        Ok(())
    } else {
        Err(format!("{stem} failed with {status}").into())
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn file_sha256(path: &Path) -> Result<String> {
    bounded_sha256(path, MAX_FILE_BYTES)
}

fn bounded_sha256(path: &Path, limit: u64) -> Result<String> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(format!(
            "{} exceeds the {limit}-byte capture hashing limit",
            path.display()
        )
        .into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > limit {
            return Err("file grew beyond its capture hashing limit".into());
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn require_clean_source(source: &(Option<String>, bool)) -> Result<()> {
    if source.0.is_none() || source.1 {
        return Err("gui-capture requires a clean Git worktree at an actual commit; checkpoint the source before recording native evidence".into());
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    let file = File::open(path)?;
    if file.metadata()?.len() > 1024 * 1024 {
        return Err("capture JSON exceeded 1 MiB".into());
    }
    Ok(serde_json::from_reader(file.take(1024 * 1024 + 1))?)
}

fn png_dimensions(path: &Path) -> Result<[u32; 2]> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES {
        return Err("native PNG exceeded 64 MiB".into());
    }
    let mut header = [0; 24];
    file.read_exact(&mut header)?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n"
        || header[8..12] != 13u32.to_be_bytes()
        || &header[12..16] != b"IHDR"
    {
        return Err("native screenshot is not a PNG with an IHDR header".into());
    }
    let dimensions = [
        u32::from_be_bytes(header[16..20].try_into()?),
        u32::from_be_bytes(header[20..24].try_into()?),
    ];
    if dimensions[0] < 320
        || dimensions[1] < 240
        || u64::from(dimensions[0]) * u64::from(dimensions[1]) > 16 * 1024 * 1024
    {
        return Err("native screenshot has invalid or excessive dimensions".into());
    }
    Ok(dimensions)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut file = File::create(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Value {
        json!({
            "schema_version": 1, "scene": "02-image-preview",
            "selected_entry_name": "Photos/Color-study.png",
            "capture_kind": "native-client-area", "native_window_chrome": false,
            "renderer": "eframe Glow", "preview_crc32_verified": true,
            "rendered_frames_after_ready": 2, "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH, "pixel_dimensions": [1280, 800],
            "pixels_per_point": 1.0, "snapshot_id": "opened-real-fixture",
            "metadata_revision": "ab".repeat(32), "preview_image_size": [960, 640],
        })
    }

    #[test]
    fn rejects_wrong_platform_unchecked_preview_or_missing_real_image() {
        let valid = scene();
        assert!(validate_scene(&valid, SCENES[1].0, SCENES[1].1, [1280, 800]).is_ok());
        for (key, value) in [
            ("os", json!("made-up-os")),
            ("preview_crc32_verified", json!(false)),
            ("preview_image_size", json!(null)),
            ("rendered_frames_after_ready", json!(0)),
            ("native_window_chrome", json!(true)),
            ("pixel_dimensions", json!([100, 100])),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(
                validate_scene(&invalid, SCENES[1].0, SCENES[1].1, [1280, 800]).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn native_evidence_requires_a_clean_committed_source() {
        assert!(require_clean_source(&(Some("a".repeat(40)), false)).is_ok());
        assert!(require_clean_source(&(Some("a".repeat(40)), true)).is_err());
        assert!(require_clean_source(&(None, false)).is_err());
    }
}
