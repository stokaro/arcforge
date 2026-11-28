//! Maintainer-only driving of real workspace commands for native CI captures.
//!
//! This module never inserts a document, preview, or completed operation. The
//! ordinary archive worker opens the fixture and produces every displayed byte.

use super::{ArchiveWorkspace, EntryId, ViewMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scene {
    Text,
    Image,
    Empty,
    #[cfg(target_os = "macos")]
    OpenPanel,
}

impl Scene {
    pub const ALL: &'static [Self] = &[
        Self::Text,
        Self::Image,
        Self::Empty,
        #[cfg(target_os = "macos")]
        Self::OpenPanel,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "01-text-preview",
            Self::Image => "02-image-preview",
            Self::Empty => "03-empty-window",
            #[cfg(target_os = "macos")]
            Self::OpenPanel => "04-open-panel",
        }
    }

    pub fn entry_name(self) -> &'static str {
        match self {
            Self::Text => "Documents/Guide.txt",
            Self::Image => "Photos/Color-study.png",
            Self::Empty => "",
            #[cfg(target_os = "macos")]
            Self::OpenPanel => "",
        }
    }
}

/// Observed state after the worker has returned a complete CRC-checked preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadyScene {
    Preview(ReadyPreview),
    Empty,
    #[cfg(target_os = "macos")]
    OpenPanelRequested,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadyPreview {
    pub snapshot_id: String,
    pub metadata_revision: String,
    pub entry_id: EntryId,
    pub entry_name: String,
    pub preview_bytes: usize,
    pub image_size: Option<[usize; 2]>,
}

impl ArchiveWorkspace {
    /// Drive selection through the same commands used by the visible workspace.
    /// Call after `show`; `Some` means the requested state was actually drawn in
    /// that UI pass. The native harness separately waits for renderer callbacks.
    pub fn prepare_native_capture(
        &mut self,
        scene: Scene,
        context: &egui::Context,
    ) -> Result<Option<ReadyScene>, String> {
        if let Some(error) = self.filesystem_favorites_startup_error() {
            return Err(error.to_owned());
        }
        if !self.filesystem_favorites_ready() {
            return Ok(None);
        }
        if scene.entry_name().is_empty() {
            if self.document.is_some() || self.initial_path.is_some() {
                self.close();
                return Ok(None);
            }
            if self.current.is_some() {
                return Ok(None);
            }
            #[cfg(target_os = "macos")]
            if scene == Scene::OpenPanel {
                if !self.native_picker_pending() {
                    self.request_open_archive();
                    return Ok(None);
                }
                return Ok(Some(ReadyScene::OpenPanelRequested));
            }
            if self.selected.is_some() || self.preview.is_some() || self.native_picker_pending() {
                return Err("empty capture retained a selection, preview, or active picker".into());
            }
            return Ok(Some(ReadyScene::Empty));
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if self.source_changed {
            return Err("the fixture changed while preparing the capture".into());
        }
        if self.current.is_some() || self.initial_path.is_some() {
            return Ok(None);
        }
        let Some(document) = self.document.clone() else {
            return Ok(None);
        };
        let mut matches = document
            .snapshot
            .entries
            .iter()
            .filter(|entry| entry.name.display == scene.entry_name());
        let entry = matches
            .next()
            .ok_or_else(|| format!("capture fixture lacks {}", scene.entry_name()))?;
        if matches.next().is_some() {
            return Err(format!(
                "capture fixture has ambiguous {}",
                scene.entry_name()
            ));
        }
        if entry.is_directory {
            return Err("capture preview requires a file".into());
        }

        if self.view_mode != ViewMode::Outline || !self.query.is_empty() || self.folder.is_some() {
            return Err("capture requires the ordinary unfiltered Outline workspace".into());
        }
        let mut changed = false;
        for (offset, character) in scene.entry_name().char_indices() {
            if character == '/' {
                changed |= self
                    .expanded_folders
                    .insert(scene.entry_name()[..=offset].to_owned());
            }
        }
        if changed {
            self.filter(context);
            return Ok(None);
        }

        if self.selected != Some(entry.id) {
            let row = self
                .rows
                .iter()
                .position(|row| {
                    row.entry
                        .is_some_and(|index| document.snapshot.entries[index].id == entry.id)
                })
                .ok_or("capture entry is not visible in the prepared Outline rows")?;
            self.select_row(row, egui::Modifiers::NONE);
            self.preview_selected(context);
            return Ok(None);
        }
        let Some(preview) = &self.preview else {
            self.preview_selected(context);
            return Ok(None);
        };
        if preview.preview.entry_id != entry.id || !preview.preview.crc32_verified {
            return Err("capture preview does not match the CRC-checked selection".into());
        }
        let image_size = match scene {
            Scene::Text if preview.text_rows.is_some() && !self.preview_hex => None,
            Scene::Text => return Err("text capture did not produce a text preview".into()),
            Scene::Image => {
                let image = preview.image.as_ref().ok_or_else(|| {
                    preview
                        .image_error
                        .clone()
                        .unwrap_or_else(|| "image capture did not decode an image".into())
                })?;
                if !self.preview_show_image || self.preview_texture.is_none() {
                    return Ok(None);
                }
                Some(image.size)
            }
            Scene::Empty => unreachable!("empty scenes return before archive inspection"),
            #[cfg(target_os = "macos")]
            Scene::OpenPanel => {
                unreachable!("native panel scenes return before archive inspection")
            }
        };
        Ok(Some(ReadyScene::Preview(ReadyPreview {
            snapshot_id: document.snapshot.id.clone(),
            metadata_revision: document.snapshot.metadata_revision.clone(),
            entry_id: entry.id,
            entry_name: entry.name.display.clone(),
            preview_bytes: preview.preview.bytes.len(),
            image_size,
        })))
    }
}
