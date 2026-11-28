//! Developer-only capture of a real native eframe window.
//!
//! The public screenshot callback reads the window's rendered client framebuffer.
//! It does not include OS decorations. `xtask gui-capture` owns the fixture,
//! process deadline, source provenance and optional macOS window capture.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arcforge_ui_egui::native_capture::{ReadyScene, Scene};
use eframe::egui;
use image::ImageEncoder;
use serde_json::{Value, json};

const DEADLINE: Duration = Duration::from_secs(120);
const ACK_DEADLINE: Duration = Duration::from_secs(45);
const MAX_PIXELS: usize = 16 * 1024 * 1024;

type Outcome = Arc<Mutex<Option<Result<(), String>>>>;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Options {
    pub directory: PathBuf,
    pub archive: PathBuf,
}

pub(crate) fn run(options: Options) -> ExitCode {
    let outcome = Outcome::default();
    let app_outcome = outcome.clone();
    if let Err(error) = fs::create_dir(&options.directory) {
        eprintln!("arcforge capture: create a new output directory: {error}");
        return ExitCode::FAILURE;
    }
    arcforge_ui_egui::typography::prepare();
    let result = eframe::run_native(
        "ArcForge",
        super::platform::native_options(),
        Box::new(move |context| {
            let capture = Session::new(options.directory, context.egui_ctx.clone(), app_outcome)?;
            Ok(Box::new(super::DesktopApp {
                workspace: arcforge_ui_egui::ArchiveWorkspace::new(Some(options.archive)),
                capture: Some(capture),
            }))
        }),
    );
    if let Err(error) = result {
        eprintln!("arcforge capture: could not run the native window: {error}");
        return ExitCode::FAILURE;
    }
    match outcome.lock().ok().and_then(|mut result| result.take()) {
        Some(Ok(())) => ExitCode::SUCCESS,
        Some(Err(error)) => {
            eprintln!("arcforge capture: {error}");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("arcforge capture: window closed before both scenes completed");
            ExitCode::FAILURE
        }
    }
}

enum Event {
    Painted,
    Saved,
    Failed(String),
}

struct Shot {
    scene: Scene,
    image: Arc<egui::ColorImage>,
    metadata: Value,
}

enum Phase {
    Preparing,
    AwaitingPaint(ReadyScene),
    Painted(ReadyScene),
    Saving,
    Finished,
}

pub(crate) struct Session {
    started: Instant,
    scene_index: usize,
    phase: Phase,
    events: Receiver<Event>,
    event_sender: SyncSender<Event>,
    writer: SyncSender<Shot>,
    outcome: Outcome,
}

impl Session {
    fn new(directory: PathBuf, context: egui::Context, outcome: Outcome) -> std::io::Result<Self> {
        let (writer, shots) = mpsc::sync_channel::<Shot>(1);
        let (event_sender, events) = mpsc::sync_channel(4);
        let sender = event_sender.clone();
        let thread = std::thread::Builder::new()
            .name("arcforge-capture-writer".into())
            .spawn(move || {
                while let Ok(shot) = shots.recv() {
                    let event = match save_shot(&directory, shot) {
                        Ok(()) => Event::Saved,
                        Err(error) => Event::Failed(error),
                    };
                    if sender.send(event).is_err() {
                        break;
                    }
                    context.request_repaint();
                }
            })?;
        // The writer has bounded work and exits when the session closes its channel.
        drop(thread);
        Ok(Self {
            started: Instant::now(),
            scene_index: 0,
            phase: Phase::Preparing,
            events,
            event_sender,
            writer,
            outcome,
        })
    }

    pub(crate) fn after_frame(
        &mut self,
        workspace: &mut arcforge_ui_egui::ArchiveWorkspace,
        context: &egui::Context,
    ) {
        if matches!(self.phase, Phase::Finished) {
            return;
        }
        if self.started.elapsed() > DEADLINE {
            self.finish(Err("native capture exceeded 120 seconds".into()), context);
            return;
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Failed(error) => {
                    self.finish(Err(error), context);
                    return;
                }
                Event::Painted => {
                    let Phase::AwaitingPaint(ready) =
                        std::mem::replace(&mut self.phase, Phase::Preparing)
                    else {
                        self.finish(
                            Err("unexpected native paint acknowledgement".into()),
                            context,
                        );
                        return;
                    };
                    self.phase = Phase::Painted(ready);
                }
                Event::Saved => {
                    self.scene_index += 1;
                    if self.scene_index == Scene::ALL.len() {
                        self.finish(Ok(()), context);
                        return;
                    }
                    self.phase = Phase::Preparing;
                }
            }
        }
        if matches!(self.phase, Phase::AwaitingPaint(_) | Phase::Saving) {
            context.request_repaint_after(Duration::from_millis(50));
            return;
        }
        let scene = Scene::ALL[self.scene_index];
        let ready = match workspace.prepare_native_capture(scene, context) {
            Ok(Some(ready)) => ready,
            Ok(None) => {
                context.request_repaint_after(Duration::from_millis(25));
                return;
            }
            Err(error) => {
                self.finish(Err(error), context);
                return;
            }
        };
        let visible = context.input(|input| {
            let viewport = input.viewport();
            viewport.minimized != Some(true) && viewport.occluded != Some(true)
        });
        if !visible {
            self.finish(
                Err("native capture window became minimized or occluded".into()),
                context,
            );
            return;
        }
        match &self.phase {
            Phase::Preparing => {
                self.phase = Phase::AwaitingPaint(ready);
                let sender = self.event_sender.clone();
                let repaint = context.clone();
                context.request_screenshot(move |_pixels| {
                    let _ = sender.try_send(Event::Painted);
                    repaint.request_repaint();
                });
                context.request_repaint();
            }
            Phase::Painted(previous) if previous == &ready => {
                let metadata = scene_metadata(scene, &ready, context);
                self.phase = Phase::Saving;
                let writer = self.writer.clone();
                let sender = self.event_sender.clone();
                let repaint = context.clone();
                context.request_screenshot(move |image| {
                    if let Err(error) = writer.try_send(Shot {
                        scene,
                        image,
                        metadata,
                    }) {
                        let _ = sender.try_send(Event::Failed(format!(
                            "capture writer did not accept rendered pixels: {error}"
                        )));
                    }
                    repaint.request_repaint();
                });
                context.request_repaint();
            }
            Phase::Painted(_) => self.finish(
                Err("workspace state changed between rendered capture frames".into()),
                context,
            ),
            _ => {}
        }
    }

    fn finish(&mut self, result: Result<(), String>, context: &egui::Context) {
        if let Ok(mut outcome) = self.outcome.lock() {
            *outcome = Some(result);
        }
        self.phase = Phase::Finished;
        context.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn scene_metadata(scene: Scene, ready: &ReadyScene, context: &egui::Context) -> Value {
    let viewport = context.input(|input| {
        let viewport = input.viewport();
        let rectangle = |rect: Option<egui::Rect>| {
            rect.map(|rect| [rect.min.x, rect.min.y, rect.width(), rect.height()])
        };
        json!({
            "native_pixels_per_point": viewport.native_pixels_per_point,
            "inner_rect_points": rectangle(viewport.inner_rect),
            "outer_rect_points": rectangle(viewport.outer_rect),
            "focused": viewport.focused,
            "minimized": viewport.minimized,
            "occluded": viewport.occluded,
        })
    });
    let fonts = arcforge_ui_egui::typography::selection();
    let font = |face: arcforge_ui_egui::typography::FaceSelection| {
        json!({
            "source_path": face.source_path.to_string_lossy(),
            "embedded": face.embedded,
            "font_name": face.font_name,
            "face_index": face.face_index,
            "weight": face.weight,
            "variation_coordinates": face.variation_coordinates,
        })
    };
    json!({
        "schema_version": 1,
        "scene": scene.name(),
        "capture_kind": "native-client-area",
        "native_window_chrome": false,
        "capture_api": "egui::Context::request_screenshot",
        "renderer": "eframe Glow",
        "rendered_frames_after_ready": 2,
        "os": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "process_id": std::process::id(),
        "unix_timestamp_seconds": SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|time| time.as_secs()),
        "pixels_per_point": context.pixels_per_point(),
        "theme": if context.global_style().visuals.dark_mode { "dark" } else { "light" },
        "viewport": viewport,
        "fonts": {"regular": font(fonts.regular), "semibold": font(fonts.semibold)},
        "snapshot_id": ready.snapshot_id,
        "metadata_revision": ready.metadata_revision,
        "selected_entry_id": ready.entry_id,
        "selected_entry_name": ready.entry_name,
        "preview_bytes": ready.preview_bytes,
        "preview_crc32_verified": true,
        "preview_image_size": ready.image_size,
    })
}

fn save_shot(directory: &Path, mut shot: Shot) -> Result<(), String> {
    let [width, height] = shot.image.size;
    let count = width
        .checked_mul(height)
        .ok_or("capture dimensions overflow")?;
    if width < 320 || height < 240 || count > MAX_PIXELS || shot.image.pixels.len() != count {
        return Err("capture has invalid or excessive pixel dimensions".into());
    }
    if shot
        .image
        .pixels
        .iter()
        .all(|pixel| *pixel == shot.image.pixels[0])
    {
        return Err("renderer returned a uniform image, not the prepared workspace".into());
    }
    // egui stores premultiplied colors; PNG requires straight alpha. This is an
    // encoding conversion, preserving the renderer's colors and transparency.
    let bytes: Vec<_> = shot
        .image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_srgba_unmultiplied)
        .collect();
    let path = directory.join(format!("{}.png", shot.scene.name()));
    write_png(&path, width as u32, height as u32, &bytes)?;
    shot.metadata["pixel_dimensions"] = json!([width, height]);
    let metadata_path = directory.join(format!("{}.json", shot.scene.name()));
    let mut metadata = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(metadata_path)
        .map_err(|error| format!("create capture metadata: {error}"))?;
    serde_json::to_writer_pretty(&mut metadata, &shot.metadata)
        .map_err(|error| format!("write capture metadata: {error}"))?;
    metadata
        .flush()
        .map_err(|error| format!("flush capture metadata: {error}"))?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(format!("{}.ready", shot.scene.name())))
        .map_err(|error| format!("publish capture readiness: {error}"))?;

    // The native window stays on this exact scene while xtask optionally captures
    // its OS chrome. Only the writer thread polls; the UI remains responsive.
    let acknowledgement = directory.join(format!("{}.continue", shot.scene.name()));
    let started = Instant::now();
    loop {
        if acknowledgement
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        if started.elapsed() > ACK_DEADLINE {
            return Err(
                "capture driver did not acknowledge the saved scene within 45 seconds".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn write_png(path: &Path, width: u32, height: u32, bytes: &[u8]) -> Result<(), String> {
    let file: File = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create native PNG: {error}"))?;
    let mut writer = BufWriter::new(file);
    image::codecs::png::PngEncoder::new(&mut writer)
        .write_image(bytes, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("encode native PNG: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush native PNG: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_uniform_or_excessive_renderer_images_before_writing() {
        let invalid = [
            egui::ColorImage::new([320, 240], vec![egui::Color32::BLACK; 320 * 240]),
            egui::ColorImage::new([1, 1], vec![egui::Color32::BLACK]),
        ];
        for image in invalid {
            let shot = Shot {
                scene: Scene::Text,
                image: Arc::new(image),
                metadata: json!({}),
            };
            assert!(save_shot(Path::new("unused-capture-directory"), shot).is_err());
        }
    }
}
