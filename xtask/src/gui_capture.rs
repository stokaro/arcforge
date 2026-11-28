//! Native GUI evidence, separate from release packaging and headless unit tests.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Options, Result};

const SCENES: &[(&str, Option<&str>)] = &[
    ("01-text-preview", Some("Documents/Guide.txt")),
    ("02-image-preview", Some("Photos/Color-study.png")),
    ("05-staged-delete", None),
    ("06-discard-prompt", None),
    #[cfg(target_os = "macos")]
    ("07-save-panel", None),
    ("08-save-collision", None),
    ("09-saved-copy", None),
    ("03-empty-window", None),
    #[cfg(target_os = "macos")]
    ("04-open-panel", None),
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
        "Captured native workspace and first-use scenes for {target}: {}",
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
    evidence["display_configuration"] = configure_mac_display(&mac_helper, directory);
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
            let scene_json = clients.join(format!("{scene}.json"));
            let observed = read_json(&scene_json)?;
            if observed["process_id"].as_u64() != Some(u64::from(child.0.id())) {
                return Err(
                    "scene provenance does not belong to the launched native process".into(),
                );
            }
            let mut recorded = json!({
                "scene": scene,
                "metadata": format!("client/{scene}.json"),
                "metadata_sha256": file_sha256(&scene_json)?,
            });
            if matches!(*scene, "04-open-panel" | "07-save-panel") {
                let panel_id = validate_file_panel(&observed, scene)?;
                let mut panel =
                    capture_mac_window(&mac_helper, directory, scene, child.0.id(), Some(panel_id));
                if panel["status"] != "captured"
                    || panel["window_id"].as_u64() != Some(u64::from(panel_id))
                    || panel["process_id"].as_u64() != Some(u64::from(child.0.id()))
                {
                    recorded["native_window"] = panel;
                    captured.push(recorded);
                    evidence["scenes"] = json!(captured);
                    return Err(format!("the actual native {scene} could not be captured; inspect its native-window logs").into());
                }
                panel["window_role"] = json!(if *scene == "04-open-panel" {
                    "open-panel"
                } else {
                    "save-panel"
                });
                recorded["native_window"] = panel;
            } else {
                let png = clients.join(format!("{scene}.png"));
                let dimensions = png_dimensions(&png)?;
                validate_scene(&observed, scene, *entry, dimensions)?;
                recorded["client_png"] = json!(format!("client/{scene}.png"));
                recorded["client_sha256"] = json!(file_sha256(&png)?);
                let window_id = observed["native_window_id"]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok());
                recorded["native_window"] =
                    capture_mac_window(&mac_helper, directory, scene, child.0.id(), window_id);
            }
            captured.push(recorded);
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
                return Err("native GUI exited before all required scenes were captured".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if file_sha256(&fixture)? != fixture_hash {
        return Err("native capture changed its source ZIP fixture".into());
    }
    evidence["fixture"]["unchanged_after_capture"] = json!(true);
    verify_saved_copy(root, directory, &fixture, evidence)?;
    evidence["binary_sha256_after_capture"] = json!(bounded_sha256(&binary, 1024 * 1024 * 1024)?);
    if evidence["binary_sha256"] != evidence["binary_sha256_after_capture"] {
        return Err("native executable changed during capture".into());
    }
    Ok(())
}

fn valid_source(value: &Value, scene: &str) -> bool {
    value["schema_version"] == 2
        && value["scene"] == scene
        && value["os"] == std::env::consts::OS
        && value["architecture"] == std::env::consts::ARCH
}

fn validate_scene(
    value: &Value,
    scene: &str,
    entry: Option<&str>,
    dimensions: [u32; 2],
) -> Result<()> {
    if !valid_source(value, scene)
        || value["capture_kind"] != "native-client-area"
        || value["native_window_chrome"] != false
        || value["renderer"] != "eframe Glow"
        || value["rendered_frames_after_ready"] != 2
        || value["pixel_dimensions"] != json!(dimensions)
        || !value["pixels_per_point"]
            .as_f64()
            .is_some_and(|scale| scale.is_finite() && scale > 0.0)
    {
        return Err(format!("invalid native scene provenance for {scene}").into());
    }
    if cfg!(target_os = "macos")
        && (value["native_shell_attached"] != true
            || value["native_window_id"]
                .as_u64()
                .is_none_or(|id| id == 0 || id > u64::from(u32::MAX)))
    {
        return Err("macOS capture did not use the required AppKit shell".into());
    }
    if let Some(entry) = entry {
        if value["archive_open"] != true
            || value["selected_entry_name"] != entry
            || value["preview_crc32_verified"] != true
            || value["snapshot_id"].as_str().is_none_or(str::is_empty)
            || value["metadata_revision"].as_str().is_none_or(|revision| {
                revision.len() != 64
                    || !revision
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err(format!("invalid archive preview provenance for {scene}").into());
        }
    } else if matches!(
        scene,
        "05-staged-delete" | "06-discard-prompt" | "08-save-collision" | "09-saved-copy"
    ) {
        validate_edit_state(value, scene)?;
    } else if scene != "03-empty-window"
        || value["archive_open"] != false
        || value["native_picker_pending"] != false
        || !value["snapshot_id"].is_null()
        || !value["selected_entry_name"].is_null()
    {
        return Err("empty scene retained archive, selection, or picker state".into());
    }
    if scene == "02-image-preview" && value["preview_image_size"] != json!([960, 640]) {
        return Err("image scene did not show the fixture's decoded 960×640 PNG".into());
    }
    Ok(())
}

fn validate_edit_state(value: &Value, scene: &str) -> Result<()> {
    let saved = scene == "09-saved-copy";
    let source = if saved {
        "workspace-demo-edited.zip"
    } else {
        "workspace-demo.zip"
    };
    let error = &value["workspace_error"];
    if value["archive_open"] != true
        || value["archive_name"] != source
        || value["pending_deletions"] != if saved { 0 } else { 1 }
        || value["visible_entry_count"] != 13
        || value["discard_prompt"] != (scene == "06-discard-prompt")
        || value["native_picker_pending"] != (scene == "07-save-panel")
        || value["snapshot_id"].as_str().is_none_or(str::is_empty)
        || !value["selected_entry_name"].is_null()
        || if scene == "08-save-collision" {
            error
                .as_str()
                .is_none_or(|message| !message.contains("already exists"))
        } else {
            !error.is_null()
        }
    {
        return Err(format!("invalid staged-edit provenance for {scene}").into());
    }
    Ok(())
}

fn validate_file_panel(value: &Value, scene: &str) -> Result<u32> {
    let opening = scene == "04-open-panel";
    let class = if opening {
        "NSOpenPanel"
    } else {
        "NSSavePanel"
    };
    let panel = &value["native_panel"];
    let window = panel["window_id"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok());
    let parent = panel["parent_window_id"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok());
    if !valid_source(value, scene)
        || value["os"] != "macos"
        || value["capture_kind"] != "native-panel-request"
        || value["archive_open"].as_bool() != Some(!opening)
        || value["native_picker_pending"] != true
        || value["native_shell_attached"] != true
        || value["native_window_id"].as_u64() != parent.map(u64::from)
        || value["rendered_frames_after_ready"] != 0
        || !value["pixel_dimensions"].is_null()
        || (opening && !value["snapshot_id"].is_null())
        || panel["class"] != class
        || window.is_none_or(|id| id == 0)
        || parent.is_none_or(|id| id == 0)
        || window == parent
    {
        return Err(
            format!("native {scene} readiness did not identify an actual owned {class}").into(),
        );
    }
    if !opening {
        validate_edit_state(value, scene)?;
    }
    Ok(window.expect("nonzero window ID validated"))
}

fn verify_saved_copy(
    root: &Path,
    directory: &Path,
    fixture: &Path,
    evidence: &mut Value,
) -> Result<()> {
    let saved = directory.join("workspace-demo-edited.zip");
    let mut verify = Command::new(if cfg!(windows) { "python" } else { "python3" });
    verify
        .current_dir(root)
        .arg("-c")
        .arg(
            r#"
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as source, zipfile.ZipFile(sys.argv[2]) as saved:
    removed = 'Photos/Color-study.png'
    expected = [name for name in source.namelist() if name != removed]
    if len(source.namelist()) != len(expected) + 1 or saved.namelist() != expected:
        raise SystemExit('saved copy differs from the expected deletion')
    if saved.comment != source.comment or saved.testzip() is not None:
        raise SystemExit('saved copy failed archive comment or CRC verification')
    if any(saved.read(name) != source.read(name) for name in expected):
        raise SystemExit('saved copy modified a retained payload')
"#,
        )
        .arg(fixture)
        .arg(&saved);
    crate::run(&mut verify)?;
    evidence["saved_copy"] = json!({
        "file": "workspace-demo-edited.zip",
        "sha256": file_sha256(&saved)?,
        "removed_entry": "Photos/Color-study.png",
        "verified_independently_with": "Python zipfile",
    });
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
        "ARCFORGE_CAPTURE_GL_DRIVER",
        "ARCFORGE_CAPTURE_GL_SOURCE_COMMIT",
        "ARCFORGE_CAPTURE_GL_PACKAGE_SHA256",
        "ARCFORGE_CAPTURE_GL_LIBRARY_SHA256",
        "ARCFORGE_CAPTURE_GL_LIBRARY_PATH",
        "ARCFORGE_CAPTURE_GL_LICENSES",
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

fn configure_mac_display(helper: &Result<PathBuf>, directory: &Path) -> Value {
    if !cfg!(target_os = "macos") || std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return json!({"status": "not-requested", "reason": "desktop mode selection is limited to macOS CI"});
    }
    let binary = match helper {
        Ok(binary) => binary,
        Err(error) => return json!({"status": "unavailable", "reason": error.to_string()}),
    };
    let mut command = Command::new(binary);
    command.arg("--fit-desktop");
    let mut report = match logged_command(
        &mut command,
        directory,
        "mac-display",
        Duration::from_secs(10),
    )
    .and_then(|()| read_json(&directory.join("mac-display.stdout.log")))
    {
        Ok(report) => report,
        Err(error) => json!({"status": "unavailable", "reason": error.to_string()}),
    };
    // The setter process has exited. A new observer catches process-scoped
    // changes that revert before ArcForge can create its native window.
    let mut observer = Command::new(binary);
    observer.arg("--describe-desktop");
    let observed = match logged_command(
        &mut observer,
        directory,
        "mac-display-after-helper-exit",
        Duration::from_secs(10),
    )
    .and_then(|()| read_json(&directory.join("mac-display-after-helper-exit.stdout.log")))
    {
        Ok(observed) => observed,
        Err(error) => json!({"status": "unavailable", "reason": error.to_string()}),
    };
    record_display_after_helper_exit(&mut report, observed);
    report
}

fn record_display_after_helper_exit(report: &mut Value, observed: Value) {
    let claimed_ready = matches!(
        report["status"].as_str(),
        Some("changed" | "already-sufficient")
    );
    let expected_mode = report["after"]["mode_id"].as_u64();
    let expected_display = report["display_id"].as_u64();
    let persists = claimed_ready
        && observed["status"] == "observed"
        && expected_mode.is_some()
        && observed["mode"]["mode_id"].as_u64() == expected_mode
        && expected_display.is_some()
        && observed["display_id"].as_u64() == expected_display
        && observed["mode"]["width"]
            .as_u64()
            .is_some_and(|width| width >= 1440)
        && observed["mode"]["height"]
            .as_u64()
            .is_some_and(|height| height >= 900)
        && observed["bounds"][2]
            .as_f64()
            .is_some_and(|width| width >= 1440.0)
        && observed["bounds"][3]
            .as_f64()
            .is_some_and(|height| height >= 900.0);
    report["after_helper_exit"] = observed;
    report["persists_after_helper_exit"] = json!(persists);
    if claimed_ready && !persists {
        report["status"] = json!("unavailable");
        report["reason"] = json!(
            "larger desktop mode was not observed by a separate process after the configuration helper exited"
        );
    }
}

fn capture_mac_window(
    helper: &Result<PathBuf>,
    directory: &Path,
    scene: &str,
    pid: u32,
    window_id: Option<u32>,
) -> Value {
    let binary = match helper {
        Ok(binary) => binary,
        Err(error) => return json!({"status": "unavailable", "reason": error.to_string()}),
    };
    let stem = format!("{scene}-native-window");
    let png = directory.join(format!("{stem}.png"));
    let mut command = Command::new(binary);
    command.arg(pid.to_string()).arg(&png);
    if let Some(window_id) = window_id {
        command.arg(window_id.to_string());
    }
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
            "schema_version": 2, "scene": "02-image-preview", "archive_open": true,
            "selected_entry_name": "Photos/Color-study.png",
            "capture_kind": "native-client-area", "native_window_chrome": false,
            "renderer": "eframe Glow", "preview_crc32_verified": true,
            "rendered_frames_after_ready": 2, "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH, "pixel_dimensions": [1280, 800],
            "pixels_per_point": 1.0, "snapshot_id": "opened-real-fixture",
            "metadata_revision": "ab".repeat(32), "preview_image_size": [960, 640],
            "native_shell_attached": cfg!(target_os = "macos"),
            "native_window_id": if cfg!(target_os = "macos") { Some(12) } else { None },
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

    #[test]
    fn display_mode_must_survive_the_configuration_helper_process() {
        let configured = json!({
            "status": "changed", "display_id": 1,
            "after": {"mode_id": 9, "width": 1600, "height": 1200},
            "after_bounds": [0, 0, 1600, 1200],
        });
        let observed = json!({
            "status": "observed", "display_id": 1,
            "mode": {"mode_id": 9, "width": 1600, "height": 1200},
            "bounds": [0, 0, 1600, 1200],
        });
        let mut retained = configured.clone();
        record_display_after_helper_exit(&mut retained, observed.clone());
        assert_eq!(retained["status"], "changed");
        assert_eq!(retained["persists_after_helper_exit"], true);
        assert_eq!(retained["after_helper_exit"], observed);

        let mut reverted = observed.clone();
        reverted["mode"] = json!({"mode_id": 3, "width": 1024, "height": 768});
        reverted["bounds"] = json!([0, 0, 1024, 768]);
        let mut stale_bounds = observed;
        stale_bounds["bounds"] = json!([0, 0, 1024, 768]);
        for observation in [reverted, stale_bounds, json!({"status": "unavailable"})] {
            let mut report = configured.clone();
            record_display_after_helper_exit(&mut report, observation.clone());
            assert_eq!(report["status"], "unavailable");
            assert_eq!(report["persists_after_helper_exit"], false);
            assert_eq!(report["after_helper_exit"], observation);
        }
    }

    #[test]
    fn empty_capture_rejects_a_retained_document_selection_or_picker() {
        let mut empty = scene();
        empty["scene"] = json!("03-empty-window");
        empty["archive_open"] = json!(false);
        empty["native_picker_pending"] = json!(false);
        empty.as_object_mut().unwrap().remove("snapshot_id");
        empty.as_object_mut().unwrap().remove("selected_entry_name");
        assert!(validate_scene(&empty, "03-empty-window", None, [1280, 800]).is_ok());
        for (key, value) in [
            ("archive_open", json!(true)),
            ("native_picker_pending", json!(true)),
            ("snapshot_id", json!("a-retained-document")),
            ("selected_entry_name", json!("Documents/Guide.txt")),
        ] {
            let mut invalid = empty.clone();
            invalid[key] = value;
            assert!(validate_scene(&invalid, "03-empty-window", None, [1280, 800]).is_err());
        }
    }

    #[test]
    fn a_client_frame_cannot_establish_a_native_open_panel() {
        assert!(validate_file_panel(&scene(), "04-open-panel").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_open_readiness_requires_distinct_real_window_identities() {
        let ready = json!({
            "schema_version": 2, "scene": "04-open-panel",
            "os": "macos", "architecture": std::env::consts::ARCH,
            "capture_kind": "native-panel-request", "archive_open": false,
            "native_picker_pending": true, "rendered_frames_after_ready": 0,
            "native_shell_attached": true, "native_window_id": 12,
            "native_panel": {"class": "NSOpenPanel", "window_id": 14, "parent_window_id": 12},
        });
        assert_eq!(validate_file_panel(&ready, "04-open-panel").unwrap(), 14);
        for value in [json!(0), json!(12), json!(u64::MAX), json!(null)] {
            let mut invalid = ready.clone();
            invalid["native_panel"]["window_id"] = value;
            assert!(validate_file_panel(&invalid, "04-open-panel").is_err());
        }
        for (key, value) in [
            ("native_window_id", json!(13)),
            ("native_shell_attached", json!(false)),
        ] {
            let mut invalid = ready.clone();
            invalid[key] = value;
            assert!(validate_file_panel(&invalid, "04-open-panel").is_err());
        }
        let mut wrong_parent = ready.clone();
        wrong_parent["native_panel"]["parent_window_id"] = json!(13);
        assert!(validate_file_panel(&wrong_parent, "04-open-panel").is_err());
        let mut reconstructed = ready;
        reconstructed["pixel_dimensions"] = json!([1280, 800]);
        assert!(validate_file_panel(&reconstructed, "04-open-panel").is_err());
    }
}
